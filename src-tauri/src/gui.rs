//! GUI composition root(architecture.md 依賴規則 4:唯一知道具體型別 + tauri 的地方)。
//! 本檔是 Phase 5 新增的組裝點,類比 bin/pipeline.rs 但由 Tauri 事件迴圈驅動而非阻塞
//! run_forever。Task 7:最小開窗;Task 9:接 command + 管線 thread + emitter。
//!
//! token 三不(Global Constraints 1)落點,與 bin/pipeline.rs 同構:
//! - 不落檔:token 只經 CLACKS_BOT_TOKEN 環境變數(Keychain 注入),本檔不寫檔
//! - 不進 CLI:taster/cyrano 經 spawn/spawn_continue 的 env_clear + 白名單排除(adapters::pty)
//! - 不進 webview:TelegramHttp 只在 pipeline_thread 內部持有,絕不放進 emit payload
//!   或 command 回傳;emit 的只有 pane bytes / MessageOutcome debug / redacted 錯誤字串

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

/// PTY bytes → webview 事件(pane 專屬 topic)。實作 Write 以接進 pty output 工廠。
/// 只 emit,絕不寫控制終端(Global Constraints 2:kitty/TUI 逃逸序列不污染宿主終端,
/// 且 stdout 不是本流的去向——xterm.js 才是)
struct PaneEmitter {
    app: AppHandle,
    topic: &'static str,
}

impl Write for PaneEmitter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // lossy:pane 顯示用;xterm.js 消化 TUI 逃逸序列。token 結構性不在此流
        // (CLI 子行程 env 白名單排除 CLACKS_BOT_TOKEN——Global Constraints 1/3)
        let _ = self.app.emit(self.topic, String::from_utf8_lossy(buf).to_string());
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// 管線執行期狀態:停止旗標 + thread handle + 人工輸入 channel。
/// `.manage()` 需 Send + Sync——三個欄位皆滿足
#[derive(Default)]
struct PipelineState {
    running: Arc<AtomicBool>,
    handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// 人工輸入:(role, bytes) 送進管線 thread,thread 內經 write_raw_to 到對應 session
    input_tx: Mutex<Option<Sender<(String, Vec<u8>)>>>,
}

#[tauri::command]
fn start_pipeline(app: AppHandle, state: tauri::State<PipelineState>) -> Result<(), String> {
    if state.running.swap(true, Ordering::SeqCst) {
        return Err("管線已在執行".into());
    }
    let running = Arc::clone(&state.running);
    let (tx, rx) = std::sync::mpsc::channel::<(String, Vec<u8>)>();
    *state.input_tx.lock().unwrap() = Some(tx);
    let handle = std::thread::spawn(move || pipeline_thread(app, running, rx));
    *state.handle.lock().unwrap() = Some(handle);
    Ok(())
}

#[tauri::command]
fn stop_pipeline(state: tauri::State<PipelineState>) -> Result<(), String> {
    state.running.store(false, Ordering::SeqCst); // poll 迴圈下輪跳出
    *state.input_tx.lock().unwrap() = None; // 斷 channel,send_input 之後回「未執行」
    if let Some(h) = state.handle.lock().unwrap().take() {
        // join → thread 內 orchestrator/taster/cyrano drop → Drop→teardown(Global Constraints 6)
        let _ = h.join();
    }
    Ok(())
}

#[tauri::command]
fn send_input(
    role: String,
    data: String,
    state: tauri::State<PipelineState>,
) -> Result<(), String> {
    // 人工介入通道(findings:trust/login 對話框)。data 原樣送(含 \r 由前端決定)
    if let Some(tx) = state.input_tx.lock().unwrap().as_ref() {
        tx.send((role, data.into_bytes())).map_err(|e| e.to_string())
    } else {
        Err("管線未執行".into())
    }
}

fn pipeline_thread(app: AppHandle, running: Arc<AtomicBool>, input_rx: Receiver<(String, Vec<u8>)>) {
    use crate::adapters::pty::ClaudePtySession;
    use crate::adapters::store::SqliteStore;
    use crate::adapters::telegram::TelegramHttp;
    use crate::app::{Orchestrator, PipelineConfig};
    use crate::core::pipeline::{poll_backoff, AwaitTarget};
    use crate::ports::TelegramGateway;
    use std::time::Duration;

    // ---- 路徑組裝:與 bin/pipeline.rs 同構,失敗改 emit("fatal") + return(GUI thread 不能 exit) ----
    // runtime 相對路徑一律轉絕對:CLAUDE_CONFIG_DIR 交給 CLI 子行程,子行程 cwd = workdir
    // (≠ 本行程 cwd),相對路徑會被解析到錯處 → 誤判全新 config 跳 onboarding/login(真機 bug #1)
    let runtime = match std::fs::canonicalize("../clacks-runtime") {
        Ok(path) => path,
        Err(error) => {
            let _ = app.emit(
                "fatal",
                format!(
                    "../clacks-runtime 無法解析({error});先部署 runtime:\
                     cp -R templates/taster ../clacks-runtime/taster;\
                     cp -R templates/cyrano ../clacks-runtime/cyrano"
                ),
            );
            running.store(false, Ordering::SeqCst);
            return;
        }
    };
    let taster_dir = runtime.join("taster");
    let cyrano_dir = runtime.join("cyrano");

    // runtime 必須在 repo 外且已部署(嵌套在 repo 內會被祖先 CLAUDE.md 污染,骨架實證)
    for dir in [&taster_dir, &cyrano_dir] {
        if !dir.join("CLAUDE.md").exists() {
            let _ = app.emit(
                "fatal",
                format!(
                    "runtime 目錄缺失或未部署:{};先部署範本(repo 外):\
                     cp -R templates/taster ../clacks-runtime/taster;\
                     cp -R templates/cyrano ../clacks-runtime/cyrano",
                    dir.display()
                ),
            );
            running.store(false, Ordering::SeqCst);
            return;
        }
    }

    // CLAUDE_CONFIG_DIR 隔離(Task 7 裁決):taster/cyrano 共用一個 config,同帳號單次登入。
    // 全新 config dir 會跳 onboarding/login/trust——GUI 可經 pane 人工介入,但仍需先 pre-seed 一次
    let cli_config = runtime.join("cli-config");
    if !cli_config.join(".claude.json").exists() {
        let _ = app.emit(
            "fatal",
            format!(
                "CLI config 未 pre-seed:{cfg};先互動完成一次(兩 workdir 各跑一次以信任各自資料夾):\
                 ( cd {taster} && CLAUDE_CONFIG_DIR={cfg} claude );\
                 ( cd {cyrano} && CLAUDE_CONFIG_DIR={cfg} claude )",
                cfg = cli_config.display(),
                taster = taster_dir.display(),
                cyrano = cyrano_dir.display()
            ),
        );
        running.store(false, Ordering::SeqCst);
        return;
    }

    let gateway = TelegramHttp::from_env(); // token 封裝在此,絕不 emit(Global Constraints 1)

    // webhook 互斥檢查(骨架實證:掛著 webhook 時 getUpdates 必 409)
    match gateway.webhook_url() {
        Ok(None) => {}
        Ok(Some(url)) => {
            let _ = app.emit("fatal", format!("webhook active ({url}) — getUpdates 會 409,先解除 webhook"));
            running.store(false, Ordering::SeqCst);
            return;
        }
        Err(error) => {
            let _ = app.emit("fatal", format!("webhook 檢查失敗:{}", error.0));
            running.store(false, Ordering::SeqCst);
            return;
        }
    }

    // ---- output 工廠(Task 3):每(re)spawn 產生 emitter → 各自 pane topic(Global Constraints 2) ----
    let app_t = app.clone();
    let taster_factory = Box::new(move || {
        Box::new(PaneEmitter { app: app_t.clone(), topic: "pty://taster" }) as Box<dyn Write + Send>
    });
    let app_c = app.clone();
    let cyrano_factory = Box::new(move || {
        Box::new(PaneEmitter { app: app_c.clone(), topic: "pty://cyrano" }) as Box<dyn Write + Send>
    });

    // 既有隔離 spawn 路徑不繞過(Global Constraints 3):spawn / spawn_continue + Some(&cli_config)
    let mut taster = match ClaudePtySession::spawn(&taster_dir, Some(&cli_config), taster_factory) {
        Ok(s) => s,
        Err(e) => {
            let _ = app.emit("fatal", format!("spawn taster: {}", e.0));
            running.store(false, Ordering::SeqCst);
            return;
        }
    };
    // cyrano 重啟以 --continue 續談(真機驗證項見 Task 9)
    let mut cyrano = match ClaudePtySession::spawn_continue(&cyrano_dir, Some(&cli_config), cyrano_factory) {
        Ok(s) => s,
        Err(e) => {
            let _ = app.emit("fatal", format!("spawn cyrano: {}", e.0));
            running.store(false, Ordering::SeqCst);
            return;
        }
    };

    // 等雙 CLI 開機(與 bin/pipeline.rs 同構;注入前 wait_idle 另有把關,此為粗略 boot 緩衝)
    let _ = app.emit("state", "booting");
    std::thread::sleep(Duration::from_secs(15));

    // 去重落地在 repo 外(重啟不重收 backlog);與 runtime 同層
    let mut store = match SqliteStore::open(&runtime.join("clacks.db")) {
        Ok(s) => s,
        Err(e) => {
            let _ = app.emit("fatal", format!("open store: {}", e.0));
            running.store(false, Ordering::SeqCst);
            return;
        }
    };

    let mut orchestrator = Orchestrator::new(
        &gateway,
        &mut taster,
        &mut cyrano,
        &mut store,
        PipelineConfig::default(),
        Box::new(|delay| std::thread::sleep(delay)),
    );

    // 自有 poll 迴圈:取代 run_forever(-> !),讓 stop 旗標能跳出 → 乾淨 teardown(GC 6/8)。
    // 邏輯 = run_forever 的可測部分(poll_once + poll_backoff),不改 orchestrator。
    // offset 0:重啟後重拉未確認 backlog,SqliteStore 去重擋掉已處理者
    let _ = app.emit("state", "running");
    let mut offset = 0i64;
    let mut consecutive = 0u32;
    while running.load(Ordering::SeqCst) {
        // 先排空人工輸入(trust/login 對話框介入)。orchestrator 持有 &mut taster/cyrano 的
        // 整個生命週期,故經 write_raw_to 純轉發到對應 session(GC 8:非決策邏輯)
        while let Ok((role, bytes)) = input_rx.try_recv() {
            let target = match role.as_str() {
                "taster" => Some(AwaitTarget::Taster),
                "cyrano" => Some(AwaitTarget::Cyrano),
                _ => None,
            };
            if let Some(target) = target {
                if let Err(e) = orchestrator.write_raw_to(target, &bytes) {
                    let _ = app.emit("poll-error", format!("write_raw {role}: {}", e.0));
                }
            } else {
                let _ = app.emit("poll-error", format!("未知輸入目標:{role}"));
            }
        }
        match orchestrator.poll_once(offset) {
            Ok((next, outcomes)) => {
                consecutive = 0;
                offset = next;
                for o in &outcomes {
                    let _ = app.emit("outcome", format!("{o:?}")); // MessageOutcome debug(無 token)
                }
            }
            Err(e) => {
                consecutive += 1;
                let _ = app.emit("poll-error", e.0); // GatewayError 已 redact(GC 1)
                std::thread::sleep(poll_backoff(consecutive));
            }
        }
    }
    // 迴圈跳出 → orchestrator drop → taster/cyrano drop → Drop→teardown(GC 6)
    let _ = app.emit("state", "stopped");
}

pub fn run() {
    tauri::Builder::default()
        .manage(PipelineState::default())
        .invoke_handler(tauri::generate_handler![start_pipeline, stop_pipeline, send_input])
        .run(tauri::generate_context!())
        .expect("啟動 Tauri 應用失敗");
}
