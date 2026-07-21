//! **狀態(Phase 5 起):dev-only**。受支援部署路徑為 GUI(見 src/gui.rs)。
//! 本 bin 無信號處理器且 PTY bytes 走 stdout(kitty 序列會污染控制終端、
//! Ctrl-C 失效——findings「設計輸入 B」);停止請於另一終端
//!   pkill -f 'target/debug/pipeline'   # 再以 pgrep -P <pid> 清孤兒 claude
//! 乾淨的信號式關閉需改 run_forever 為可跳出迴圈(重寫 orchestrator),
//! 本 phase 不做——GUI 以自己的 poll 迴圈 + stop 旗標達成乾淨 teardown
//!
//! Phase 4 composition root:真實雙 CLI 管線(taster + cyrano)。
//! 唯一的組裝點(architecture.md 依賴規則 4):建 adapter、注入 orchestrator、run_forever。
//! GUI/Tauri 留 Phase 5;本 bin 是無頭常駐版。
//!
//! 從 repo root 執行(../clacks-runtime 相對路徑才對):
//!   CLACKS_BOT_TOKEN=$(security find-generic-password -s clacks-bot -w) \
//!     cargo run --manifest-path src-tauri/Cargo.toml --bin pipeline
//!
//! token 三不(Global Constraints 1)落點:
//! - 不落檔:token 只經 CLACKS_BOT_TOKEN 環境變數(Keychain 注入),本 bin 不寫檔
//! - 不進 CLI:taster/cyrano 經 spawn/spawn_continue 的 env_clear + 白名單
//!   (CLACKS_BOT_TOKEN 不在白名單)結構性排除——見 adapters::pty
//! - 不進錯誤字串:TelegramHttp 的 redact(without_url)+ error_for_status(Task 1)

use clacks::adapters::pty::ClaudePtySession;
use clacks::adapters::store::SqliteStore;
use clacks::adapters::telegram::TelegramHttp;
use clacks::app::{Orchestrator, PipelineConfig};
use clacks::ports::TelegramGateway;
use std::time::Duration;

fn main() {
    // runtime 相對路徑一律轉絕對:CLAUDE_CONFIG_DIR 交給 CLI 子行程,而子行程的
    // cwd = workdir(≠ pipeline 的 cwd),相對路徑會被子行程對自己的 cwd 解析到
    // 錯的地方 → CLI 找不到 pre-seed 的 config → 誤判全新 config 跳 onboarding/login
    // (Task 9 真機實證)。canonicalize 需目錄存在(已部署),故先解析父目錄再 join
    let runtime = match std::fs::canonicalize("../clacks-runtime") {
        Ok(path) => path,
        Err(error) => {
            eprintln!(
                "[clacks] ../clacks-runtime 無法解析({error});先部署 runtime:\n  \
                 cp -R templates/taster ../clacks-runtime/taster\n  \
                 cp -R templates/cyrano ../clacks-runtime/cyrano"
            );
            std::process::exit(1);
        }
    };
    let taster_dir = runtime.join("taster");
    let cyrano_dir = runtime.join("cyrano");

    // runtime 必須在 repo 外且已部署(嵌套在 repo 內會被祖先 CLAUDE.md 污染,骨架實證)
    for dir in [&taster_dir, &cyrano_dir] {
        if !dir.join("CLAUDE.md").exists() {
            eprintln!(
                "[clacks] runtime 目錄缺失或未部署:{}\n先部署範本(repo 外):\n  \
                 cp -R templates/taster ../clacks-runtime/taster\n  \
                 cp -R templates/cyrano ../clacks-runtime/cyrano",
                dir.display()
            );
            std::process::exit(1);
        }
    }

    // Task 7 裁決:CLAUDE_CONFIG_DIR 隔離(taster/cyrano 共用一個 config,
    // 同帳號單次登入避開 token family 輪替風險)。全新 config dir 會跳
    // onboarding/login/trust 對話框——headless 無法通過,必須先互動 pre-seed
    let cli_config = runtime.join("cli-config");
    if !cli_config.join(".claude.json").exists() {
        eprintln!(
            "[clacks] CLI config 未 pre-seed:{cfg}\n\
             headless spawn 無法通過 onboarding/login/trust 對話框。先互動完成一次\
             (共用同一 config 只需登入一次,兩個 workdir 各跑一次以信任各自資料夾):\n  \
             ( cd {taster} && CLAUDE_CONFIG_DIR={cfg} claude )   # theme/login/trust 後 /exit\n  \
             ( cd {cyrano} && CLAUDE_CONFIG_DIR={cfg} claude )   # 只需 trust 後 /exit",
            cfg = cli_config.display(),
            taster = taster_dir.display(),
            cyrano = cyrano_dir.display()
        );
        std::process::exit(1);
    }

    let gateway = TelegramHttp::from_env();

    // webhook 互斥檢查(骨架實證:掛著 webhook 時 getUpdates 必 409)
    match gateway.webhook_url() {
        Ok(None) => {}
        Ok(Some(url)) => {
            eprintln!("[clacks] webhook active ({url}) — getUpdates 會 409,先解除 webhook");
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("[clacks] webhook 檢查失敗:{}", error.0);
            std::process::exit(1);
        }
    }

    let mut taster = ClaudePtySession::spawn(&taster_dir, Some(&cli_config), Box::new(|| Box::new(std::io::stdout())))
        .expect("spawn taster");
    // cyrano 重啟以 --continue 續談(真機驗證項見 Task 9)
    let mut cyrano = ClaudePtySession::spawn_continue(&cyrano_dir, Some(&cli_config), Box::new(|| Box::new(std::io::stdout())))
        .expect("spawn cyrano");

    eprintln!("[clacks] 等待雙 CLI 開機 15s");
    std::thread::sleep(Duration::from_secs(15));

    // 去重落地在 repo 外(重啟不重收 backlog);與 runtime 同層
    let mut store = SqliteStore::open(&runtime.join("clacks.db")).expect("open store");

    let mut orchestrator = Orchestrator::new(
        &gateway,
        &mut taster,
        &mut cyrano,
        &mut store,
        PipelineConfig::default(),
        Box::new(|delay| std::thread::sleep(delay)),
    );

    // offset 0:重啟後重拉未確認 backlog,SqliteStore 去重擋掉已處理者(設計文件錯誤處理)
    orchestrator.run_forever(0);
}
