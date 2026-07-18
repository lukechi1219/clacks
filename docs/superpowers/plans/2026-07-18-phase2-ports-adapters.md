# Phase 2:Ports 提煉 + 第一版 Adapters Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 以 walking skeleton 的實證發現(`docs/superpowers/notes/2026-07-17-skeleton-findings.md`)為依據,定義 `ports.rs` 的 trait 語意,並把骨架碼整理成第一版 adapters(只搬運 + 修正實證抓到的缺陷,不加政策邏輯)。

**Architecture:** 新建 `src-tauri/` 純 cargo package(**尚不引入 Tauri 依賴**,GUI 期才加;目錄名先卡住 architecture.md 的位置)。同步阻塞設計(std thread + mpsc),不引入 tokio/async trait。adapters 實作 ports 的 trait;重試、退避、timeout、session 維護等一切**政策**留給 Phase 3 的 core/orchestrator。

**Tech Stack:** Rust(edition 2024)、portable-pty 0.9.0、notify 8.2.0、reqwest 0.13.4(blocking + rustls)、serde/serde_json、bash + jq(hook 腳本)。

## Global Constraints

每項約束都已落到會觸犯它的任務(標註處),不只寫在這裡:

1. **bot token 只經 `CLACKS_BOT_TOKEN` 環境變數進 Rust,絕不寫進任何檔案**。token 絕不進 CLI 子行程環境(→ Task 5 `env_clear` + 白名單 + 測試);token 絕不出現在任何錯誤字串(→ Task 3 `without_url` + 測試)
2. **runtime 工作目錄一律在 repo 目錄樹之外**(`../clacks-runtime/`)——祖先 CLAUDE.md 遍歷跨 git 邊界,嵌套即污染(→ Task 7 路徑、Task 8 驗證)。repo 內 `runtime/` 只是 git-tracked 範本
3. **adapters 保持愚蠢**:只搬運(注入、監看、收發)。不得出現重試迴圈、退避、timeout 政策、「何時 /clear」等決策(smoke bin 是 composition-root 角色,其生存迴圈是骨架護欄豁免的延續,不屬 adapter)
4. **不使用 `claude -p` / Agent SDK**(成本約束),只跑訂閱制互動式 CLI(PTY 模式)
5. **依賴版本 pin 定 = 本 plan 寫的字面值**(承襲 skeleton 實測)。若 `cargo build` 時 API 與 plan 程式碼不符,任何調適都必須在報告揭露(不接受靜默偏差)。plan 撰寫時已驗證:portable-pty 0.9.0 有 `CommandBuilder::env_clear`、notify 8.2.0 有 `Error::io` 與 `ModifyKind::Name`
6. **同步阻塞**:不引入 tokio、async_trait。core/ports 只依賴 std + serde(本 phase ports.rs 連 serde 都不需要——wire 型別留在 adapter)
7. git 紀律照 repo CLAUDE.md:小 commit、`git add` 與 `git commit` 分開呼叫、不 chain `cd`

## 檔案結構(本 phase 完成後)

```
src-tauri/
├── Cargo.toml              # 純 cargo package,無 tauri 依賴
└── src/
    ├── lib.rs              # pub mod ports; pub mod adapters;
    ├── ports.rs            # 4 個 trait + DTO + error 型別(Task 2)
    ├── adapters/
    │   ├── mod.rs
    │   ├── telegram.rs     # TelegramHttp(Task 3)
    │   ├── outbox.rs       # watch_outbox(Task 4)
    │   └── pty.rs          # ClaudePtySession + minimal_env + bracketed_paste(Task 5)
    └── bin/
        └── smoke.rs        # echo 管線走 ports/adapters(Task 7)
runtime/echo/.claude/hooks/extract-reply.sh   # 契約修正(Task 6)
tests/hook/test_extract_reply.sh              # 兩個 case + 清理(Task 6)
tests/hook/fixture-thinking-race.jsonl        # 新 fixture(Task 6)
```

`skeleton/` 原樣保留(歷史證據,仍可獨立編譯)。`MessageStore`/`Clock` 只定義 trait(Task 2),adapter(rusqlite/SystemTime)與 fake 留給 Phase 3——骨架沒有對應碼可搬,現在寫實作就是在猜。

---

### Task 1: src-tauri package 腳手架

**Files:**
- Create: `src-tauri/Cargo.toml`
- Create: `src-tauri/src/lib.rs`

**Interfaces:**
- Produces: 可編譯的空 package `clacks`,後續任務往裡加模組

- [ ] **Step 1: 建立 Cargo.toml**

```toml
[package]
name = "clacks"
version = "0.1.0"
edition = "2024"

# 版本 pin 定承襲 skeleton/Cargo.toml 的實測值(骨架教訓:reqwest 0.12→0.13
# feature 改名)。任何與此字面不符的調整必須在報告揭露
[dependencies]
notify = "8.2.0"
portable-pty = "0.9.0"
reqwest = { version = "0.13.4", default-features = false, features = ["blocking", "form", "json", "query", "rustls"] }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"

[dev-dependencies]
tempfile = "3.27.0"
```

- [ ] **Step 2: 建立 lib.rs**

```rust
//! Clacks:Telegram ↔ 雙 Claude CLI 管線(見 repo 根 architecture.md)。
//! Phase 2:ports + 第一版 adapters(自 walking skeleton 提煉)。
```

- [ ] **Step 3: 驗證編譯**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: `Finished` 且無 warning(首次會下載依賴)

- [ ] **Step 4: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/lib.rs
git commit -m "chore: src-tauri package 腳手架(純 cargo,Tauri 依賴留給 GUI 期)"
```

---

### Task 2: ports.rs — trait 語意定義

**Files:**
- Create: `src-tauri/src/ports.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Produces(後續任務全部依賴,簽名逐字使用):
  - `trait TelegramGateway { poll_updates(&self, offset: i64) -> Result<Vec<Update>, GatewayError>; send_reply(&self, chat_id: i64, text: &str) -> Result<(), GatewayError>; webhook_url(&self) -> Result<Option<String>, GatewayError> }`
  - `trait CliSession { inject_message(&mut self, text: &str) -> Result<(), CliError>; inject_control(&mut self, command: &str) -> Result<(), CliError>; wait_artifact(&mut self, timeout: Duration) -> Result<Artifact, WaitError>; write_raw(&mut self, bytes: &[u8]) -> Result<(), CliError> }`
  - DTO:`Update { update_id: i64, message: Option<IncomingMessage> }`、`IncomingMessage { chat_id: i64, text: Option<String> }`、`Artifact { path: PathBuf, raw: String }`
  - Error:`GatewayError(pub String)`、`CliError(pub String)`、`StoreError(pub String)`、`enum WaitError { Timeout, Disconnected, Io(String) }`

本任務是純型別宣告,無測試——編譯通過即驗證。doc comment 是本任務的主要交付物:每條語意都必須引註實證,**逐字照抄以下內容**,不要自行改寫或省略。

- [ ] **Step 1: 寫 ports.rs**

```rust
//! Port 定義:core/orchestrator 與外界的唯一介面(architecture.md 依賴規則)。
//! 各 trait 語意以 walking skeleton 真機實證為依據:
//! docs/superpowers/notes/2026-07-17-skeleton-findings.md

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

// ---------- Telegram ----------

#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub update_id: i64,
    pub message: Option<IncomingMessage>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IncomingMessage {
    pub chat_id: i64,
    /// None = 非文字訊息(照片/貼圖等)。處理政策(拒收?取 caption?)是
    /// Phase 3 taster 管線的設計項(findings:nexus 對照的非文字盲點),
    /// port 只如實傳遞
    pub text: Option<String>,
}

/// 內含字串保證不含 bot token(建構端必須先遮蔽 URL 再轉字串)。
/// 骨架實證:reqwest 錯誤的 URL 帶 token,panic 印出即洩漏——
/// 已實際發生並被迫輪替 token
#[derive(Debug)]
pub struct GatewayError(pub String);

pub trait TelegramGateway {
    /// long-poll 取 updates。瞬時網路錯誤(本環境 os 53 為系統性現象)回 Err;
    /// 重試/退避政策屬 orchestrator,adapter 不得自行重試
    fn poll_updates(&self, offset: i64) -> Result<Vec<Update>, GatewayError>;

    /// 送出回覆。失敗處理政策同樣屬 orchestrator
    fn send_reply(&self, chat_id: i64, text: &str) -> Result<(), GatewayError>;

    /// 啟動前檢查 webhook 互斥:同 token 掛著 webhook 時 getUpdates 必 409
    /// (骨架實證:Pipedream webhook)。Some(url) = 衝突存在
    fn webhook_url(&self) -> Result<Option<String>, GatewayError>;
}

// ---------- CLI session ----------

/// Stop hook 寫進 outbox 的原始產物。raw 的解析與 schema 驗證是 core 的職責
/// (骨架實證:空文字產物 {"text":""} 會發生——thinking race、模型不遵格式;
/// 判 failed 與否是 core 的決策,port 只搬運)
#[derive(Debug, Clone, PartialEq)]
pub struct Artifact {
    pub path: PathBuf,
    pub raw: String,
}

#[derive(Debug)]
pub struct CliError(pub String);

#[derive(Debug, PartialEq)]
pub enum WaitError {
    Timeout,
    Disconnected,
    Io(String),
}

/// 一個互動式 claude CLI(PTY)。語意全數來自骨架實證:
///
/// - **注入分兩類**:訊息注入期待 outbox 產物;控制指令(/clear、/compact)
///   被 CLI 當 slash 指令執行、**不產生產物**——對它套「等產物否則 failed」
///   會空等到 timeout(骨架 /clear 真機實測)
/// - **inject_message 必須先 drain 殘留產物**:前一則 timeout 後遲到的產物
///   不得誤配給下一則(stale-outbox race,final review 實證)
/// - **實作的 spawn 必須用顯式最小環境**(env_clear + 白名單):
///   portable-pty 預設繼承全父環境,bot token 洩漏即由此發生
/// - **write_raw 是人工介入通道**:CLI 可在任意時點要求 re-login/trust,
///   無 stdin 橋接 = 管線死鎖(E2E 第三跑實測),此通道是必要品而非 nice-to-have
pub trait CliSession {
    fn inject_message(&mut self, text: &str) -> Result<(), CliError>;
    fn inject_control(&mut self, command: &str) -> Result<(), CliError>;
    fn wait_artifact(&mut self, timeout: Duration) -> Result<Artifact, WaitError>;
    /// 原樣寫入 PTY,不加信封、不觸發送出
    fn write_raw(&mut self, bytes: &[u8]) -> Result<(), CliError>;
}

// ---------- Store / Clock ----------

#[derive(Debug)]
pub struct StoreError(pub String);

/// update_id 去重。nexus 對照實證:去重狀態必須落地,重啟不得重收 backlog
/// (骨架只在記憶體,重啟會重收)。rusqlite adapter 留給 Phase 3
pub trait MessageStore {
    /// 第一次見到此 update_id → 記錄並回 true;已見過 → false
    fn first_seen(&mut self, update_id: i64) -> Result<bool, StoreError>;
}

/// 現在時刻。timeout / session 維護決策要可測,時間必須是注入的
pub trait Clock {
    fn now(&self) -> SystemTime;
}
```

- [ ] **Step 2: lib.rs 加入模組宣告**

lib.rs 全文改為:

```rust
//! Clacks:Telegram ↔ 雙 Claude CLI 管線(見 repo 根 architecture.md)。
//! Phase 2:ports + 第一版 adapters(自 walking skeleton 提煉)。

pub mod ports;
```

- [ ] **Step 3: 驗證編譯**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: `Finished`,無 warning(dead_code warning 不會出現——皆為 pub)

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/ports.rs src-tauri/src/lib.rs
git commit -m "feat: ports.rs——4 個 port trait,語意以 skeleton findings 為實證依據"
```

---

### Task 3: adapters/telegram.rs — TelegramGateway 實作

**Files:**
- Create: `src-tauri/src/adapters/mod.rs`
- Create: `src-tauri/src/adapters/telegram.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: Task 2 的 `TelegramGateway`、`Update`、`IncomingMessage`、`GatewayError`
- Produces: `TelegramHttp::from_env() -> TelegramHttp`(impl TelegramGateway)、`pub fn next_offset(updates: &[Update], current: i64) -> i64`

搬運自 `skeleton/src/telegram.rs`,新增兩處:`webhook_url`(findings:409 互斥,啟動前必檢)與 `base_url` 注入(讓 redaction 測試不打真網路)。**token 遮蔽是一級安全需求**:所有 reqwest 錯誤必先 `without_url` 再轉字串,並以測試釘死。

- [ ] **Step 1: 寫失敗測試(先建含測試的完整檔案)**

建立 `src-tauri/src/adapters/mod.rs`:

```rust
pub mod telegram;
```

lib.rs 全文改為:

```rust
//! Clacks:Telegram ↔ 雙 Claude CLI 管線(見 repo 根 architecture.md)。
//! Phase 2:ports + 第一版 adapters(自 walking skeleton 提煉)。

pub mod adapters;
pub mod ports;
```

建立 `src-tauri/src/adapters/telegram.rs`,先只放測試與空殼(讓測試紅):

```rust
use crate::ports::{GatewayError, IncomingMessage, TelegramGateway, Update};
use serde::Deserialize;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::Update;

    fn update(id: i64) -> Update {
        Update { update_id: id, message: None }
    }

    #[test]
    fn advances_past_highest_update_id() {
        assert_eq!(next_offset(&[update(7), update(9), update(8)], 5), 10);
    }

    #[test]
    fn keeps_current_offset_when_no_updates() {
        assert_eq!(next_offset(&[], 5), 5);
    }

    // 一級安全需求(骨架實證:panic 印含 token 的 URL,洩漏實際發生):
    // 三個 API 方法的錯誤都不得含 token。base_url 指向不可達位址,秒級失敗
    #[test]
    fn errors_never_contain_token() {
        let client = TelegramHttp::new(
            "SECRET123TOKEN".to_string(),
            "http://127.0.0.1:9".to_string(),
        );
        let poll_err = client.poll_updates(0).unwrap_err();
        let send_err = client.send_reply(1, "hi").unwrap_err();
        let webhook_err = client.webhook_url().unwrap_err();
        for err in [poll_err, send_err, webhook_err] {
            let shown = format!("{err:?}");
            assert!(!shown.contains("SECRET123TOKEN"), "token leaked: {shown}");
        }
    }
}
```

- [ ] **Step 2: 跑測試確認編譯失敗(紅)**

Run: `cargo test --manifest-path src-tauri/Cargo.toml adapters::telegram`
Expected: 編譯錯誤——`next_offset`、`TelegramHttp` 未定義

- [ ] **Step 3: 寫實作(在同檔測試模組之上補齊)**

`telegram.rs` 的 use 與測試之間插入:

```rust
#[derive(Deserialize)]
struct WireUpdate {
    update_id: i64,
    message: Option<WireMessage>,
}

#[derive(Deserialize)]
struct WireMessage {
    chat: WireChat,
    // 已知盲點(findings:nexus 對照):非文字訊息(帶 caption 的照片等)
    // text 為 None,政策留給 Phase 3 taster 管線設計
    text: Option<String>,
}

#[derive(Deserialize)]
struct WireChat {
    id: i64,
}

#[derive(Deserialize)]
struct UpdatesResponse {
    result: Vec<WireUpdate>,
}

#[derive(Deserialize)]
struct WebhookInfoResponse {
    result: WebhookInfo,
}

#[derive(Deserialize)]
struct WebhookInfo {
    #[serde(default)]
    url: String,
}

/// getUpdates 的下一個 offset(純函式;Phase 3 core 成形時搬移)
pub fn next_offset(updates: &[Update], current: i64) -> i64 {
    updates.iter().map(|u| u.update_id + 1).max().unwrap_or(current)
}

pub struct TelegramHttp {
    token: String,
    base_url: String,
    http: reqwest::blocking::Client,
}

impl TelegramHttp {
    pub fn from_env() -> Self {
        let token = std::env::var("CLACKS_BOT_TOKEN").expect("CLACKS_BOT_TOKEN not set");
        Self::new(token, "https://api.telegram.org".to_string())
    }

    /// base_url 可注入:redaction 測試用不可達位址,不打真實 API
    fn new(token: String, base_url: String) -> Self {
        let http = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(40))
            .build()
            .expect("build http client");
        Self { token, base_url, http }
    }

    fn url(&self, method: &str) -> String {
        format!("{}/bot{}/{}", self.base_url, self.token, method)
    }

    /// 一級安全需求(骨架實證:panic 印 URL 洩漏 token,已實際發生並輪替):
    /// 所有 reqwest 錯誤先 without_url 再轉字串,token 不得進任何錯誤訊息
    fn redact(error: reqwest::Error) -> GatewayError {
        GatewayError(error.without_url().to_string())
    }
}

impl TelegramGateway for TelegramHttp {
    fn poll_updates(&self, offset: i64) -> Result<Vec<Update>, GatewayError> {
        let resp: UpdatesResponse = self
            .http
            .get(self.url("getUpdates"))
            .query(&[("offset", offset.to_string()), ("timeout", "30".to_string())])
            .send()
            .map_err(Self::redact)?
            .json()
            .map_err(Self::redact)?;
        Ok(resp
            .result
            .into_iter()
            .map(|u| Update {
                update_id: u.update_id,
                message: u.message.map(|m| IncomingMessage {
                    chat_id: m.chat.id,
                    text: m.text,
                }),
            })
            .collect())
    }

    fn send_reply(&self, chat_id: i64, text: &str) -> Result<(), GatewayError> {
        self.http
            .post(self.url("sendMessage"))
            .form(&[("chat_id", chat_id.to_string()), ("text", text.to_string())])
            .send()
            .map_err(Self::redact)?;
        Ok(())
    }

    fn webhook_url(&self) -> Result<Option<String>, GatewayError> {
        let resp: WebhookInfoResponse = self
            .http
            .get(self.url("getWebhookInfo"))
            .send()
            .map_err(Self::redact)?
            .json()
            .map_err(Self::redact)?;
        Ok(if resp.result.url.is_empty() {
            None
        } else {
            Some(resp.result.url)
        })
    }
}
```

- [ ] **Step 4: 跑測試確認全綠**

Run: `cargo test --manifest-path src-tauri/Cargo.toml adapters::telegram`
Expected: `3 passed`(兩個 next_offset + errors_never_contain_token)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/adapters/mod.rs src-tauri/src/adapters/telegram.rs src-tauri/src/lib.rs
git commit -m "feat: telegram adapter——搬運骨架 + webhook 互斥檢查 + token 遮蔽測試釘死"
```

---

### Task 4: adapters/outbox.rs — rename-into-place 事件語意

**Files:**
- Create: `src-tauri/src/adapters/outbox.rs`
- Modify: `src-tauri/src/adapters/mod.rs`

**Interfaces:**
- Produces: `pub fn watch_outbox(dir: &Path, tx: Sender<PathBuf>) -> Result<RecommendedWatcher, notify::Error>`(Task 5 的 `ClaudePtySession::spawn` 使用)

搬運自 `skeleton/src/outbox.rs`,兩處修正:(1) hook 契約改為 rename-into-place(Task 6),macOS FSEvents 對 mv 產生的是 rename 類事件而非 Create,watcher 必須同時接受兩類;(2) 回傳 `Result` 取代 `expect`(caller 決定錯誤處理)。

**⚠️ 事件種類是經驗假設**:`emits_path_when_json_renamed_into_place` 這個測試就是在本機釘死 FSEvents 實際行為。若它失敗,代表假設錯誤——**必須回報並記錄實際收到的 EventKind,不得調整斷言蒙混過關**。

- [ ] **Step 1: 寫完整檔案(實作 + 三個測試)**

`src-tauri/src/adapters/outbox.rs`:

```rust
use notify::event::ModifyKind;
use notify::{recommended_watcher, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

/// 監看 outbox 目錄,新 .json 檔路徑送進 channel。
/// hook 契約是 rename-into-place(.partial 寫完後 mv 成 .json),
/// 故事件到達即內容完整;macOS FSEvents 對 mv 產生 rename 類事件
/// 而非 Create,兩類都要接。`.partial` 檔靠副檔名過濾排除。
/// 同一產物可能觸發多個事件(FSEvents flag 合併)——重複路徑由
/// CliSession 的 drain-before-inject 語意吸收,此處不去重。
pub fn watch_outbox(
    dir: &Path,
    tx: Sender<PathBuf>,
) -> Result<RecommendedWatcher, notify::Error> {
    std::fs::create_dir_all(dir).map_err(notify::Error::io)?;
    let mut watcher = recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
        if let Ok(event) = res {
            let relevant = matches!(
                event.kind,
                EventKind::Create(_) | EventKind::Modify(ModifyKind::Name(_))
            );
            if relevant {
                for path in event.paths {
                    if path.extension().is_some_and(|e| e == "json") && path.exists() {
                        let _ = tx.send(path);
                    }
                }
            }
        }
    })?;
    watcher.watch(dir, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn emits_path_when_json_file_created() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let _watcher = watch_outbox(dir.path(), tx).unwrap();

        std::thread::sleep(Duration::from_millis(800)); // macOS FSEvents stream 啟動有延遲,太短會 flaky
        let file = dir.path().join("123-reply.json");
        std::fs::write(&file, r#"{"text":"hi"}"#).unwrap();

        let got = rx.recv_timeout(Duration::from_secs(3)).expect("watcher event");
        assert_eq!(got.file_name(), file.file_name());
    }

    // 釘死 rename-into-place 的事件語意(hook 契約,Task 6):
    // 此測試失敗 = FSEvents 事件種類假設錯誤,必須回報實際 EventKind,不得改斷言
    #[test]
    fn emits_path_when_json_renamed_into_place() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let _watcher = watch_outbox(dir.path(), tx).unwrap();

        std::thread::sleep(Duration::from_millis(800)); // macOS FSEvents stream 啟動有延遲,太短會 flaky
        let tmp = dir.path().join("123-reply.json.partial");
        std::fs::write(&tmp, r#"{"text":"hi"}"#).unwrap();
        let done = dir.path().join("123-reply.json");
        std::fs::rename(&tmp, &done).unwrap();

        let got = rx.recv_timeout(Duration::from_secs(3)).expect("watcher event");
        assert_eq!(got.file_name(), done.file_name());
    }

    #[test]
    fn ignores_non_json_files() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let _watcher = watch_outbox(dir.path(), tx).unwrap();

        std::thread::sleep(Duration::from_millis(800)); // macOS FSEvents stream 啟動有延遲,太短會 flaky
        std::fs::write(dir.path().join("junk.tmp"), "x").unwrap();

        assert!(rx.recv_timeout(Duration::from_secs(1)).is_err());
    }
}
```

`src-tauri/src/adapters/mod.rs` 全文改為:

```rust
pub mod outbox;
pub mod telegram;
```

- [ ] **Step 2: 跑測試**

Run: `cargo test --manifest-path src-tauri/Cargo.toml adapters::outbox`
Expected: `3 passed`。若 `emits_path_when_json_renamed_into_place` 失敗 → 依上方 ⚠️ 回報,狀態 BLOCKED

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/adapters/outbox.rs src-tauri/src/adapters/mod.rs
git commit -m "feat: outbox adapter——rename-into-place 事件語意(Create + Rename 雙接)"
```

---

### Task 5: adapters/pty.rs — ClaudePtySession(CliSession 實作)

**Files:**
- Create: `src-tauri/src/adapters/pty.rs`
- Modify: `src-tauri/src/adapters/mod.rs`

**Interfaces:**
- Consumes: Task 2 的 `CliSession`、`Artifact`、`CliError`、`WaitError`;Task 4 的 `watch_outbox`
- Produces: `ClaudePtySession::spawn(workdir: &Path, output: Box<dyn Write + Send>) -> Result<ClaudePtySession, CliError>`(impl CliSession)、`pub fn minimal_env(...)`、`pub fn bracketed_paste(text: &str) -> Vec<u8>`

搬運自 `skeleton/src/pty.rs` + `skeleton/src/pty_input.rs`,依骨架教訓修正三處:

1. **顯式最小環境**(本 plan 一級安全約束的落點):`env_clear()` 後只放行白名單。骨架的 `env_remove("CLACKS_BOT_TOKEN")` 是逐項排除——漏列新 secret 即洩漏;白名單結構性排除所有非必要變數。以 `minimal_env` 純函式 + 單元測試釘死
2. **spawn 後 drop slave**(骨架註記的已知缺陷):否則 child 退出後 drain thread 收不到 EOF
3. **wait_artifact 不再 sleep 200ms**:hook 契約 rename-into-place(Task 6)保證事件到達即內容完整

- [ ] **Step 1: 寫失敗測試(先建檔,只放純函式測試與 use)**

`src-tauri/src/adapters/pty.rs`:

```rust
use crate::ports::{Artifact, CliError, CliSession, WaitError};
use notify::RecommendedWatcher;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use super::outbox::watch_outbox;

#[cfg(test)]
mod tests {
    use super::*;

    // 一級安全需求(骨架實證:portable-pty 預設繼承全父環境,token 直入子行程)
    #[test]
    fn minimal_env_excludes_secrets_and_unknowns() {
        let parent = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("CLACKS_BOT_TOKEN".to_string(), "SECRET".to_string()),
            ("HOME".to_string(), "/Users/x".to_string()),
            ("RANDOM_VAR".to_string(), "y".to_string()),
        ];
        let env = minimal_env(parent.into_iter());
        assert!(env.iter().any(|(k, _)| k == "PATH"));
        assert!(env.iter().any(|(k, _)| k == "HOME"));
        assert!(!env.iter().any(|(k, _)| k == "CLACKS_BOT_TOKEN"));
        assert!(!env.iter().any(|(k, _)| k == "RANDOM_VAR"));
    }

    // 真機實證(E2E):\r 緊跟 201~ 同次寫入不觸發 TUI 送出——
    // 信封不含 \r,\r 由 write_submit 延遲後單獨寫
    #[test]
    fn wraps_text_in_bracketed_paste_envelope() {
        let bytes = bracketed_paste("hello\nworld");
        assert_eq!(bytes, b"\x1b[200~hello\nworld\x1b[201~");
    }

    #[test]
    fn empty_text_still_produces_envelope() {
        assert_eq!(bracketed_paste(""), b"\x1b[200~\x1b[201~");
    }
}
```

`src-tauri/src/adapters/mod.rs` 全文改為:

```rust
pub mod outbox;
pub mod pty;
pub mod telegram;
```

- [ ] **Step 2: 跑測試確認編譯失敗(紅)**

Run: `cargo test --manifest-path src-tauri/Cargo.toml adapters::pty`
Expected: 編譯錯誤——`minimal_env`、`bracketed_paste` 未定義

- [ ] **Step 3: 寫實作(use 與測試模組之間插入)**

```rust
/// claude CLI 啟動所需的最小環境白名單。
/// 一級安全需求(骨架實證):CommandBuilder::new 以父行程全環境為基底,
/// CLACKS_BOT_TOKEN 不排除就直接進 CLI 子行程。白名單以外一律不繼承——
/// secret 排除靠結構(env_clear + allowlist),不靠逐項 env_remove
/// (漏列新 secret 即洩漏)。
/// HOME:CLI 讀 ~/.claude(OAuth、設定);PATH:找得到 claude;
/// TERM:TUI 渲染;其餘為 shell 與 locale 慣例最小集
const ENV_ALLOWLIST: &[&str] = &[
    "PATH", "HOME", "TERM", "LANG", "LC_ALL", "USER", "SHELL", "TMPDIR",
];

pub fn minimal_env(
    parent: impl Iterator<Item = (String, String)>,
) -> Vec<(String, String)> {
    parent
        .filter(|(key, _)| ENV_ALLOWLIST.contains(&key.as_str()))
        .collect()
}

/// 真機實證(E2E):\r 緊跟 201~ 同次寫入不觸發 TUI 送出——
/// 本函式只產生 paste 信封,\r 由 caller 延遲後單獨寫
pub fn bracketed_paste(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() + 12);
    bytes.extend_from_slice(b"\x1b[200~");
    bytes.extend_from_slice(text.as_bytes());
    bytes.extend_from_slice(b"\x1b[201~");
    bytes
}

pub struct ClaudePtySession {
    writer: Box<dyn Write + Send>,
    artifact_rx: Receiver<PathBuf>,
    _watcher: RecommendedWatcher,
    _master: Box<dyn MasterPty + Send>,
    _child: Box<dyn Child + Send + Sync>,
}

impl ClaudePtySession {
    /// output:PTY 輸出的去向(骨架/smoke 用 stdout;GUI 期換成事件流)。
    /// workdir 必須在 repo 目錄樹外(祖先 CLAUDE.md 污染,骨架實證)——
    /// 呼叫端(composition root)負責給對路徑
    pub fn spawn(
        workdir: &Path,
        mut output: Box<dyn Write + Send>,
    ) -> Result<Self, CliError> {
        let pty_system = native_pty_system();
        let portable_pty::PtyPair { slave, master } = pty_system
            .openpty(PtySize { rows: 40, cols: 120, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| CliError(e.to_string()))?;

        let mut cmd = CommandBuilder::new("claude");
        // 一級安全約束落點:清空繼承環境,只放行白名單(token 結構性排除)
        cmd.env_clear();
        for (key, value) in minimal_env(std::env::vars()) {
            cmd.env(key, value);
        }
        cmd.cwd(workdir);
        // 不用 --settings:CLI 自動載入 workdir 的 .claude/settings.json,並用會雙重註冊 hook
        let child = slave
            .spawn_command(cmd)
            .map_err(|e| CliError(e.to_string()))?;
        drop(slave); // 骨架教訓:slave 不 drop,child 退出後 drain thread 收不到 EOF

        let mut reader = master
            .try_clone_reader()
            .map_err(|e| CliError(e.to_string()))?;
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let _ = output.write_all(&buf[..n]);
                        let _ = output.flush();
                    }
                }
            }
        });

        let (artifact_tx, artifact_rx) = mpsc::channel();
        let watcher = watch_outbox(&workdir.join("outbox"), artifact_tx)
            .map_err(|e| CliError(e.to_string()))?;

        let writer = master
            .take_writer()
            .map_err(|e| CliError(e.to_string()))?;
        Ok(Self {
            writer,
            artifact_rx,
            _watcher: watcher,
            _master: master,
            _child: child,
        })
    }

    fn write_submit(&mut self, text: &str) -> Result<(), CliError> {
        self.write_raw(&bracketed_paste(text))?;
        // 真機實證:paste 信封與 \r 同寫不送出,需延遲後單獨送
        std::thread::sleep(Duration::from_millis(150));
        self.write_raw(b"\r")
    }
}

impl CliSession for ClaudePtySession {
    fn inject_message(&mut self, text: &str) -> Result<(), CliError> {
        // stale 產物 drain:前一則 timeout 後遲到的產物不得誤配給本則(port 語意)
        while self.artifact_rx.try_recv().is_ok() {}
        self.write_submit(text)
    }

    fn inject_control(&mut self, command: &str) -> Result<(), CliError> {
        // 控制指令不產生 outbox 產物(port 語意):不 drain、caller 不得 wait_artifact
        self.write_submit(command)
    }

    fn wait_artifact(&mut self, timeout: Duration) -> Result<Artifact, WaitError> {
        let path = self.artifact_rx.recv_timeout(timeout).map_err(|e| match e {
            RecvTimeoutError::Timeout => WaitError::Timeout,
            RecvTimeoutError::Disconnected => WaitError::Disconnected,
        })?;
        // hook 契約 rename-into-place:事件到達即內容完整(骨架的 200ms sleep 移除)
        let raw = std::fs::read_to_string(&path).map_err(|e| WaitError::Io(e.to_string()))?;
        Ok(Artifact { path, raw })
    }

    fn write_raw(&mut self, bytes: &[u8]) -> Result<(), CliError> {
        self.writer
            .write_all(bytes)
            .and_then(|()| self.writer.flush())
            .map_err(|e| CliError(e.to_string()))
    }
}
```

- [ ] **Step 4: 跑測試確認全綠 + 全套測試**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: telegram 3 + outbox 3 + pty 3 = `9 passed`(spawn 的真機驗證在 Task 8)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/adapters/pty.rs src-tauri/src/adapters/mod.rs
git commit -m "feat: pty adapter——CliSession 實作;env_clear+白名單、drop slave、drain 語意"
```

---

### Task 6: Hook 腳本契約修正(thinking-race + rename-into-place)

**Files:**
- Modify: `runtime/echo/.claude/hooks/extract-reply.sh`
- Modify: `tests/hook/test_extract_reply.sh`
- Create: `tests/hook/fixture-thinking-race.jsonl`

**Interfaces:**
- Produces: outbox 產物契約——`.json` 檔以 rename-into-place 出現(先寫 `<名>.json.partial` 再 `mv`),Task 4 的 watcher 與 Task 5 的「事件到達即完整」依賴此契約

兩個骨架實證的缺陷修正:

1. **thinking-race**(Task 9 實測):Stop hook 可能在最終 text 區塊 flush 前執行,「最後一個 assistant entry」當下只有 thinking → 抽出空字串。改取「最後一個**含 text 區塊**的 assistant entry」
2. **rename-into-place**(final review):`>` redirect 直寫讓 Create 事件可能早於內容寫完;先寫 `.partial` 再 `mv`(同目錄 rename 為原子操作),順帶消除同秒檔名的部分寫入風險

並修 Task 3 遺留的兩個 Minor:mktemp 目錄無清理、`set -e` 下無產物時死於 `ls` 而非走 FAIL 分支。

- [ ] **Step 1: 建立 thinking-race fixture**

`tests/hook/fixture-thinking-race.jsonl`(最後一個 assistant entry 只有 thinking——模擬 race 當下的 transcript 狀態):

```jsonl
{"type":"user","message":{"content":[{"type":"text","text":"hi"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"ECHO: real reply"}]}}
{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"pondering..."}]}}
```

- [ ] **Step 2: 改寫測試腳本(先跑,確認新 case 對舊 hook 是紅的)**

`tests/hook/test_extract_reply.sh` 全文改為:

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."

fail=0

check() {
  local fixture="$1" expected="$2" label="$3"
  local workdir outfile actual
  workdir=$(mktemp -d)

  printf '{"transcript_path": "%s"}' "$(pwd)/$fixture" \
    | CLAUDE_PROJECT_DIR="$workdir" runtime/echo/.claude/hooks/extract-reply.sh

  outfile=$(find "$workdir/outbox" -name '*-reply.json' 2>/dev/null | head -1)
  if [ -z "$outfile" ]; then
    echo "FAIL($label): no outbox artifact"
    fail=1
  else
    actual=$(jq -r '.text' "$outfile")
    if [ "$actual" = "$expected" ]; then
      echo "PASS($label)"
    else
      echo "FAIL($label): expected '$expected', got '$actual'"
      fail=1
    fi
  fi

  if find "$workdir/outbox" -name '*.partial' 2>/dev/null | grep -q .; then
    echo "FAIL($label): leftover .partial file"
    fail=1
  fi

  rm -rf "$workdir"
}

check tests/hook/fixture-transcript.jsonl "ECHO: second reply" "basic"
check tests/hook/fixture-thinking-race.jsonl "ECHO: real reply" "thinking-race"

exit "$fail"
```

Run: `bash tests/hook/test_extract_reply.sh`
Expected: `PASS(basic)` + `FAIL(thinking-race): expected 'ECHO: real reply', got ''`——舊 hook 對 race fixture 抽出空字串,紅燈確認測試有效

- [ ] **Step 3: 改寫 hook 腳本**

`runtime/echo/.claude/hooks/extract-reply.sh` 全文改為:

```bash
#!/usr/bin/env bash
set -euo pipefail

input=$(cat)
transcript=$(printf '%s' "$input" | jq -r '.transcript_path')

outbox="${CLAUDE_PROJECT_DIR:?}/outbox"
mkdir -p "$outbox"

# thinking-race 實證(skeleton findings, Task 9):最後一個 assistant entry
# 可能只有 thinking 區塊(text 尚未 flush)——取「最後一個含 text 區塊的
# entry」而非單純 last;全都沒有 text 時輸出空字串,由 Rust 端 schema 驗證判 failed
reply=$(jq -rs '
  [.[] | select(.type == "assistant")
       | [.message.content[]? | select(.type == "text") | .text]
       | select(length > 0)]
  | last // []
  | join("\n")
' "$transcript")

# rename-into-place(skeleton findings, final review):.partial 寫完再 mv
# 成 .json(同目錄 rename 原子),watcher 收到事件時內容保證完整;
# .partial 不以 .json 結尾,不會被 watcher 的副檔名過濾撿走
final="$outbox/$(date +%s)-$$-reply.json"
tmp="$final.partial"
printf '{"text": %s}\n' "$(printf '%s' "$reply" | jq -Rs .)" > "$tmp"
mv "$tmp" "$final"
```

- [ ] **Step 4: 跑測試確認全綠**

Run: `bash tests/hook/test_extract_reply.sh`
Expected: `PASS(basic)`、`PASS(thinking-race)`,exit code 0(`echo $?` 確認)

- [ ] **Step 5: 同步到 repo 外的 live runtime**

```bash
cp runtime/echo/.claude/hooks/extract-reply.sh ../clacks-runtime/echo/.claude/hooks/extract-reply.sh
diff runtime/echo/.claude/hooks/extract-reply.sh ../clacks-runtime/echo/.claude/hooks/extract-reply.sh
```

Expected: diff 無輸出(兩份一致)。live runtime 在 repo 外,不入 git——這步是部署,不是版控

- [ ] **Step 6: Commit**

```bash
git add runtime/echo/.claude/hooks/extract-reply.sh tests/hook/test_extract_reply.sh tests/hook/fixture-thinking-race.jsonl
git commit -m "fix: hook 契約——thinking-race 取最後含 text 的 entry;rename-into-place 原子寫入"
```

---

### Task 7: smoke bin — echo 管線改走 ports/adapters

**Files:**
- Create: `src-tauri/src/bin/smoke.rs`

**Interfaces:**
- Consumes: Task 3 `TelegramHttp`/`next_offset`、Task 5 `ClaudePtySession`、Task 2 的兩個 trait

與 `skeleton/src/main.rs` 同款 echo 管線,但全程經 ports/adapters——證明 adapter 忠實搬運骨架行為。smoke bin 扮演 composition root:poller 生存迴圈(骨架護欄豁免的延續)與「`/` 開頭 = 控制指令」的分流邏輯放這裡,**不放進 adapter**。真機執行在 Task 8(需真人)。

- [ ] **Step 1: 寫 smoke.rs**

```rust
//! Phase 2 smoke:與骨架同款 echo 管線,但全程走 ports + adapters。
//! 驗證 adapter 忠實搬運骨架行為(尤其 env_clear 最小環境下 claude 仍可啟動)。
//! 從 repo root 執行(../clacks-runtime 相對路徑才會對):
//!   CLACKS_BOT_TOKEN=$(security find-generic-password -s clacks-bot -w) \
//!     cargo run --manifest-path src-tauri/Cargo.toml --bin smoke

use clacks::adapters::pty::ClaudePtySession;
use clacks::adapters::telegram::{next_offset, TelegramHttp};
use clacks::ports::{CliSession, TelegramGateway};
use std::sync::mpsc;
use std::time::Duration;

fn main() {
    // 工作目錄在 repo 外:嵌套在 repo 內會被祖先 CLAUDE.md 污染角色(骨架實證)
    let runtime = std::path::Path::new("../clacks-runtime/echo");
    std::fs::remove_dir_all(runtime.join("outbox")).ok();

    let tg = TelegramHttp::from_env();
    // webhook 互斥檢查(骨架實證:掛著 webhook 時 getUpdates 必 409)
    match tg.webhook_url() {
        Ok(None) => {}
        Ok(Some(url)) => {
            eprintln!("[smoke] webhook active ({url}) — getUpdates 會 409,先解除 webhook");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("[smoke] webhook check failed: {e:?}");
            std::process::exit(1);
        }
    }

    let mut cli = ClaudePtySession::spawn(runtime, Box::new(std::io::stdout()))
        .expect("spawn claude");

    println!("\n[smoke] waiting 15s for CLI boot");
    std::thread::sleep(Duration::from_secs(15));

    let (msg_tx, msg_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let poller = TelegramHttp::from_env();
        let mut offset = 0i64;
        loop {
            // 最小生存迴圈(骨架護欄豁免沿用):os 53 為本環境系統性現象,
            // 固定 3s 重拉、無退避——政策仍留給 Phase 3 orchestrator
            let updates = match poller.poll_updates(offset) {
                Ok(updates) => updates,
                Err(e) => {
                    println!("\n[smoke] poll error, retry in 3s: {e:?}");
                    std::thread::sleep(Duration::from_secs(3));
                    continue;
                }
            };
            offset = next_offset(&updates, offset);
            for update in updates {
                if let Some(message) = update.message {
                    if let Some(text) = message.text {
                        let _ = msg_tx.send((message.chat_id, text));
                    }
                }
            }
        }
    });

    for (chat_id, text) in msg_rx {
        if text.starts_with('/') {
            // 控制指令分流(port 語意驗證點):不等產物——骨架在這裡會空等 120s
            println!("\n[smoke] chat {chat_id} -> control {text}");
            cli.inject_control(&text).expect("inject control");
            let _ = tg.send_reply(chat_id, &format!("[smoke] control injected: {text}"));
            continue;
        }
        println!("\n[smoke] chat {chat_id} -> inject");
        cli.inject_message(&text).expect("inject message");
        match cli.wait_artifact(Duration::from_secs(120)) {
            Ok(artifact) => {
                let value: serde_json::Value =
                    serde_json::from_str(&artifact.raw).expect("artifact json");
                let reply = value["text"].as_str().unwrap_or("(empty)");
                let _ = tg.send_reply(chat_id, reply);
            }
            Err(e) => {
                let _ = tg.send_reply(chat_id, &format!("[smoke] no artifact: {e:?}"));
            }
        }
    }
}
```

- [ ] **Step 2: 驗證編譯 + 全套測試**

Run: `cargo build --manifest-path src-tauri/Cargo.toml --bin smoke && cargo test --manifest-path src-tauri/Cargo.toml`
Expected: build `Finished` 無 warning;`9 passed`

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/bin/smoke.rs
git commit -m "feat: smoke bin——echo 管線改走 ports/adapters(真機驗證留 Task 8)"
```

---

### Task 8: 真機 smoke 驗證(PENDING-HUMAN)

**Files:**
- Modify: `docs/superpowers/notes/2026-07-17-skeleton-findings.md`(追記 Phase 2 驗證結果)

真人操作。逐項核對,結果(含失敗)寫回 findings:

- [ ] **Step 1: 啟動**

```bash
CLACKS_BOT_TOKEN=$(security find-generic-password -s clacks-bot -w) \
  cargo run --manifest-path src-tauri/Cargo.toml --bin smoke
```

從 repo root 執行。Expected: webhook 檢查通過、claude TUI 正常開機(**這是 env_clear 最小環境的真機驗證點**——若 claude 啟動失敗或要求重新 login/trust,代表白名單缺變數:記下畫面症狀回報,由 controller 判斷補哪個變數,不要自行亂加)

- [ ] **Step 2: 訊息注入 ×2(順序)**

Telegram 對 @ChatSummary_37927_bot 連續發「phase two ports」、「phase two adapters」。
Expected: 依序收到 `ECHO: phase two ports`、`ECHO: phase two adapters`

- [ ] **Step 3: 控制指令分流**

發 `/clear`。
Expected: **立即**收到 `[smoke] control injected: /clear`(不是等 120 秒後的 timeout 訊息——這就是 inject_control 語意修正的驗證);TUI 畫面確認 /clear 有執行

- [ ] **Step 4: /clear 後管線仍活**

再發「after clear」。Expected: 收到 `ECHO: after clear`

- [ ] **Step 5: 產物衛生**

```bash
ls ../clacks-runtime/echo/outbox/
```

Expected: 只有 `*-reply.json`,無 `.partial` 殘留

- [ ] **Step 6: token 衛生**

檢查整段終端輸出(含 poll error 訊息,若有)。Expected: 任何輸出都不含 bot token

- [ ] **Step 7: 結果寫回 findings 並 commit**

在 findings 文件末尾新增「## Phase 2 smoke(真機)」段落記錄各項結果;commit:

```bash
git add docs/superpowers/notes/2026-07-17-skeleton-findings.md
git commit -m "findings: Phase 2 smoke 真機驗證結果"
```

---

## 驗收總表

| 驗證 | 指令 | 通過標準 |
|---|---|---|
| Rust 單元測試 | `cargo test --manifest-path src-tauri/Cargo.toml` | 9 passed(telegram 3、outbox 3、pty 3) |
| token 遮蔽 | 同上(`errors_never_contain_token`) | 錯誤字串無 `SECRET123TOKEN` |
| token 不進子行程 | 同上(`minimal_env_excludes_secrets_and_unknowns`) | 白名單外全排除 |
| hook 契約 | `bash tests/hook/test_extract_reply.sh` | PASS×2,exit 0,無 .partial 殘留 |
| rename 事件語意 | `cargo test ... adapters::outbox` | rename-into-place 測試綠(失敗=假設錯,回報) |
| 真機 E2E | Task 8 checklist | 全項通過,結果落 findings |
