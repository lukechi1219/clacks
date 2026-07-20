# Phase 5:GUI(Tauri 前端)+ 注入可靠性(idle 偵測)+ 生命週期收尾 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Phase 4 已在真機端到端跑通的無頭雙 CLI 管線接上 **Tauri GUI**(兩塊 xterm.js pane + 狀態顯示 + 人工介入輸入),同時修掉真機 smoke 抓到的三個可靠性缺口:**注入前 idle 偵測**(設計輸入 A:連發訊息 bracketed-paste 被吞、內容靜默掉字)、**respawn 後 settle**(設計輸入 C:恢復後立刻注入撞 EIO)、**headless 信號/終端污染**(設計輸入 B:Ctrl-C 失效、SIGTERM 不跑 teardown → 孤兒 claude)。A 與 C 以**同一個 idle 機制**一次解決;B 以「GUI 接管 PTY 生命週期」使該缺口對受支援部署路徑失效,headless bin 降級為 dev-only。附帶清掉 Phase 3 起無消費端的 `Clock` port(裁決見 Task 5)。

**Architecture:** 沿用 architecture.md 依賴規則。core 純函式與 orchestrator 的**決策邏輯不重寫**——GUI 只新增一個 composition root(Tauri command 模組,類比 `bin/pipeline.rs`,但由 Tauri 事件迴圈驅動而非阻塞 `run_forever`)與一個把 PTY bytes/MessageOutcome 推給 webview 的 adapter 級 emitter。idle 偵測沿 respawn 先例:**機制**(PTY 輸出靜默追蹤)落 adapter(`pty.rs`),**門檻政策**(靜默多久算就緒)落 core 常數(`core::session`),**編排**(注入前先等 idle)落 orchestrator(`app.rs`)——不新增第 6 個 port,只在既有 `CliSession` 上加方法(與 Phase 4 `respawn` 同型)。前端**保持扁平不分層**(architecture.md「刻意不採用/前端分層」:UI 只 render PTY 輸出 + 狀態,無自己的業務邏輯)。

**Tech Stack:** Rust(edition 2024);既有 dep 全部沿用(notify 8.2.0、portable-pty 0.9.0、reqwest 0.13.4、rusqlite 0.40.1、serde 1.0.228、serde_json 1.0.150、tempfile 3.27.0)。**新增 Tauri v2**(`tauri` / `tauri-build`,授權 MIT/Apache-2.0——與 repo 雙授權相容)與前端 **`@xterm/xterm`**(MIT)、`@tauri-apps/api`、`@tauri-apps/cli`、Vite。**版本敏感(見 Global Constraints 9)**:Tauri 2.x / xterm 5.x 的 API 與 config schema 會隨版本漂移;plan 給的 `cargo add tauri@2`、`npm install @xterm/xterm@5.5.0` 等指令**以撰寫時版本為準,resolved 版本必須寫進報告,任何字面偏差(feature 名、config 鍵、API)必須揭露**。不引入 `ctrlc` 或其他信號庫(理由見設計輸入 B 裁決)。不引入 tokio/async_trait(同步阻塞不變)。

## Global Constraints

每項約束都落到會觸犯它的那個任務(標註處),不只寫在這裡:

1. **bot token 絕不進前端/webview**(新增的最高安全紅線):token 只存在 `adapters::telegram::TelegramHttp`(Keychain → `CLACKS_BOT_TOKEN`,Phase 4 已封裝)。GUI 的 Tauri command 表面**只暴露 (pane bytes, MessageOutcome, 人工輸入 bytes)**,不得回傳/emit token、不得把 `TelegramHttp` 或 raw token 交給任何 `#[tauri::command]` 的回傳值或 `emit` payload。
   - 落點 Task 9:command 模組不 import token、gateway 只在管線 thread 內部持有;附 `rg` 驗證 command 簽名與 emit payload 不含 token/gateway 型別。
2. **PTY 輸出只進 webview,不進控制終端**(設計輸入 B 的正面不變式):GUI 版 PTY bytes 一律經 emitter 導向 xterm.js pane,**不寫 stdout/控制 tty**——這正是 headless 版 kitty-keyboard 序列污染終端、打爆 Ctrl-C 的根因消除。
   - 落點 Task 3(output factory 讓 respawn 也持續串流,不再 `sink()`)+ Task 9(GUI 的 output factory 產生 emitter 而非 stdout)。
3. **taster/cyrano 隔離必須透過 GUI spawn 路徑保持**(Phase 4 安全模型不得被 GUI 繞過):GUI composition root 走既有 `ClaudePtySession::spawn`/`spawn_continue`(`env_clear` + `ENV_ALLOWLIST` + `CLAUDE_CONFIG_DIR` 隔離、workdir 在 repo 外),**不新開一條繞過結構性排除的 spawn**。
   - 落點 Task 9:GUI 用既有 spawn API(`Some(cli_config)`、repo 外 workdir);附 `minimal_env_excludes_secrets_and_unknowns` 迴歸未動的檢查。
4. **core / orchestrator 依賴規則**(architecture.md):core 只 std + serde;`app.rs` 只 import core + ports;**Tauri/webview 只准在 composition root(GUI command 模組)出現**。
   - 落點 Task 1(core 加常數不引 IO)、Task 4(app.rs 加 wait_idle 編排、不 import adapter/tauri)、Task 9(gui 模組是唯一 import tauri 的地方);各附 `^use` 行 rg(掃 use 陳述非註解——Phase 3/4 教訓:命中 doc comment 是假陽性,`src-tauri` 路徑含 `tauri` 子字串會讓管線式 rg 假陽性)。
5. **政策集中 core/orchestrator,adapter 保持愚蠢**:idle 門檻值(靜默多久 = 就緒)是 core 常數;「注入前等 idle」的編排在 orchestrator;PTY 靜默追蹤的機制在 adapter。
   - 落點 Task 1(core 常數)+ Task 2(pty 機制)+ Task 4(orchestrator 編排)。
6. **teardown 必經 + 乾淨關閉**(承 Phase 3/4 裁決):GUI 停止/視窗關閉時 `CliSession` 必須被 drop → 走既有 `Drop`→`teardown`(kill + 有界等待 + SIGKILL),不得留孤兒 claude。GUI 的管線 thread 以停止旗標在 `poll_once` 之間跳出後 join,讓 session drop——這是 headless `run_forever`(`-> !` 永不返回)做不到的乾淨關閉。
   - 落點 Task 9(stop 路徑 drop session)+ Task 12(真機 pgrep 無孤兒)。
7. **idle 門檻只能真機校正**:`IDLE_QUIET`(靜默視窗)先取保守起始值;實際「TUI 回到可接受 bracketed-paste」的量級只能真機量測(fake 只能驗機制:wait_idle 有被呼叫、靜默後返回)。
   - 落點 Task 1 起始值 + Task 12 真機校正回填。
8. **不重寫 orchestrator 決策邏輯**:GUI 只新增 composition root + emitter;`process_update`/`judge_*`/`recover`/`run_forever` 的既有邏輯不動。GUI 用既有 `poll_once`(公開、已測)在自己的 thread 迴圈驅動並 emit outcome,`poll_backoff` 沿用——不改 `run_forever`。
   - 落點 Task 9。
9. **依賴 pin + 版本敏感揭露**(repo 規劃守則):新增 cargo/npm 依賴以 `cargo add tauri@2` / `npm install @xterm/xterm@5.5.0` 形式安裝,**resolved 版本寫進報告**;Tauri 2.x config schema、xterm 5.x import 路徑、`@tauri-apps/api` 的 `event`/`core` 匯出以撰寫時版本為準,任何字面偏差(含 feature 名、config 鍵)必須揭露。新增依賴前確認授權相容(Tauri MIT/Apache、xterm MIT——皆與 repo 相容,Task 7 檢查)。
10. **git 紀律**(repo CLAUDE.md):小 commit、`git add` 與 `git commit` 分開呼叫、不 chain `cd`(用 `--manifest-path` / `git -C`)、結構性與行為性變更分開 commit。

## 刻意不做(避免撈過界)

- **不重規劃 HOME 重定位完全隔離的完整調查**:findings「Phase 4 ~/.claude 隔離調查裁決」第 3 點已把 HOME 重定位 + OAuth 交互列為**未驗證、不得事前斷言**的 Phase 5 真機驗證項。GUI **不 block 在此**(既有 `CLAUDE_CONFIG_DIR` 部分隔離已足夠運作)。僅以 Task 11 一個**有界的 (human) 驗證任務**收尾(類比 Phase 4 Task 7/9),不寫成編碼任務。
- **不佈線 sandbox-exec 進 GUI spawn**:Phase 4 Task 8 已部署 `templates/sandbox/clacks.sb` 正本但未佈線。經 portable-pty spawn 的 sandbox 包裝與 PTY 生命週期/重啟的交互仍未端到端驗證,且會與本 phase 的 idle/respawn 改動疊加風險——留待 GUI 穩定後獨立一輪。GUI 用既有(未 sandbox)spawn 路徑。
- **不做私訊白名單 / 群組模式 / 非文字制式回覆 / `/compact` 佈線 / 多 bot**:與 Phase 4「刻意不做」一致,均非 GUI 或本三缺口所需。
- **不改 taster/cyrano 角色 template 與契約**:core 契約、envelope、hook 均凍結;GUI 只顯示既有 outcome 詞彙表。
- **前端不分層、不引框架**:純 HTML + TS + xterm.js + `@tauri-apps/api`;無 React/狀態庫(architecture.md:前端出現業務邏輯才分層,本 phase 不會)。

## 檔案結構(本 phase 完成後)

```
src-tauri/
├── Cargo.toml                # Task 7:加 tauri/tauri-build 依賴 + [lib]/[[bin]] 調整
├── build.rs                  # Task 7:tauri_build::build()
├── tauri.conf.json           # Task 7:Tauri app 設定(視窗、frontendDist、identifier)
├── src/
│   ├── core/session.rs       # Task 1:IDLE_QUIET / IDLE_SETTLE_TIMEOUT 常數
│   ├── ports.rs              # Task 1:CliSession 加 wait_idle;Task 5:刪 Clock
│   ├── app.rs                # Task 4:exec 注入前呼叫 wait_idle
│   ├── adapters/
│   │   ├── pty.rs            # Task 2:輸出靜默追蹤 + wait_idle;Task 3:output factory
│   │   ├── clock.rs          # Task 5:刪除(dead code)
│   │   └── mod.rs            # Task 5:移除 clock 模組宣告
│   ├── gui.rs                # Task 9:Tauri command 模組(composition root + emitter)
│   ├── lib.rs                # Task 7/9:pub mod gui;
│   └── bin/
│       ├── clacks-gui.rs     # Task 7:Tauri app 入口 bin(呼叫 clacks::gui::run())
│       └── pipeline.rs       # Task 6:doc comment 降級為 dev-only(邏輯不動)
├── tests/
│   ├── support/mod.rs        # Task 1:ScriptedCli 加 wait_idle;Task 5:刪 ManualClock
│   ├── orchestrator.rs       # Task 4:注入前 wait_idle 測試
│   └── fakes_selftest.rs     # Task 5:刪 Clock selftest
src/                          # 前端(repo root,architecture.md 指定;不分層)
├── index.html               # Task 8:兩 pane + 狀態列 + 輸入框骨架
├── main.ts                  # Task 8 骨架;Task 10 接線(listen 事件、invoke command)
├── styles.css               # Task 8
package.json                 # Task 8:@xterm/xterm、@tauri-apps/{api,cli}、vite
docs/superpowers/notes/
└── 2026-07-17-skeleton-findings.md   # Task 6/11/12:decision + 真機記錄 append
```

所有 cargo 指令從 repo 根執行,一律帶 `--manifest-path src-tauri/Cargo.toml`。前端 npm 指令從 repo 根(`package.json` 所在)執行。

TDD 步驟慣例:RED 骨架的過渡 unused 警告不需處理;GREEN 與完工檢核要求零 warning。(human)標記的真機/真裝置任務寫成可勾稽 checklist——這些需 Telegram / 真 claude CLI / GUI 視窗,**本 agent 無法執行**,必須由人操作並回填。

---

### Task 1: core idle 門檻常數 + CliSession::wait_idle 方法(port 擴充,不新增 port)

**Files:**
- Modify: `src-tauri/src/core/session.rs`(加兩個常數)
- Modify: `src-tauri/src/ports.rs`(CliSession 加 `wait_idle` 方法簽名)
- Modify: `src-tauri/tests/support/mod.rs`(ScriptedCli 實作 wait_idle)

**背景(必讀)**:findings「Phase 5 設計輸入 A/C」——Rust 等的是 Stop hook 產物(=生成結束),但「生成結束」≠「TUI 回到可接受 bracketed-paste」;連發訊息(A)與 respawn 之後(C)都會在這個空窗注入 → paste 信封被 TUI 整段吞掉、殘留 `\r` 送空 prompt → 掉字/EmptyReply。正解是**注入前 idle 偵測**(確認 CLI 真回到可輸入),而非再加死 sleep。本任務先立 port 介面與 core 門檻,機制/編排在 Task 2/4。

**約束落點:**
- **政策集中 core(Global Constraints 5)**:門檻值是 core 常數(與 `CONTROL_BUFFER`/`ARTIFACT_TIMEOUT` 並列),不落 adapter。
- **門檻只能真機校正(Global Constraints 7)**:`IDLE_QUIET` 是保守起始值,doc 註明真機量測項(Task 12 回填)。
- **不新增第 6 個 port**:在既有 `CliSession` 上加方法(Phase 4 `respawn` 先例)。
- **core 零 IO(Global Constraints 4)**:只加 `Duration` 常數。

**Interfaces:**
- Produces:`session::IDLE_QUIET`、`session::IDLE_SETTLE_TIMEOUT`;`CliSession::wait_idle(&mut self, quiet_for: Duration, timeout: Duration) -> Result<(), WaitError>`;ScriptedCli 記錄呼叫。

- [ ] **Step 1: core::session 加常數**

在 `src-tauri/src/core/session.rs`(`ARTIFACT_TIMEOUT` 之後)加入:

```rust
/// 注入前 idle 偵測的靜默視窗:PTY 輸出連續靜默達此長度 = TUI 回到可接受
/// bracketed-paste 的就緒態(findings「Phase 5 設計輸入 A/C」:產物≠可輸入)。
///
/// 真機校正項(Task 12):此為保守起始值。太短 → 仍在收尾/thinking 停頓被
/// 誤判就緒 → 掉字重演;太長 → 每則注入平白延遲。實際「收尾空窗」量級只能
/// 真機量測(單發乾淨性 + 連發不吞字),回填此值並在報告揭露偏差。
pub const IDLE_QUIET: Duration = Duration::from_millis(750);

/// 等待就緒的上限:含開機/respawn 後 CLI 首次靜默(取代 spawn 後的死 sleep 15s)。
/// 逾時 = 未能在期限內觀察到靜默(可能卡在 login/trust 對話框)——orchestrator
/// 以 best-effort 續注入(GUI 版使用者可經 pane 人工介入),不視為 session 失敗
pub const IDLE_SETTLE_TIMEOUT: Duration = Duration::from_secs(30);
```

補對應單元測試(門檻關係的防呆,純函式層可測的部分):

```rust
    #[test]
    fn idle_quiet_is_shorter_than_settle_timeout() {
        // 靜默視窗必須遠小於就緒上限,否則永遠等不到一個完整靜默窗
        assert!(IDLE_QUIET < IDLE_SETTLE_TIMEOUT);
    }
```

- [ ] **Step 2: ports.rs 的 CliSession 加 wait_idle**

在 `pub trait CliSession` 的 `respawn` 之後加入(方法簽名 + doc):

```rust
    /// 阻塞至 PTY 輸出連續靜默達 `quiet_for`(= TUI 回到可接受 bracketed-paste),
    /// 或達 `timeout`。注入訊息前呼叫,消除「產物已到但 TUI 未就緒」的注入空窗
    /// (findings「Phase 5 設計輸入 A/C」:連發掉字、respawn 後 EIO)。
    ///
    /// - Ok(()):已觀察到就緒靜默,可安全注入
    /// - Err(WaitError::Timeout):期限內未靜默(可能卡互動對話框)——呼叫端
    ///   best-effort 續注入,不得視為 session 失敗(GUI 使用者可經 pane 介入)
    ///
    /// 控制指令(/clear)後的緩衝仍由 orchestrator 的 CONTROL_BUFFER 負責,兩者互補
    fn wait_idle(&mut self, quiet_for: Duration, timeout: Duration) -> Result<(), WaitError>;
```

**注意**:加了 trait 方法後所有 `impl CliSession`(`ClaudePtySession`、`ScriptedCli`)未實作即編譯失敗——Step 3 補 fake,Task 2 補真 adapter。此步先不編譯全套件。

- [ ] **Step 3: support/mod.rs 的 ScriptedCli 實作 wait_idle**

`ScriptedCli` struct 加欄位(在 `respawns` 旁):

```rust
    /// wait_idle 呼叫次數(注入前 idle 編排測試斷言用)
    pub idle_waits: u32,
    /// >0 時下 N 次 wait_idle 回 Timeout(測 best-effort 續注入路徑);耗盡後正常
    pub idle_timeouts: u32,
```

`ScriptedCli::new` 初始化補 `idle_waits: 0, idle_timeouts: 0`。在 `impl CliSession for ScriptedCli` 內加(緊接 `respawn` 之後):

```rust
    fn wait_idle(&mut self, _quiet_for: Duration, _timeout: Duration) -> Result<(), WaitError> {
        self.idle_waits += 1;
        if self.idle_timeouts > 0 {
            self.idle_timeouts -= 1;
            return Err(WaitError::Timeout);
        }
        Ok(())
    }
```

- [ ] **Step 4: 編譯 + fake selftest GREEN**

Run: `cargo test --manifest-path src-tauri/Cargo.toml core::session`
Expected: 含 `idle_quiet_is_shorter_than_settle_timeout` 全綠

Run: `cargo build --manifest-path src-tauri/Cargo.toml --tests`
Expected: `ClaudePtySession` 因尚未實作 `wait_idle` **編譯失敗**(預期的 RED,Task 2 補齊);`--tests` 中 support/ 的 fake 已可編。(此步確認 fake 側就位,adapter 側待 Task 2)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/core/session.rs src-tauri/src/ports.rs src-tauri/tests/support/mod.rs
git commit -m "feat: idle 門檻常數 + CliSession::wait_idle port 方法(注入前就緒偵測介面,fake 就位)"
```

---

### Task 2: pty.rs 實作 wait_idle — PTY 輸出靜默追蹤(設計輸入 A/C 機制面)

**Files:**
- Modify: `src-tauri/src/adapters/pty.rs`(reader thread 記錄最後輸出時刻 + wait_idle)

**背景(必讀)**:`ClaudePtySession` 的 reader thread 目前把 PTY bytes 寫進 `output` 後即丟。本任務讓 reader thread**額外**更新一個共享「最後輸出時刻」,`wait_idle` 輪詢該時刻直到靜默達門檻。機制在 adapter(Global Constraints 5),門檻由呼叫端(orchestrator,Task 4)帶入 core 常數。

**約束落點:**
- **機制在 adapter(Global Constraints 5)**:靜默追蹤純屬 PTY 觀察,不含門檻政策(門檻是參數)。
- **安全紅線(Phase 4 承接)**:不得改動 `env_clear` + `ENV_ALLOWLIST` 及 `minimal_env_excludes_secrets_and_unknowns`;不動 teardown。
- **單調時鐘**:用 `std::time::Instant`(單調),**不**用 `Clock` port(`SystemTime` 牆鐘,型別本就不對——佐證 Clock 對本 phase 無用,見 Task 5)。

**Interfaces:**
- Consumes:Task 1 的 `wait_idle` 簽名。
- Produces:`ClaudePtySession` 新增欄位 `last_output: Arc<Mutex<Instant>>`(reader thread 更新);`impl CliSession::wait_idle`。

- [ ] **Step 1: struct 加共享時刻欄位 + reader thread 更新**

`use` 補:`use std::sync::{Arc, Mutex};`(`Instant` 已 import)。

`ClaudePtySession` struct 加欄位(在 `config_dir` 之後):

```rust
    /// PTY 最後一次有輸出的時刻(reader thread 更新)。wait_idle 據此判就緒靜默。
    /// 單調時鐘 Instant——非 Clock port 的 SystemTime(idle 是牆鐘無關的量)
    last_output: Arc<Mutex<Instant>>,
```

在 `spawn_program` 建 reader thread 前建立共享時刻,並在 thread 內每次讀到 bytes 後更新;struct 建構補欄位:

```rust
        let last_output = Arc::new(Mutex::new(Instant::now()));
        let reader_clock = Arc::clone(&last_output);
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        *reader_clock.lock().unwrap() = Instant::now();
                        let _ = output.write_all(&buf[..n]);
                        let _ = output.flush();
                    }
                }
            }
        });
```

`Ok(Self { ... })` 補 `last_output,`(在 `config_dir: ...` 之後)。

- [ ] **Step 2: 實作 wait_idle(緊接 respawn 之後)**

```rust
    fn wait_idle(&mut self, quiet_for: Duration, timeout: Duration) -> Result<(), WaitError> {
        // 輪詢最後輸出時刻:距今 >= quiet_for 即視為就緒。輪詢間隔取 quiet_for 的
        // 一小段(下限 20ms)以免忙等。deadline 內未達靜默 → Timeout(呼叫端 best-effort)
        let deadline = Instant::now() + timeout;
        let poll = (quiet_for / 8).max(Duration::from_millis(20));
        loop {
            let quiet_since = self.last_output.lock().unwrap().elapsed();
            if quiet_since >= quiet_for {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(WaitError::Timeout);
            }
            std::thread::sleep(poll);
        }
    }
```

同步更新既有直呼 `spawn_program` 的測試不需改(struct 建構在函式內部,測試呼叫簽名不變)。

- [ ] **Step 3: 加 adapter 級 wait_idle 測試(真實子行程,tempdir)**

在 `mod tests` 追加(用 `sh` 先 burst 輸出、再靜默,驗證 wait_idle 在靜默後才返回):

```rust
    // 設計輸入 A/C 機制:輸出停止後 wait_idle 才返回 Ok;有輸出時不會提早就緒
    #[test]
    fn wait_idle_returns_after_output_goes_quiet() {
        let dir = tempfile::tempdir().unwrap();
        // 立刻噴一行、然後靜默(sleep 遠長於 quiet_for)
        let mut session = ClaudePtySession::spawn_program(
            &["sh", "-c", "printf 'boot\\n'; sleep 5"],
            vec!["sh".to_string()],
            dir.path(),
            None,
            Box::new(std::io::sink()),
        )
        .unwrap();
        let quiet_for = Duration::from_millis(300);
        let start = Instant::now();
        let r = session.wait_idle(quiet_for, Duration::from_secs(3));
        let waited = start.elapsed();
        drop(session);
        assert!(r.is_ok(), "靜默後必須就緒");
        // 必然等到至少一個 quiet_for 窗(不會在還有輸出時就返回)
        assert!(waited >= quiet_for, "不得在靜默窗達成前就返回:{waited:?}");
    }

    // 期限內從不靜默(持續輸出)→ Timeout,呼叫端 best-effort
    #[test]
    fn wait_idle_times_out_when_never_quiet() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = ClaudePtySession::spawn_program(
            &["sh", "-c", "while :; do printf x; sleep 0.05; done"],
            vec!["sh".to_string()],
            dir.path(),
            None,
            Box::new(std::io::sink()),
        )
        .unwrap();
        let r = session.wait_idle(Duration::from_millis(500), Duration::from_millis(800));
        drop(session);
        assert_eq!(r, Err(WaitError::Timeout));
    }
```

- [ ] **Step 4: GREEN + 紅線**

Run: `cargo test --manifest-path src-tauri/Cargo.toml adapters::pty`
Expected: 全綠(含兩個新測試),無 warning。`minimal_env_excludes_secrets_and_unknowns`、三個 teardown/respawn 測試原樣通過

Run: `rg -n 'env_clear|ENV_ALLOWLIST' src-tauri/src/adapters/pty.rs`
Expected: 仍存在——token 結構性排除紅線未動

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/adapters/pty.rs
git commit -m "feat: pty wait_idle——PTY 輸出靜默追蹤(設計輸入 A/C 機制,單調 Instant 非 Clock)"
```

---

### Task 3: pty.rs output factory — respawn 後持續串流(消除 sink() 缺口)

**Files:**
- Modify: `src-tauri/src/adapters/pty.rs`(output 由一次性 Box 改為工廠)
- Modify: `src-tauri/src/bin/pipeline.rs`(呼叫端改傳工廠)
- Modify: `src-tauri/src/bin/smoke.rs`(呼叫端改傳工廠)

**背景(必讀)**:現況 `respawn()` 把新 session 的輸出寫死成 `Box::new(std::io::sink())`——headless 期無妨,但 **GUI 期會讓 respawn 後的 pane 靜止**(recover 後 CLI 的 TUI 輸出不再進 xterm.js)。根因:`output` 是一次性 `Box<dyn Write + Send>`,被 move 進 reader thread,respawn 無從重建。本任務把 `output` 參數改為**工廠** `Box<dyn FnMut() -> Box<dyn Write + Send> + Send>`,存進 struct,spawn 與 respawn 都呼叫它取一個新 writer——GUI 的工廠回傳 emitter(Task 9),headless/smoke 的工廠回傳 stdout/sink。

**約束落點:**
- **PTY 輸出只進 webview(Global Constraints 2)**:此工廠是「respawn 後仍串流到 pane」的機制基礎;GUI 工廠(Task 9)產生 emitter。
- **不新增 dep**:純 std 閉包。
- **teardown 必經**:僅換 output 生成方式,不動 teardown/respawn 的 kill 順序。

**Interfaces:**
- Produces:`spawn`/`spawn_continue`/`spawn_program` 的 `output: Box<dyn Write + Send>` 改為 `make_output: Box<dyn FnMut() -> Box<dyn Write + Send> + Send>`;`ClaudePtySession` 存 `make_output` 供 respawn 重用。

- [ ] **Step 1: 簽名改工廠 + struct 存工廠 + respawn 用工廠**

`spawn_program` 簽名末參數改:

```rust
    fn spawn_program(
        argv: &[&str],
        respawn_argv: Vec<String>,
        workdir: &Path,
        config_dir: Option<&Path>,
        mut make_output: Box<dyn FnMut() -> Box<dyn Write + Send> + Send>,
    ) -> Result<Self, CliError> {
        // …openpty / cmd / env 不變…
        let mut output = make_output();   // 本次 session 的 writer
        // …reader thread 用 output(含 Task 2 的 last_output 更新)…
```

struct 加欄位(在 `last_output` 之後):

```rust
    /// 輸出工廠:respawn 時重建 writer,讓新 session 續串流(GUI pane 不靜止)
    make_output: Box<dyn FnMut() -> Box<dyn Write + Send> + Send>,
```

`Ok(Self { ... })` 補 `make_output,`。

`spawn` / `spawn_continue` 簽名同步改(`output` → `make_output`)並下傳:

```rust
    pub fn spawn(
        workdir: &Path,
        config_dir: Option<&Path>,
        make_output: Box<dyn FnMut() -> Box<dyn Write + Send> + Send>,
    ) -> Result<Self, CliError> {
        Self::spawn_program(&["claude"], vec!["claude".to_string()], workdir, config_dir, make_output)
    }
    // spawn_continue 同型
```

`respawn` 改用 `self.make_output`(不再 `sink()`)。因 `make_output` 是 `FnMut` 且 struct 會被 `*self = fresh` 覆寫,先把工廠 move 進新 session:need 取出既有工廠。作法:respawn 建新 session 時,把 `self.make_output` 以 `std::mem::replace` 換出一個暫時 noop,交給 `spawn_program`:

```rust
    fn respawn(&mut self) -> Result<(), CliError> {
        let respawn_argv = self.respawn_argv.clone();
        let workdir = self.workdir.clone();
        let config_dir = self.config_dir.clone();
        // 取出工廠交給新 session(fresh 會連工廠一起持有);建失敗則放回原狀
        let noop: Box<dyn FnMut() -> Box<dyn Write + Send> + Send> =
            Box::new(|| Box::new(std::io::sink()));
        let taken = std::mem::replace(&mut self.make_output, noop);
        let argv: Vec<&str> = respawn_argv.iter().map(String::as_str).collect();
        match Self::spawn_program(&argv, respawn_argv.clone(), &workdir, config_dir.as_deref(), taken) {
            Ok(fresh) => {
                // 舊 session drop → teardown;新 session 續用工廠串流(GUI pane 不靜止)
                *self = fresh;
                Ok(())
            }
            Err(e) => {
                // 建失敗:工廠已 move 進 spawn_program 並在其失敗路徑 drop——
                // 無法取回原工廠,但 self 仍是舊 session(仍在串流);後續 respawn
                // 會以 noop 工廠重建(GUI 期若走到此,pane 靜止但 session 可用,
                // 屬極端失敗降級,可接受)。回 Err 供 orchestrator 記錄重試
                Err(e)
            }
        }
    }
```

> 註:`spawn_program` 失敗即 CLI 起不來(罕見),此時管線本就要 recover;工廠遺失只影響 GUI pane 顯示,不影響安全/正確性。若要更嚴謹可讓工廠 `Clone`,但 `Box<dyn FnMut>` 不可 Clone——保留降級語意,Task 9 的 GUI 工廠設計成「重建 emitter 便宜」以最小化此窗。

- [ ] **Step 2: 更新 pty.rs 內所有測試呼叫**

`mod tests` 內所有 `spawn_program(... Box::new(std::io::sink()))` 與 Task 2 新測試的末參數改為工廠形式:

```rust
        Box::new(|| Box::new(std::io::sink())),
```

(共 5 處:`drop_kills_child_process`、`config_dir_sets_env_for_child`、`no_config_dir_leaves_env_unset`、`drop_escalates_to_sigkill_when_sighup_trapped`、`respawn_kills_old_child_and_starts_new`、加 Task 2 的兩個 wait_idle 測試)

- [ ] **Step 3: 更新 bin 呼叫端**

`smoke.rs`:`ClaudePtySession::spawn(&runtime, None, Box::new(std::io::stdout()))` →

```rust
    let mut session = ClaudePtySession::spawn(&runtime, None, Box::new(|| Box::new(std::io::stdout())))
        .expect("spawn echo");
```

`pipeline.rs`:taster/cyrano 兩處的 `Box::new(std::io::stdout())` → `Box::new(|| Box::new(std::io::stdout()))`。

- [ ] **Step 4: 全套件 GREEN**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: 全綠,無 warning

Run: `cargo build --manifest-path src-tauri/Cargo.toml --bin pipeline --bin smoke`
Expected: `Finished`,無 warning

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/adapters/pty.rs src-tauri/src/bin/pipeline.rs src-tauri/src/bin/smoke.rs
git commit -m "feat: pty output 工廠——respawn 後續串流(消除 sink() 缺口,GUI pane 不靜止)"
```

---

### Task 4: orchestrator 注入前 wait_idle 編排(設計輸入 A/C 一次解決)

**Files:**
- Modify: `src-tauri/src/app.rs`(exec 的訊息注入前呼叫 wait_idle)
- Modify: `src-tauri/tests/orchestrator.rs`(注入前 idle 測試)

**背景(必讀)**:findings「設計輸入 A/C」的統一解——A(連發之間)與 C(respawn 之後)同屬「注入時機 vs CLI 真實可用狀態落差」。因 recover() 後的**下一則** `process_update` 的第一個 `InjectTaster` 就會經過本編排的 wait_idle,**C 自動被覆蓋**(新 spawn 的 CLI 未靜默就緒前不會被注入),無需在 recover 內另加 settle。同理 A(每則注入前都等 idle)。

**設計決策(controller 應複核)**:wait_idle 只加在**訊息注入**(`InjectTaster`/`InjectCyrano`),不加在 `ClearTaster`(控制指令,`/clear` 後的節奏由既有 `CONTROL_BUFFER` 負責,兩者互補不重複)。wait_idle **Timeout 不視為 session 失敗**——best-effort 續注入(卡對話框時 GUI 使用者可經 pane 介入;headless 則沿舊行為續注入)。此選擇讓 idle 偵測是**注入穩健性增益**而非新失敗模式,不改任何既有 outcome 語意。

**約束落點:**
- **編排在 orchestrator(Global Constraints 5)**:此處呼叫 wait_idle,門檻取 core 常數,機制委派 adapter。
- **依賴規則(Global Constraints 4)**:`app.rs` 只 import core + ports;附 `^use` rg。
- **不重寫決策邏輯(Global Constraints 8)**:只在既有 `exec` 的兩個訊息注入分支前插一行 wait_idle,狀態機/recover/poll 不動。

**Interfaces:**
- Consumes:Task 1 的 `CliSession::wait_idle`、`session::IDLE_QUIET`、`session::IDLE_SETTLE_TIMEOUT`。
- Produces:`exec(InjectTaster/InjectCyrano)` 注入前 settle;私有 helper `settle_before_inject`。

- [ ] **Step 1: 追加注入前 idle 測試,確認 RED**

在 `src-tauri/tests/orchestrator.rs` 追加(沿用檔頭既有 helper):

```rust
#[test]
fn injects_only_after_waiting_for_idle() {
    // 設計輸入 A/C:每則訊息注入前必先等 CLI 就緒(wait_idle),消除掉字空窗
    let gateway = FakeGateway::new();
    let mut taster = ScriptedCli::new(vec![ok_artifact(&taster_artifact(SAFE_VERDICT))]);
    let mut cyrano = ScriptedCli::new(vec![ok_artifact(r#"{"text":"回覆"}"#)]);
    let mut store = InMemoryStore::new();
    let (_slept, sleeper) = recording_sleeper();

    let outcome = {
        let mut orchestrator = Orchestrator::new(
            &gateway, &mut taster, &mut cyrano, &mut store,
            PipelineConfig::default(), sleeper,
        );
        orchestrator.process_update(&text_update(1, 42, "hi"))
    };

    assert_eq!(outcome, Some(MessageOutcome::Replied));
    assert!(taster.idle_waits >= 1, "taster 注入前必須先 wait_idle");
    assert!(cyrano.idle_waits >= 1, "cyrano 注入前必須先 wait_idle");
}

#[test]
fn idle_timeout_still_injects_best_effort() {
    // wait_idle 逾時(卡就緒偵測)不得變成失敗:best-effort 續注入,仍走完管線
    let gateway = FakeGateway::new();
    let mut taster = ScriptedCli::new(vec![ok_artifact(&taster_artifact(SAFE_VERDICT))]);
    taster.idle_timeouts = 1; // taster 首次 wait_idle 回 Timeout
    let mut cyrano = ScriptedCli::new(vec![ok_artifact(r#"{"text":"回覆"}"#)]);
    let mut store = InMemoryStore::new();
    let (_slept, sleeper) = recording_sleeper();

    let outcome = {
        let mut orchestrator = Orchestrator::new(
            &gateway, &mut taster, &mut cyrano, &mut store,
            PipelineConfig::default(), sleeper,
        );
        orchestrator.process_update(&text_update(1, 42, "hi"))
    };

    assert_eq!(outcome, Some(MessageOutcome::Replied), "idle 逾時仍應 best-effort 完成");
    assert_eq!(taster.messages.len(), 1, "逾時後仍注入(訊息有送達 taster)");
}
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test orchestrator`
Expected: 兩個新測試 FAILED(`taster.idle_waits` 為 0——尚無編排);既有測試 PASS

- [ ] **Step 2: exec 注入前 settle**

在 `src-tauri/src/app.rs`,`use crate::core::session;`(已 import)。`exec` 的兩個訊息注入分支改為先 settle:

```rust
            Action::InjectTaster(text) => {
                self.settle_before_inject(AwaitTarget::Taster);
                self.taster.inject_message(&text).map_err(cli_failure)
            }
            // …ClearTaster 不變(CONTROL_BUFFER 負責控制指令節奏)…
            Action::InjectCyrano(text) => {
                self.settle_before_inject(AwaitTarget::Cyrano);
                self.cyrano.inject_message(&text).map_err(cli_failure)
            }
```

在 `impl<'a> Orchestrator<'a>` 內加私有 helper(緊接 `recover` 之後):

```rust
    /// 注入前就緒偵測(findings「設計輸入 A/C」):等 CLI 的 PTY 靜默達門檻。
    /// 逾時(卡就緒/對話框)不視為失敗——best-effort 續注入(GUI 使用者可經
    /// pane 人工介入)。此編排同時覆蓋 A(連發之間)與 C(recover 後的首次注入)
    fn settle_before_inject(&mut self, target: AwaitTarget) {
        let session: &mut dyn CliSession = match target {
            AwaitTarget::Taster => &mut *self.taster,
            AwaitTarget::Cyrano => &mut *self.cyrano,
        };
        if session
            .wait_idle(session::IDLE_QUIET, session::IDLE_SETTLE_TIMEOUT)
            .is_err()
        {
            eprintln!("[clacks] wait_idle 逾時,best-effort 續注入({target:?})");
        }
    }
```

(`AwaitTarget` 已在 `use crate::core::pipeline::{... AwaitTarget ...}` 內。)

- [ ] **Step 3: GREEN + 依賴規則檢查**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: 全綠,無 warning

Run: `rg -n '^use ' src-tauri/src/app.rs | rg 'adapters|portable_pty|rusqlite|notify|reqwest|tokio|tauri'`
Expected: 無輸出(exit code 1)——orchestrator 只 import core + ports

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/app.rs src-tauri/tests/orchestrator.rs
git commit -m "feat: orchestrator 注入前 wait_idle——A(連發)與 C(respawn 後)一次解決,逾時 best-effort"
```

---

### Task 5: 刪除無消費端的 Clock port(Phase 4 承接裁決落地)

**Files:**
- Modify: `src-tauri/src/ports.rs`(刪 `Clock` trait)
- Delete: `src-tauri/src/adapters/clock.rs`
- Modify: `src-tauri/src/adapters/mod.rs`(移除 `pub mod clock;`)
- Modify: `src-tauri/tests/support/mod.rs`(刪 `ManualClock` + import 的 `Clock`)
- Modify: `src-tauri/tests/fakes_selftest.rs`(刪 Clock selftest)

**背景(必讀)**:findings「Phase 4 規劃承接項」末條——`Clock` port 與 `SystemClock`/`ManualClock` 自 Phase 3 起**無消費端**(orchestrator 用注入 sleep + port 內部 `recv_timeout`;idle 偵測 Task 2 用單調 `Instant` 而非牆鐘 `SystemTime`,型別本就不合)。裁決指示:Phase 5 確認 GUI timeout/status 是否真需 Clock,否則刪。

**裁決(本任務定案,附理由)**:**刪除**。Phase 5 的三個時間相關需求均不需 Clock port:(1) 注入 idle 偵測用 `Instant`(單調,adapter 內部,Task 2);(2) artifact timeout 已由 `wait_artifact` 的 `recv_timeout(ARTIFACT_TIMEOUT)` 處理(不需注入時鐘);(3) GUI 狀態顯示的是 `MessageOutcome` 詞彙表(Replied/Rejected/…),「經過時間」若要顯示屬前端 `Date.now()` 的呈現層,不進 Rust core。留著是 dead code(architecture.md 依賴規則的反例徵兆之一是無用抽象)。此為**純刪除**任務,五檔皆機械移除單一概念,無邏輯變更——刻意不拆(拆開反而破壞「一次移除一個完整概念」的原子性)。

**約束落點:**
- **依賴規則(Global Constraints 4)**:刪除後 `rg Clock` 應零命中(除非本任務遺漏),完工檢核迴歸。

- [ ] **Step 1: 刪 ports.rs 的 Clock**

移除 `src-tauri/src/ports.rs` 尾端整段(`// ---------- Store / Clock ----------` 的 Clock 部分):

```rust
/// 現在時刻。timeout / session 維護決策要可測,時間必須是注入的
pub trait Clock {
    fn now(&self) -> SystemTime;
}
```

並把區塊標題改回 `// ---------- Store ----------`;移除檔頭 `use std::time::{Duration, SystemTime};` 的 `SystemTime`(若 `Duration` 他處仍用則保留 `Duration`)——改為 `use std::time::Duration;`。

- [ ] **Step 2: 刪 adapter**

```bash
git rm src-tauri/src/adapters/clock.rs
```

`src-tauri/src/adapters/mod.rs` 移除 `pub mod clock;` 一行。

- [ ] **Step 3: 刪測試替身與 selftest**

`src-tauri/tests/support/mod.rs`:
- import 行移除 `Clock`(第 5–8 行的 `use clacks::ports::{...}` 拿掉 `Clock`);若 `SystemTime` 僅 ManualClock 用則一併移除。
- 移除整個 `pub struct ManualClock { ... }` + `impl ManualClock` + `impl Clock for ManualClock`(約 158–177 行)。

`src-tauri/tests/fakes_selftest.rs`:移除使用 `ManualClock`/`Clock` 的 selftest 段(約 56–60 行附近整個 test fn)。

- [ ] **Step 4: 編譯 + 迴歸掃描**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: 全綠,無 warning(尤其無 unused import 警告)

Run: `rg -n 'Clock|SystemClock|ManualClock' src-tauri/src src-tauri/tests`
Expected: 無輸出(exit code 1)——Clock 概念已徹底移除

- [ ] **Step 5: Commit(結構性刪除,獨立 commit)**

```bash
git add src-tauri/src/ports.rs src-tauri/src/adapters/mod.rs src-tauri/tests/support/mod.rs src-tauri/tests/fakes_selftest.rs
git commit -m "refactor: 刪除無消費端的 Clock port(idle 用 Instant、timeout 用 recv_timeout、狀態用 MessageOutcome——Clock 為 dead code)"
```

---

### Task 6: 設計輸入 B 裁決 — GUI 接管生命週期,headless bin 降級 dev-only

**Files:**
- Modify: `docs/superpowers/notes/2026-07-17-skeleton-findings.md`(append 裁決)
- Modify: `src-tauri/src/bin/pipeline.rs`(doc comment 降級標註,邏輯不動)

**背景(必讀)**:findings「設計輸入 B」——headless pipeline 有兩個缺口:(1) cyrano TUI 經 PTY 把 kitty keyboard protocol 序列(`^[[…u`)打到外層終端 → Ctrl-C 不再產生 SIGINT;(2) pipeline 無信號處理器,被 SIGTERM/`pkill` 殺時 `Drop` teardown 不跑 → 孤兒 claude 子行程。

**裁決(本任務定案,附理由,寫成 Global Constraint 依據)**:
**採「GUI 接管 PTY 生命週期使 B 對受支援路徑失效,headless bin 降級 dev-only,且不對 headless 加信號處理器」。**理由三點:
1. **正面消除而非補丁**:GUI 版 PTY bytes 只進 xterm.js pane、不進控制終端(Global Constraints 2),kitty 序列污染外層終端的根因不復存在,Ctrl-C 語意在 GUI 宿主完全正常;視窗關閉 → GUI stop 路徑主動 drop session → `Drop`→teardown 跑(Global Constraints 6),孤兒問題消除。故 B 對受支援部署路徑(GUI)結構性失效。
2. **headless 加信號處理器與「不重寫 orchestrator」衝突**:`run_forever` 是 `-> !`(永不返回),信號處理器在獨立 thread 無法 drop `main` 堆疊上持有的 session,只能 `process::exit`(不跑 Drop)或自行 SIGKILL 子行程——要跑 teardown 就得改 `run_forever` 讓它以旗標跳出,即重寫 orchestrator 迴圈(Global Constraints 8 禁止)。GUI 版之所以能乾淨關閉,正是因為它用自己的 poll 迴圈(Task 9)而非 `run_forever`。
3. **dev-only 的孤兒清理已有既定程序**:findings「設計輸入 B」已記錄可靠辨識/清理法(`pgrep -P <pid>` 抓直接子行程、`lsof | grep taster/cyrano` 辨角色、`pkill -f 'target/debug/pipeline'`)。dev 場景可接受手動清理,不值得為此違反 Global Constraints 8。

**約束落點:** 此裁決即 Global Constraints 2 與 6 的依據;其「一個任務具體執法」落在 **Task 9**(GUI 的 output 只進 emitter、stop 主動 drop session)與 **Task 12**(真機 pgrep 驗無孤兒)。本任務只定案 + 標註,不編碼。

- [ ] **Step 1: append 裁決到 findings**

在 `docs/superpowers/notes/2026-07-17-skeleton-findings.md` 末尾新增一節「Phase 5 設計輸入 B 裁決(2026-07-20)」,逐字寫入上述三點理由 + 落點(Global Constraints 2/6、Task 9/12);明確寫「headless `bin/pipeline.rs` 自本 phase 起為 **dev-only**,不加信號處理器;受支援部署路徑為 GUI」。

- [ ] **Step 2: pipeline.rs doc comment 降級標註(邏輯零改動)**

在 `src-tauri/src/bin/pipeline.rs` 檔頭 doc comment 補一行(不改任何程式碼行):

```rust
//! **狀態(Phase 5 起):dev-only**。受支援部署路徑為 GUI(見 src/gui.rs)。
//! 本 bin 無信號處理器且 PTY bytes 走 stdout(kitty 序列會污染控制終端、
//! Ctrl-C 失效——findings「設計輸入 B」);停止請於另一終端
//!   pkill -f 'target/debug/pipeline'   # 再以 pgrep -P <pid> 清孤兒 claude
//! 乾淨的信號式關閉需改 run_forever 為可跳出迴圈(重寫 orchestrator),
//! 本 phase 不做——GUI 以自己的 poll 迴圈 + stop 旗標達成乾淨 teardown
```

- [ ] **Step 3: 驗證(邏輯未動)**

Run: `git -C /Users/lukechimbp2023/Documents_local/idea/clacks diff --stat src-tauri/src/bin/pipeline.rs`
Expected: 只有 doc comment 增行(無 `fn`/邏輯行變動)

Run: `cargo build --manifest-path src-tauri/Cargo.toml --bin pipeline`
Expected: `Finished`,無 warning

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/notes/2026-07-17-skeleton-findings.md src-tauri/src/bin/pipeline.rs
git commit -m "docs: 設計輸入 B 裁決——GUI 接管生命週期使缺口失效,headless bin 降級 dev-only(不加信號處理器,理由入 findings)"
```

---

### Task 7: Tauri 後端 scaffolding(空視窗可開)

**Files:**
- Modify: `src-tauri/Cargo.toml`(加 tauri/tauri-build 依賴 + bin/lib 設定)
- Create: `src-tauri/build.rs`
- Create: `src-tauri/tauri.conf.json`
- Create: `src-tauri/src/bin/clacks-gui.rs`(Tauri app 入口)
- Modify: `src-tauri/src/lib.rs`(`pub mod gui;` — gui.rs 於 Task 9 建;本任務先放最小 `run`)
- Create: `src-tauri/src/gui.rs`(最小 `pub fn run()` 開空視窗)

**背景**:repo 目前**無任何 Tauri scaffolding**(無 `tauri.conf.json`、無 `src/` 前端、無 `package.json`——已 `ls` 確認)。本任務建最小可開視窗的 Tauri v2 後端,command/管線接線留 Task 9。前端 `src/` 留 Task 8(本任務的 `frontendDist` 先指向 Task 8 產出目錄,dev 期可先用 devUrl)。

**約束落點:**
- **依賴 pin + 版本敏感(Global Constraints 9)**:以 `cargo add tauri@2` / `cargo add --build tauri-build@2` 安裝,**resolved 版本寫進報告**;`tauri.conf.json` 的 schema 鍵以撰寫時 Tauri 2.x 為準,偏差揭露。授權檢查:Tauri = MIT/Apache-2.0,相容 repo 雙授權。
- **依賴規則(Global Constraints 4)**:tauri 只出現在 `gui.rs` 與入口 bin;core/app 不得 import tauri(完工檢核掃描)。

- [ ] **Step 1: 加依賴(pin + 記錄 resolved 版本)**

Run:
```bash
cargo add tauri@2 --manifest-path src-tauri/Cargo.toml
cargo add tauri-build@2 --build --manifest-path src-tauri/Cargo.toml
```
**報告揭露**:把 `Cargo.toml` 實際寫入的版本字串(如 `tauri = "2.x.y"`)記進實作報告——Global Constraints 9。若 features 需調整(如 window/webview 相關),揭露實際 feature 名。

在 `Cargo.toml` 補 lib 與 bin 設定(Tauri app 走 `clacks::gui::run()`):

```toml
[lib]
name = "clacks"
# 既有 bins(pipeline/smoke)沿用 src/bin/;新增 GUI 入口 bin

[[bin]]
name = "clacks-gui"
path = "src/bin/clacks-gui.rs"
```

- [ ] **Step 2: build.rs**

`src-tauri/build.rs`:

```rust
fn main() {
    tauri_build::build();
}
```

- [ ] **Step 3: tauri.conf.json(最小)**

`src-tauri/tauri.conf.json`(鍵以撰寫時 Tauri 2.x schema 為準,偏差揭露):

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Clacks",
  "version": "0.1.0",
  "identifier": "dev.clacks.app",
  "build": {
    "frontendDist": "../src",
    "devUrl": "http://localhost:5173"
  },
  "app": {
    "windows": [
      { "title": "Clacks", "width": 1200, "height": 800 }
    ],
    "security": { "csp": null }
  },
  "bundle": { "active": false }
}
```

- [ ] **Step 4: 最小 gui.rs + 入口 bin**

`src-tauri/src/gui.rs`:

```rust
//! GUI composition root(architecture.md 依賴規則 4:唯一知道具體型別 + tauri 的地方)。
//! 本檔是 Phase 5 新增的組裝點,類比 bin/pipeline.rs 但由 Tauri 事件迴圈驅動。
//! Task 7:最小開窗;Task 9:接 command + 管線 thread + emitter。

pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("啟動 Tauri 應用失敗");
}
```

`src-tauri/src/lib.rs` 補一行:`pub mod gui;`

`src-tauri/src/bin/clacks-gui.rs`:

```rust
fn main() {
    clacks::gui::run();
}
```

- [ ] **Step 5: 驗證(編譯;開窗屬 human)**

Run: `cargo build --manifest-path src-tauri/Cargo.toml --bin clacks-gui`
Expected: `Finished`(Tauri context 生成成功、依賴解析成功)。**若** `generate_context!` 因缺 `frontendDist`(Task 8 尚未建 `src/`)而失敗,先建 `src/index.html` 佔位一行 `<!doctype html><title>Clacks</title>`(Task 8 覆寫)並在報告註明順序調整。

Run(依賴規則迴歸):`rg -n '^use tauri' src-tauri/src/core/ src-tauri/src/app.rs`
Expected: 無輸出——core/app 未 import tauri

- [ ] **Step 6: (human) 開窗確認**

(human)Run: `cargo run --manifest-path src-tauri/Cargo.toml --bin clacks-gui`
Expected(human 觀察):一個標題 `Clacks`、1200×800 的空視窗開啟(內容空白屬正常,前端 Task 8 補)。記錄 OS 是否跳 webview 權限提示。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/build.rs src-tauri/tauri.conf.json src-tauri/src/gui.rs src-tauri/src/lib.rs src-tauri/src/bin/clacks-gui.rs
git commit -m "feat: Tauri 後端 scaffolding——最小可開窗 GUI 入口(tauri v2 pin,gui.rs 為新 composition root)"
```

---

### Task 8: 前端 scaffolding — 兩 pane + 狀態列骨架(不接線)

**Files:**
- Create: `package.json`(repo root)
- Create: `src/index.html`
- Create: `src/main.ts`
- Create: `src/styles.css`

**背景**:architecture.md 指定前端在 repo root `src/`、**不分層**(兩塊 xterm.js pane + 狀態顯示,保持薄)。本任務只建靜態骨架(兩個 pane 容器、狀態列、每 pane 一個人工輸入框、start/stop 按鈕),xterm 實例化即可,**不接 Tauri 事件**(Task 10 接線)。

**約束落點:**
- **依賴 pin + 版本敏感(Global Constraints 9)**:`@xterm/xterm@5.5.0`(注意 5.x 起套件名為 `@xterm/xterm`,非舊 `xterm`)、`@tauri-apps/api@2`、`@tauri-apps/cli@2`、`vite`;resolved 版本入報告,import 路徑偏差揭露。授權:xterm MIT、Tauri MIT/Apache——相容。
- **前端不分層(刻意不做)**:純 TS + xterm,無框架、無狀態庫。

- [ ] **Step 1: package.json + 安裝**

`package.json`(repo root):

```json
{
  "name": "clacks-frontend",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "preview": "vite preview"
  }
}
```

Run(pin + 記錄 resolved 版本):
```bash
npm install @xterm/xterm@5.5.0 @xterm/addon-fit@0.10.0
npm install -D @tauri-apps/cli@2 vite
npm install @tauri-apps/api@2
```
**報告揭露**:記錄 `package.json`/lock 實際 resolved 版本;若 `@xterm/addon-fit` 版本號不同或 import 路徑不同,揭露。

- [ ] **Step 2: index.html + styles.css**

`src/index.html`(兩 pane + 狀態列 + 每 pane 人工輸入 + 控制按鈕):

```html
<!doctype html>
<html lang="zh-Hant">
  <head>
    <meta charset="UTF-8" />
    <title>Clacks</title>
    <link rel="stylesheet" href="./styles.css" />
    <link rel="stylesheet" href="/node_modules/@xterm/xterm/css/xterm.css" />
  </head>
  <body>
    <div id="controls">
      <button id="start">啟動管線</button>
      <button id="stop" disabled>停止</button>
      <span id="pipeline-state">未啟動</span>
    </div>
    <div id="panes">
      <section class="pane">
        <h2>taster(消毒者)</h2>
        <div id="taster-term" class="term"></div>
        <input class="manual" data-role="taster" placeholder="人工輸入(trust/login 對話框)…" />
      </section>
      <section class="pane">
        <h2>cyrano(回應者)</h2>
        <div id="cyrano-term" class="term"></div>
        <input class="manual" data-role="cyrano" placeholder="人工輸入…" />
      </section>
    </div>
    <div id="status">
      <h2>訊息結果</h2>
      <ul id="outcomes"></ul>
    </div>
    <script type="module" src="./main.ts"></script>
  </body>
</html>
```

`src/styles.css`:兩欄 pane grid + 狀態列(最小可讀樣式;內容不 load-bearing,略)。

- [ ] **Step 3: main.ts 骨架(只實例化 xterm,不接事件)**

```ts
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";

function mountTerm(id: string): Terminal {
  const term = new Terminal({ convertEol: true, fontSize: 12 });
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.open(document.getElementById(id)!);
  fit.fit();
  return term;
}

const taster = mountTerm("taster-term");
const cyrano = mountTerm("cyrano-term");
taster.write("taster pane 就緒(等待管線啟動)\r\n");
cyrano.write("cyrano pane 就緒\r\n");

// Task 10 接線:listen("pty://taster"/"pty://cyrano") → term.write;
// listen("outcome") → 狀態列;按鈕 invoke("start_pipeline"/"stop_pipeline");
// 人工輸入 invoke("send_input")。本任務僅骨架。
export { taster, cyrano };
```

- [ ] **Step 4: 驗證(前端 build 成功)**

Run: `npm run build`
Expected: Vite build 成功(`dist/` 或依 config 產出;無 TS import 解析錯誤)。**若** xterm CSS 路徑或 addon import 名與 pin 版本不符,揭露實際路徑並修正。

Run: `rg -n 'invoke|listen' src/main.ts`
Expected: 無輸出——本任務刻意不接線(Task 10 才有)

- [ ] **Step 5: Commit**

```bash
git add package.json src/index.html src/main.ts src/styles.css
git commit -m "feat: 前端 scaffolding——兩 xterm pane + 狀態列骨架(不分層,未接線,xterm 5.5 pin)"
```

---

### Task 9: GUI composition root — Tauri command + 管線 thread + PTY emitter

**Files:**
- Modify: `src-tauri/src/gui.rs`(command、管線 thread、emitter、stop 旗標)

**背景(必讀)**:本檔是 GUI 的唯一組裝點(類比 `bin/pipeline.rs`),但由 Tauri 事件迴圈驅動而非阻塞 `run_forever`。它:(1) 以既有 `ClaudePtySession::spawn`/`spawn_continue`(隔離 spawn 路徑不變)+ output 工廠(Task 3)產生 **PTY emitter**(bytes → `emit("pty://taster"/"cyrano")`)→ xterm.js;(2) 在**自己的 std::thread poll 迴圈**跑既有 `poll_once`(Global Constraints 8:不改 orchestrator),把每個 `MessageOutcome` `emit("outcome", …)`;(3) `stop` 以旗標讓 poll 迴圈跳出後 join → session drop → teardown(Global Constraints 6);(4) `send_input` 把使用者輸入經 `write_raw` 送進對應 pane(findings:trust/login 對話框需人工介入通道,GUI pane + 輸入即此通道,消除 headless 的 pre-seed 死鎖需求)。

**約束落點(逐一具體化):**
- **token 絕不進 webview(Global Constraints 1)**:command 模組不 import raw token;`gateway = TelegramHttp::from_env()` 只在管線 thread 內部持有,**不**放進任何 command 回傳/`emit` payload。emit 的只有 (pane bytes, MessageOutcome debug 字串, pipeline state)。附 rg 驗證。
- **PTY 只進 webview(Global Constraints 2)**:output 工廠回傳的 emitter 只 `emit`,**不寫 stdout**;taster/cyrano 兩個工廠分別 emit 各自 topic。
- **隔離 spawn 路徑不繞過(Global Constraints 3)**:用既有 `spawn(&taster_dir, Some(&cli_config), factory)` / `spawn_continue`,workdir 在 repo 外;不新開 spawn。
- **乾淨關閉(Global Constraints 6)**:stop 旗標 + join + drop → teardown。
- **不重寫 orchestrator(Global Constraints 8)**:poll 迴圈用 `poll_once` + `poll_backoff`(皆公開已測),不碰 `run_forever`。
- **依賴規則(Global Constraints 4)**:tauri 只在本檔。

- [ ] **Step 1: emitter(實作 std::io::Write,bytes → Tauri 事件)**

在 `gui.rs` 加(emitter 只 emit、絕不寫 stdout——Global Constraints 2):

```rust
use std::io::Write;
use tauri::{AppHandle, Emitter};

/// PTY bytes → webview 事件(pane 專屬 topic)。實作 Write 以接進 pty output 工廠。
/// 只 emit,不寫控制終端(Global Constraints 2:kitty 序列不污染宿主終端)
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
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}
```

- [ ] **Step 2: 管線狀態 + start/stop/send_input command**

用 `AtomicBool` 停止旗標 + `Mutex<Option<JoinHandle>>` 管理管線 thread;送人工輸入用一對 channel(Sender 存 state,thread 內把 bytes 經 `write_raw` 送對應 session)。骨架:

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{Sender, Receiver};

#[derive(Default)]
struct PipelineState {
    running: Arc<AtomicBool>,
    handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    // 人工輸入:(role, bytes) 送進管線 thread,thread 內 write_raw 到對應 session
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
    *state.input_tx.lock().unwrap() = None;
    if let Some(h) = state.handle.lock().unwrap().take() {
        let _ = h.join(); // join → thread 內 session drop → Drop→teardown(Global Constraints 6)
    }
    Ok(())
}

#[tauri::command]
fn send_input(role: String, data: String, state: tauri::State<PipelineState>) -> Result<(), String> {
    // 人工介入通道(findings:trust/login 對話框)。data 原樣送 write_raw(含 \r 由前端決定)
    if let Some(tx) = state.input_tx.lock().unwrap().as_ref() {
        tx.send((role, data.into_bytes())).map_err(|e| e.to_string())
    } else {
        Err("管線未執行".into())
    }
}
```

- [ ] **Step 3: 管線 thread(組裝 + 自有 poll 迴圈,不用 run_forever)**

```rust
fn pipeline_thread(app: AppHandle, running: Arc<AtomicBool>, input_rx: Receiver<(String, Vec<u8>)>) {
    use clacks::adapters::pty::ClaudePtySession;
    use clacks::adapters::store::SqliteStore;
    use clacks::adapters::telegram::TelegramHttp;
    use clacks::app::{Orchestrator, PipelineConfig};
    use clacks::core::pipeline::poll_backoff;
    use clacks::ports::TelegramGateway;

    // 路徑組裝與 pipeline.rs 同構:runtime 外置、CLAUDE_CONFIG_DIR 共用 cli-config、
    // canonicalize 絕對路徑(Task 9 真機 bug #1 教訓)。缺部署/pre-seed → emit 錯誤事件並返回
    // …（省略:與 bin/pipeline.rs 相同的 runtime/cli_config 檢查,失敗改 app.emit("fatal", msg) 而非 exit）…

    let gateway = TelegramHttp::from_env(); // token 封裝在此,絕不 emit(Global Constraints 1)
    // webhook 互斥檢查同 pipeline.rs（失敗 emit fatal 並返回）

    // output 工廠(Task 3):每(re)spawn 產生 emitter → 各自 pane topic(Global Constraints 2)
    let app_t = app.clone();
    let taster_factory = Box::new(move || {
        Box::new(PaneEmitter { app: app_t.clone(), topic: "pty://taster" }) as Box<dyn Write + Send>
    });
    let app_c = app.clone();
    let cyrano_factory = Box::new(move || {
        Box::new(PaneEmitter { app: app_c.clone(), topic: "pty://cyrano" }) as Box<dyn Write + Send>
    });

    let mut taster = match ClaudePtySession::spawn(&taster_dir, Some(&cli_config), taster_factory) {
        Ok(s) => s, Err(e) => { let _ = app.emit("fatal", format!("spawn taster: {}", e.0)); return; }
    };
    let mut cyrano = match ClaudePtySession::spawn_continue(&cyrano_dir, Some(&cli_config), cyrano_factory) {
        Ok(s) => s, Err(e) => { let _ = app.emit("fatal", format!("spawn cyrano: {}", e.0)); return; }
    };
    let mut store = match SqliteStore::open(&runtime.join("clacks.db")) {
        Ok(s) => s, Err(e) => { let _ = app.emit("fatal", format!("open store: {}", e.0)); return; }
    };

    let mut orchestrator = Orchestrator::new(
        &gateway, &mut taster, &mut cyrano, &mut store,
        PipelineConfig::default(), Box::new(|d| std::thread::sleep(d)),
    );

    // 自有 poll 迴圈:取代 run_forever(-> !),讓 stop 旗標能跳出 → 乾淨 teardown(GC 6/8)。
    // 邏輯 = run_forever 的可測部分(poll_once + poll_backoff),不改 orchestrator
    let _ = app.emit("state", "running");
    let mut offset = 0i64;
    let mut consecutive = 0u32;
    while running.load(Ordering::SeqCst) {
        // 先排空人工輸入(trust/login 對話框介入)
        while let Ok((role, bytes)) = input_rx.try_recv() {
            // 經 write_raw 送對應 session(orchestrator 未持有時直接對 session；
            // 此處示意——實作以 orchestrator 借用期安排,或於 poll 空檔 drain）
            let _ = (&role, &bytes);
        }
        match orchestrator.poll_once(offset) {
            Ok((next, outcomes)) => {
                consecutive = 0;
                offset = next;
                for o in &outcomes {
                    let _ = app.emit("outcome", format!("{o:?}")); // MessageOutcome debug（無 token）
                }
            }
            Err(e) => {
                consecutive += 1;
                let _ = app.emit("poll-error", e.0); // GatewayError 已 redact（GC 1）
                std::thread::sleep(poll_backoff(consecutive));
            }
        }
    }
    // 迴圈跳出 → orchestrator drop → taster/cyrano drop → Drop→teardown（GC 6）
    let _ = app.emit("state", "stopped");
}
```

> 註(人工輸入借用):`Orchestrator` 借用 `&mut taster/cyrano`,poll 期間無法同時外部 `write_raw`。實作採「poll 空檔 drain input」——`poll_once` 回來後、下一輪前排空 `input_rx` 並經 orchestrator 尚未持有的短窗寫入;或給 orchestrator 加一個 `write_raw_to(target, bytes)` 轉呼叫(不改決策邏輯,只轉發既有 port 方法)。**實作者擇一並在報告說明**;若加轉發方法,附一行單元/整合測試證明轉發到對的 session。

- [ ] **Step 4: 註冊 command + state**

`run()` 改為:

```rust
pub fn run() {
    tauri::Builder::default()
        .manage(PipelineState::default())
        .invoke_handler(tauri::generate_handler![start_pipeline, stop_pipeline, send_input])
        .run(tauri::generate_context!())
        .expect("啟動 Tauri 應用失敗");
}
```

- [ ] **Step 5: 編譯 + 安全紅線 rg**

Run: `cargo build --manifest-path src-tauri/Cargo.toml --bin clacks-gui`
Expected: `Finished`,無 warning

Run(token 不進 webview,Global Constraints 1):`rg -n 'CLACKS_BOT_TOKEN|from_env' src-tauri/src/gui.rs`
Expected: 僅 `TelegramHttp::from_env()` 一處(在 thread 內部);**無** `CLACKS_BOT_TOKEN` 字面、無把 token/gateway 放進 `emit`/command 回傳

Run(emit 的 topic 皆非 token 來源;人工核對):`rg -n 'app.emit|\.emit\(' src-tauri/src/gui.rs`
Expected: 只出現 `pty://taster`、`pty://cyrano`、`outcome`、`state`、`poll-error`、`fatal`——payload 皆為 pane bytes / MessageOutcome / redacted 錯誤字串,無 token

Run(依賴規則):`rg -n '^use tauri' src-tauri/src/core/ src-tauri/src/app.rs`
Expected: 無輸出——tauri 只在 gui.rs/入口 bin

Run(隔離 spawn 未繞過,Global Constraints 3):`rg -n 'spawn|spawn_continue' src-tauri/src/gui.rs` 且 `rg -n 'env_clear|ENV_ALLOWLIST' src-tauri/src/adapters/pty.rs`
Expected: gui 用既有 `spawn`/`spawn_continue`;pty 的 env_clear/白名單原樣健在

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/gui.rs
git commit -m "feat: GUI composition root——Tauri command + 自有 poll 迴圈(不改 orchestrator)+ PTY emitter(token 不進 webview,stop 乾淨 teardown)"
```

---

### Task 10: 前端接線 — 事件 → pane/狀態列,按鈕/輸入 → command

**Files:**
- Modify: `src/main.ts`(listen 事件、invoke command、人工輸入)

**背景**:把 Task 8 的骨架接上 Task 9 的 command/事件。前端保持薄(architecture.md:只 render PTY 輸出 + 狀態,無業務邏輯):`pty://*` → `term.write`;`outcome`/`state`/`fatal`/`poll-error` → 狀態列;按鈕 → `invoke("start_pipeline"/"stop_pipeline")`;人工輸入框 Enter → `invoke("send_input")`(送 trust/login 對話框介入,附 `\r`)。

**約束落點:**
- **前端不分層 / 無業務邏輯(刻意不做)**:main.ts 只做事件↔DOM 轉發,不含任何管線決策。
- **PTY 只 render 不回傳(Global Constraints 2 前端面)**:pane 只 `term.write`;pane 不把內容送回任何外部端點。

- [ ] **Step 1: 接事件到 pane / 狀態列**

```ts
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

listen<string>("pty://taster", (e) => taster.write(e.payload));
listen<string>("pty://cyrano", (e) => cyrano.write(e.payload));

const outcomes = document.getElementById("outcomes")!;
listen<string>("outcome", (e) => {
  const li = document.createElement("li");
  li.textContent = `${new Date().toLocaleTimeString()}  ${e.payload}`; // 經過時間屬呈現層（非 Clock port）
  outcomes.prepend(li);
});
const stateEl = document.getElementById("pipeline-state")!;
listen<string>("state", (e) => { stateEl.textContent = e.payload; });
listen<string>("fatal", (e) => { stateEl.textContent = `fatal: ${e.payload}`; });
listen<string>("poll-error", (e) => { stateEl.textContent = `poll-error: ${e.payload}`; });
```

- [ ] **Step 2: 按鈕 + 人工輸入 → command**

```ts
const startBtn = document.getElementById("start") as HTMLButtonElement;
const stopBtn = document.getElementById("stop") as HTMLButtonElement;
startBtn.onclick = async () => {
  await invoke("start_pipeline");
  startBtn.disabled = true; stopBtn.disabled = false;
};
stopBtn.onclick = async () => {
  await invoke("stop_pipeline");
  startBtn.disabled = false; stopBtn.disabled = true;
};

// 人工介入(trust/login 對話框):Enter 送該 pane,附 \r 觸發
document.querySelectorAll<HTMLInputElement>("input.manual").forEach((box) => {
  box.addEventListener("keydown", async (ev) => {
    if (ev.key === "Enter") {
      await invoke("send_input", { role: box.dataset.role, data: box.value + "\r" });
      box.value = "";
    }
  });
});
```

- [ ] **Step 3: 驗證(build + 接線存在)**

Run: `npm run build`
Expected: build 成功,無 TS 錯誤。**若** `@tauri-apps/api` 的 `event`/`core` 匯出路徑與 pin 版本不同(如舊版是 `@tauri-apps/api/tauri`),揭露實際路徑並修正(Global Constraints 9)

Run: `rg -n 'invoke\("start_pipeline"\)|invoke\("stop_pipeline"\)|invoke\("send_input"|listen<string>\("pty://' src/main.ts`
Expected: 四類接線皆命中(start/stop/send_input + pty listen)

- [ ] **Step 4: Commit**

```bash
git add src/main.ts
git commit -m "feat: 前端接線——PTY 事件→xterm、outcome→狀態列、按鈕/人工輸入→Tauri command(薄前端無業務邏輯)"
```

---

### Task 11:(human)HOME 重定位完全隔離真機驗證(有界,非阻斷)

**Files:**
- Modify: `docs/superpowers/notes/2026-07-17-skeleton-findings.md`(append 真機結果 + 裁決)

**背景(必讀)**:findings「Phase 4 ~/.claude 隔離調查裁決」第 3 點——`CLAUDE_CONFIG_DIR` 隔了 MCP/model/plugins/settings/OAuth,**但不隔 user 全域 `$HOME/.claude/CLAUDE.md`**(靠 HOME 定位)。完全隔離候選:把 HOME 指向專屬乾淨家目錄(如 `../clacks-runtime/taster-home`)令 `$HOME/.claude/CLAUDE.md` 解析到受控空檔。但 **HOME 重定位 × OAuth 交互未驗證**(骨架曾認為 HOME 為 OAuth 必需;Phase 4 實證 OAuth 已落 `CLAUDE_CONFIG_DIR/.claude.json`,HOME 或可改)——**不得事前斷言**。

**此為有界的調查+記錄任務,非編碼、非阻斷**:GUI 以既有部分隔離即可運作(Task 12 走既有 `cli-config`)。僅在此把 findings 明示的 Phase 5 驗證項收尾。若真機證實可行,落 `adapters::pty` 的 HOME 佈線屬**後續獨立任務**,不在本 phase。

- [ ] **Step 1:(human)真機測 HOME 重定位 × OAuth**

在真機:設 `../clacks-runtime/taster-home` 為空家目錄,以 `HOME=<該目錄>` + 既有 `CLAUDE_CONFIG_DIR` 啟動一個 probe workdir 的 claude。觀察並記錄:(1) OAuth/登入是否仍可用(或要求重登);(2) 啟動時是否**不再**跳 user 全域 `CLAUDE.md`(`RTK.md`)的 external-imports 對話框;(3) 是否有其他 HOME 相依功能失效。

- [ ] **Step 2:(human)append findings + 裁決**

在 findings 新增「Phase 5 HOME 重定位完全隔離驗證(2026-07-20)」節,三選一記錄(附實測輸出,非「確認正常」):
- **可行**:HOME 重定位不破 OAuth 且消掉 user CLAUDE.md 洩漏 → 記錄機制,標「落 `adapters::pty` HOME 佈線為後續獨立任務」。
- **部分/有代價**:記錄哪些失效、代價為何。
- **不可行(fallback)**:HOME 為某功能(如 OAuth)必需 → 維持 Phase 4 部分隔離 + taster `--append-system-prompt` + core 契約兜底;記錄之。

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/notes/2026-07-17-skeleton-findings.md
git commit -m "findings:(human)HOME 重定位完全隔離真機驗證——OAuth 交互裁決(非阻斷,佈線留後續)"
```

---

### Task 12:(human)GUI 真機端到端 smoke(最後一個 task)

**Files:**
- 無程式碼改動(執行 + 記錄);真機發現與門檻校正回填 `docs/superpowers/notes/`,如需微調常數各自獨立 commit(揭露偏差)

**背景**:前 11 個 task 的正確性只到「編譯 + fake ports + 單元 + 空窗」層級。GUI 的真角色生效、idle 偵測是否真的消除掉字(設計輸入 A)、respawn 後 settle(C)、狀態顯示、乾淨 teardown(B)、人工介入通道,必須真機端到端跑一輪(參照 Phase 4 Task 9 checklist 風格)。**本 agent 無法執行**(需 Telegram、真 claude CLI、GUI 視窗)——human 任務。

**前置(human):**
- 部署 runtime:`cp -R templates/taster ../clacks-runtime/taster`、`cp -R templates/cyrano ../clacks-runtime/cyrano`;`cli-config` 已 pre-seed(或經 GUI pane 人工完成 onboarding/login/trust——見下)
- `CLACKS_BOT_TOKEN` 經 Keychain 注入;確認無其他行程用同 token 打 `getUpdates`(否則 409)
- 前端:`npm run build`(或 dev);啟動:`CLACKS_BOT_TOKEN=$(security find-generic-password -s clacks-bot -w) cargo run --manifest-path src-tauri/Cargo.toml --bin clacks-gui`

**Checklist(逐項真機勾稽,記錄實際觀察):**

- [ ] **開窗 + 啟動**:按「啟動管線」→ 狀態列顯示 `running`;兩 pane 出現 taster/cyrano 的 TUI 開機輸出(emitter → xterm 端到端)。
- [ ] **人工介入通道(findings 必要品)**:若 `cli-config` 未 pre-seed,啟動時 pane 應顯示 trust/login/theme 對話框——在該 pane 的人工輸入框完成互動(消除 headless pre-seed 死鎖需求)。記錄是否成功穿透。
- [ ] **乾淨訊息端到端**:發一則普通問句 → taster `safe:true` → cyrano 生成 → Telegram 收到回覆;狀態列出現 `Replied`。
- [ ] **惡意訊息**:發 prompt injection → 狀態列 `RejectedByTaster { reason }`,不回覆。
- [ ] **設計輸入 A 驗證(核心)**:**連發 ~1s 兩則短訊** → 兩則都完整處理、**無空內容/EmptyReply**(對照 Phase 4 #7:第二則 sanitized_text 掉字)。若仍掉字 → `IDLE_QUIET` 太短,量測實際收尾空窗、調大並回填(Global Constraints 7);記錄前後對照。
- [ ] **設計輸入 A 單發驗證**:單發乾淨訊息重複數次,確認無 `{"text":""}`→EmptyReply(對照 Phase 4 #3 的注入吞掉)。記錄 idle 偵測前後的成功率。
- [ ] **設計輸入 C 驗證(respawn 後 settle)**:手動 kill cyrano pid → 下則訊息觸發 `SessionLost`→recover(pane 顯示新 session 開機)→ **緊接的下一則不再撞 EIO/`os error 5`**(對照 Phase 4 #5/#6:recover 後立刻注入失敗要再繞一圈)。因 recover 後首次注入經 wait_idle 等新 session 就緒。記錄是否一次收斂。
- [ ] **設計輸入 B 驗證(乾淨 teardown)**:按「停止」(或關視窗)→ 管線 thread 跳出、join、session drop → `pgrep -P <clacks-gui pid>` 及 `pgrep -f 'claude'` 確認**無本管線遺留的 claude**(對照啟動前既有實例)。Ctrl-C 在 GUI 宿主終端正常(pane 不污染宿主)。
- [ ] **token 不進 webview(Global Constraints 1 真機面)**:全程檢查 pane 輸出、狀態列、devtools 事件 payload 無 `CLACKS_BOT_TOKEN` 值;瞬時 poll 錯誤的 `poll-error` 事件只含 redacted 字串(無 URL/token)。
- [ ] **記錄**:把 idle 門檻實測量級(`IDLE_QUIET` 是否需調)、A/C 前後對照、B 的 pgrep 輸出、任何 Tauri/xterm 版本偏差(Global Constraints 9)寫回 `docs/superpowers/notes/`。若微調常數(如 `IDLE_QUIET`),獨立 commit 並揭露。

---

## 完工檢核(final review 前)

- **Rust 測試全綠、無 warning**:`cargo test --manifest-path src-tauri/Cargo.toml`
  - Phase 5 新增 Rust 測試:Task 1 `idle_quiet_is_shorter_than_settle_timeout`(+1)、Task 2 `wait_idle_returns_after_output_goes_quiet` + `wait_idle_times_out_when_never_quiet`(+2)、Task 4 `injects_only_after_waiting_for_idle` + `idle_timeout_still_injects_best_effort`(+2);Task 5 刪除 Clock selftest(-1)。淨變化疊加 Phase 4 既有全綠
  - `wait_idle_*` 因真實子行程起落與靜默窗各需約 1–3s,屬正常
- **依賴規則掃描(掃 `^use` 陳述,非註解——Phase 3/4 教訓:命中 doc comment 是假陽性;`src-tauri` 路徑含 `tauri` 子字串會讓管線式 rg 假陽性,pattern 要錨在 use 目標上)**:
  - `rg -n '^use (tokio|tauri|notify|portable_pty|rusqlite|reqwest)' src-tauri/src/core/` → 無輸出(core 純淨)
  - `rg -n '^use (crate::adapters|portable_pty|rusqlite|notify|reqwest|tokio|tauri)' src-tauri/src/app.rs` → 無輸出(orchestrator 只 core + ports)
  - `rg -n '^use tauri' src-tauri/src/` → 僅 `gui.rs`(與入口 bin)命中——tauri 只在 composition root
- **安全紅線**:
  - `pty.rs` 的 `env_clear` + `ENV_ALLOWLIST` 與 `minimal_env_excludes_secrets_and_unknowns` 原樣健在(Task 2/3 未動)
  - `gui.rs` 無 `CLACKS_BOT_TOKEN` 字面、`emit` payload 僅 (pane bytes / MessageOutcome / redacted 錯誤)——`rg` 驗證(Task 9 Step 5)
  - `telegram.rs` 的 `redact` / `error_for_status`(Phase 4)原樣——GUI 的 `poll-error` 事件承接其 redacted 字串
- **teardown 必經(Global Constraints 6)**:GUI stop 路徑 drop session(Task 9);真機 pgrep 無孤兒(Task 12)
- **Clock 徹底移除(Task 5)**:`rg -n 'Clock' src-tauri/src src-tauri/tests` → 無輸出
- **composition root 可編譯**:`cargo build --manifest-path src-tauri/Cargo.toml --bin clacks-gui --bin pipeline --bin smoke` → `Finished`
- **前端 build**:`npm run build` → 成功;`@xterm/xterm`、`@tauri-apps/api` resolved 版本已入報告(Global Constraints 9)
- **依賴新增揭露(Global Constraints 9)**:`git diff Cargo.toml package.json` 的新增(tauri/tauri-build/xterm/@tauri-apps/*/vite)版本已 pin 且 resolved 版本入報告;授權相容性(MIT/Apache)已確認
- **真機承接項(Task 11/12 human 記錄)**:設計輸入 A(連發不掉字)、C(respawn 後不撞 EIO)、B(乾淨 teardown、Ctrl-C 正常)、idle 門檻量級、HOME 隔離裁決均有真機實測記錄或明確 fallback

---

## 任務總覽

| # | 任務 | 檔數 | 類型 | 對應設計輸入 |
|---|---|---|---|---|
| 1 | core idle 常數 + `CliSession::wait_idle` 介面 | 3 | 編碼(port 擴充) | A/C 介面 |
| 2 | pty.rs 實作 wait_idle(輸出靜默追蹤) | 1 | 編碼(機制) | A/C 機制 |
| 3 | pty.rs output 工廠(respawn 續串流,消除 sink 缺口) | 3 | 編碼 | GUI/C |
| 4 | orchestrator 注入前 wait_idle 編排 | 2 | 編碼(編排) | A + C 統一 |
| 5 | 刪除無消費端 Clock port | 5(機械刪) | 重構 | 承接項 5 |
| 6 | 設計輸入 B 裁決 + headless bin 降級 dev-only | 2 | 決策/文件 | B |
| 7 | Tauri 後端 scaffolding(空窗可開) | 6 | 編碼(scaffold) | GUI |
| 8 | 前端 scaffolding(兩 pane + 狀態列骨架) | 4 | 編碼(scaffold) | GUI |
| 9 | GUI composition root(command + poll 迴圈 + emitter) | 1 | 編碼(組裝) | GUI/B |
| 10 | 前端接線(事件→pane/狀態、按鈕/輸入→command) | 1 | 編碼 | GUI |
| 11 | **(human)** HOME 重定位完全隔離真機驗證 | 1 | 調查/記錄 | 承接項 6 |
| 12 | **(human)** GUI 真機端到端 smoke | 0(記錄) | 真機驗證 | A/B/C 全 |

**人工/真裝置驗證不可免(本 agent 無法執行,清楚標記):**
- **Task 6 Step**、**Task 7 Step 6**(開窗確認)、**Task 11**(HOME × OAuth)、**Task 12**(GUI 端到端)——均需真 claude CLI / Telegram / GUI 視窗。
- **只能真機驗證、fake 無法覆蓋的核心項**:`IDLE_QUIET` 門檻值是否真的消除掉字(設計輸入 A),以及 respawn 後 settle 是否一次收斂(設計輸入 C)——fake 只驗「wait_idle 被呼叫且靜默後返回」的機制(Task 2/4),**實際門檻量級必須 Task 12 真機校正回填**(Global Constraints 7)。
- Task 12 若證實 `IDLE_QUIET` 需調整,或發現 Tauri/xterm 版本 API 偏差,各自獨立 commit 並揭露(Global Constraints 9)。
