# Walking Skeleton(Phase 1)Implementation Plan

> **執行後修正(2026-07-17)**:真機驗證發現 CLI 工作目錄嵌在 repo 內會被祖先 CLAUDE.md 污染(角色失效),工作目錄已移至 repo 外 `../clacks-runtime/echo/`——本文件中所有 `runtime/echo` 路徑均以此為準,skeleton 程式碼已同步。另:E2E 首跑發現 `\r` 與 paste 信封同寫不觸發送出,`bracketed_paste` 契約已改為不含 `\r`(caller 延遲後單獨送);telegram adapter 的 expect 前先 `without_url()` 遮 token。實證與裁決見 [skeleton findings](../notes/2026-07-17-skeleton-findings.md)。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 一條 hard-coded echo pipeline 細線:Telegram 訊息 → PTY 注入互動式 claude CLI → Stop hook 寫 outbox → 回覆 Telegram,一次驗證四個高風險整合假設(bracketed paste、Stop hook 觸發時機、`/clear` 行為、sandbox-exec)。

**Architecture:** 獨立 Rust bin crate `skeleton/`(不建 Tauri、不分層)。std threads + mpsc channel,blocking IO。詳見 [architecture.md](../../../architecture.md) 的「實作策略:Walking Skeleton 優先」。

**Tech Stack:** Rust(edition 2024)、reqwest(blocking)、serde/serde_json、portable-pty、notify、bash + jq(hook 腳本)。

## Global Constraints

- **不使用 `claude -p` / Agent SDK**——只跑訂閱制互動式 `claude` CLI(PTY 模式)
- **bot token 只經環境變數 `CLACKS_BOT_TOKEN` 進 Rust**,絕不寫進任何檔案、絕不傳給 CLI
- **骨架期 adapter 保持愚蠢**:只做搬運,無重試、無 timeout 政策(僅一個 hard-coded 等待上限)、無狀態機;`expect`/panic 合法
- **macOS 環境**:BSD 工具(無 `grep -P`、無 `date +%N`),shell 腳本須可在 zsh/bash 跑
- 每個 task 結尾 commit;commit 訊息依 repo 慣例(zh-TW、附 Co-Authored-By)
- 執行本 plan 前需要:`claude` CLI 已登入訂閱帳號、Telegram bot token 已建好(找 @BotFather)、`jq` 已安裝

## File Structure

```
skeleton/                       # 獨立 bin crate(Phase 2 收割成 adapters 後移除)
├── Cargo.toml
└── src/
    ├── main.rs                 # 接線:telegram thread + pty + outbox watcher
    ├── pty_input.rs            # bracketed_paste()(純函式,可單元測試)
    ├── telegram.rs             # TelegramClient(getUpdates / sendMessage)+ next_offset()
    ├── pty.rs                  # spawn_claude():portable-pty 起 CLI + 輸出 drain thread
    └── outbox.rs               # watch_outbox():notify 監看目錄 → mpsc
runtime/echo/                   # 骨架用的 CLI 工作目錄
├── CLAUDE.md                   # echo 角色指示
├── .claude/
│   ├── settings.json           # Stop hook 設定
│   └── hooks/extract-reply.sh  # transcript → outbox JSON
└── outbox/                     # hook 產物(gitignored)
tests/hook/
├── fixture-transcript.jsonl    # 假 transcript
└── test_extract_reply.sh       # hook 腳本測試
docs/superpowers/notes/2026-07-17-skeleton-findings.md   # Task 8/9 的實驗發現
```

---

### Task 1: Crate 腳手架 + bracketed paste 純函式

**Files:**
- Create: `skeleton/Cargo.toml`
- Create: `skeleton/src/main.rs`
- Create: `skeleton/src/pty_input.rs`
- Create: `.gitignore`

**Interfaces:**
- Produces: `pty_input::bracketed_paste(text: &str) -> Vec<u8>`(Task 5、7 使用)

- [ ] **Step 1: 建立 crate 與 .gitignore**

```bash
cargo new skeleton --name clacks-skeleton
```

在 repo 根目錄建 `.gitignore`:

```gitignore
skeleton/target/
runtime/echo/outbox/
```

- [ ] **Step 2: 寫失敗測試**

`skeleton/src/pty_input.rs`:

```rust
pub fn bracketed_paste(text: &str) -> Vec<u8> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_text_in_bracketed_paste_and_appends_carriage_return() {
        let bytes = bracketed_paste("hello\nworld");
        assert_eq!(bytes, b"\x1b[200~hello\nworld\x1b[201~\r");
    }

    #[test]
    fn empty_text_still_produces_envelope_and_return() {
        assert_eq!(bracketed_paste(""), b"\x1b[200~\x1b[201~\r");
    }
}
```

`skeleton/src/main.rs` 暫時:

```rust
mod pty_input;

fn main() {
    println!("clacks skeleton");
}
```

- [ ] **Step 3: 跑測試確認失敗**

Run: `cargo test --manifest-path skeleton/Cargo.toml`
Expected: FAIL(`not yet implemented` panic)

- [ ] **Step 4: 最小實作**

```rust
pub fn bracketed_paste(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() + 13);
    bytes.extend_from_slice(b"\x1b[200~");
    bytes.extend_from_slice(text.as_bytes());
    bytes.extend_from_slice(b"\x1b[201~");
    bytes.push(b'\r');
    bytes
}
```

- [ ] **Step 5: 跑測試確認通過**

Run: `cargo test --manifest-path skeleton/Cargo.toml`
Expected: `test result: ok. 2 passed`

- [ ] **Step 6: Commit**

```bash
git add .gitignore skeleton/
git commit -m "骨架 Task 1:crate 腳手架與 bracketed paste 純函式"
```

---

### Task 2: Telegram echo bot(不經 CLI)

**Files:**
- Create: `skeleton/src/telegram.rs`
- Modify: `skeleton/src/main.rs`
- Modify: `skeleton/Cargo.toml`(加依賴)

**Interfaces:**
- Produces: `telegram::TelegramClient::from_env() -> TelegramClient`、`.get_updates(offset: i64) -> Vec<Update>`、`.send_message(chat_id: i64, text: &str)`、`telegram::next_offset(updates: &[Update], current: i64) -> i64`、`Update { update_id: i64, message: Option<Message> }`、`Message { chat: Chat { id: i64 }, text: Option<String> }`(Task 7 使用)

- [ ] **Step 1: 加依賴**

```bash
cargo add --manifest-path skeleton/Cargo.toml reqwest --no-default-features --features blocking,json,form,query,rustls
cargo add --manifest-path skeleton/Cargo.toml serde --features derive
cargo add --manifest-path skeleton/Cargo.toml serde_json
```

(reqwest 0.13 起 `rustls-tls` 更名為 `rustls`,`.form()`/`.query()` 各自成為 opt-in feature;TLS 憑證由 rustls-platform-verifier 走 macOS 系統信任庫,不需額外 roots feature。)

- [ ] **Step 2: 寫失敗測試(offset 推進是唯一純邏輯)**

`skeleton/src/telegram.rs`:

```rust
use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Update {
    pub update_id: i64,
    pub message: Option<Message>,
}

#[derive(Deserialize, Debug)]
pub struct Message {
    pub chat: Chat,
    pub text: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct Chat {
    pub id: i64,
}

#[derive(Deserialize)]
struct UpdatesResponse {
    result: Vec<Update>,
}

pub fn next_offset(updates: &[Update], current: i64) -> i64 {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
```

- [ ] **Step 3: 跑測試確認失敗**

Run: `cargo test --manifest-path skeleton/Cargo.toml next_offset`
Expected: FAIL(`not yet implemented`)

- [ ] **Step 4: 實作 next_offset 與 TelegramClient**

```rust
pub fn next_offset(updates: &[Update], current: i64) -> i64 {
    updates.iter().map(|u| u.update_id + 1).max().unwrap_or(current)
}

pub struct TelegramClient {
    token: String,
    http: reqwest::blocking::Client,
}

impl TelegramClient {
    pub fn from_env() -> Self {
        let token = std::env::var("CLACKS_BOT_TOKEN").expect("CLACKS_BOT_TOKEN not set");
        let http = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(40))
            .build()
            .expect("build http client");
        Self { token, http }
    }

    pub fn get_updates(&self, offset: i64) -> Vec<Update> {
        let url = format!("https://api.telegram.org/bot{}/getUpdates", self.token);
        let resp: UpdatesResponse = self
            .http
            .get(&url)
            .query(&[("offset", offset.to_string()), ("timeout", "30".to_string())])
            .send()
            .expect("getUpdates request")
            .json()
            .expect("getUpdates parse");
        resp.result
    }

    pub fn send_message(&self, chat_id: i64, text: &str) {
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.token);
        self.http
            .post(&url)
            .form(&[("chat_id", chat_id.to_string()), ("text", text.to_string())])
            .send()
            .expect("sendMessage request");
    }
}
```

- [ ] **Step 5: 跑測試確認通過**

Run: `cargo test --manifest-path skeleton/Cargo.toml`
Expected: `test result: ok. 4 passed`

- [ ] **Step 6: main 改成純 echo loop**

`skeleton/src/main.rs`:

```rust
mod pty_input;
mod telegram;

fn main() {
    let tg = telegram::TelegramClient::from_env();
    let mut offset = 0i64;
    println!("echo bot up");
    loop {
        let updates = tg.get_updates(offset);
        offset = telegram::next_offset(&updates, offset);
        for u in updates {
            if let Some(m) = u.message {
                if let Some(text) = m.text {
                    println!("[echo] chat {} -> {}", m.chat.id, text);
                    tg.send_message(m.chat.id, &text);
                }
            }
        }
    }
}
```

- [ ] **Step 7: 真機驗證**

Run: `CLACKS_BOT_TOKEN=<你的token> cargo run --manifest-path skeleton/Cargo.toml`
用手機發「test123」給 bot。
Expected: console 印出 `[echo] chat <id> -> test123`,手機收到「test123」回覆。Ctrl-C 結束。

- [ ] **Step 8: Commit**

```bash
git add skeleton/
git commit -m "骨架 Task 2:Telegram long-polling echo bot"
```

---

### Task 3: Stop hook 腳本 + fixture 測試

**Files:**
- Create: `runtime/echo/CLAUDE.md`
- Create: `runtime/echo/.claude/settings.json`
- Create: `runtime/echo/.claude/hooks/extract-reply.sh`
- Create: `tests/hook/fixture-transcript.jsonl`
- Create: `tests/hook/test_extract_reply.sh`

**Interfaces:**
- Produces: outbox 檔案契約——`runtime/echo/outbox/<epoch>-<pid>-reply.json`,內容 `{"text": "<最後一則 assistant 回覆全文>"}`(Task 5、6、7 依賴此契約)

- [ ] **Step 1: 建 CLI 工作目錄與角色指示**

`runtime/echo/CLAUDE.md`:

```markdown
# Echo 測試角色

你是管線測試用的 echo bot。收到任何訊息,回覆一律以「ECHO: 」開頭,後接你收到的訊息原文。不要使用任何工具,不要延伸討論,回覆保持一行。
```

- [ ] **Step 2: 寫 hook 腳本**

`runtime/echo/.claude/hooks/extract-reply.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

input=$(cat)
transcript=$(printf '%s' "$input" | jq -r '.transcript_path')

outbox="${CLAUDE_PROJECT_DIR:?}/outbox"
mkdir -p "$outbox"

reply=$(jq -rs '
  [.[] | select(.type == "assistant")]
  | last
  | [.message.content[]? | select(.type == "text") | .text]
  | join("\n")
' "$transcript")

printf '{"text": %s}\n' "$(printf '%s' "$reply" | jq -Rs .)" \
  > "$outbox/$(date +%s)-$$-reply.json"
```

```bash
chmod +x runtime/echo/.claude/hooks/extract-reply.sh
```

`runtime/echo/.claude/settings.json`:

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "\"$CLAUDE_PROJECT_DIR/.claude/hooks/extract-reply.sh\""
          }
        ]
      }
    ]
  }
}
```

- [ ] **Step 3: 寫 fixture 與測試腳本(先跑,確認測的是對的東西)**

`tests/hook/fixture-transcript.jsonl`(兩則 assistant,驗證取的是**最後**一則;中間夾雜其他 type):

```jsonl
{"type":"user","message":{"content":[{"type":"text","text":"hi"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"first reply"}]}}
{"type":"user","message":{"content":[{"type":"text","text":"again"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"ECHO: second reply"}]}}
```

`tests/hook/test_extract_reply.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."

workdir=$(mktemp -d)
export CLAUDE_PROJECT_DIR="$workdir"

printf '{"transcript_path": "%s"}' "$(pwd)/tests/hook/fixture-transcript.jsonl" \
  | runtime/echo/.claude/hooks/extract-reply.sh

outfile=$(ls "$workdir"/outbox/*-reply.json)
actual=$(jq -r '.text' "$outfile")
expected="ECHO: second reply"

if [ "$actual" = "$expected" ]; then
  echo "PASS"
else
  echo "FAIL: expected '$expected', got '$actual'"
  exit 1
fi
```

```bash
chmod +x tests/hook/test_extract_reply.sh
```

- [ ] **Step 4: 跑測試確認通過**

Run: `tests/hook/test_extract_reply.sh`
Expected: `PASS`

- [ ] **Step 5: 手動驗證 hook 在真實 CLI 觸發(這是本 task 的整合驗證核心)**

```bash
cd runtime/echo && claude
```

(不用 `--settings` 旗標——Claude Code 會自動載入 project 的 `.claude/settings.json`;兩者並用會把 Stop hook 註冊兩次,一次回覆產出兩個 outbox 檔。)

首次啟動接受 trust 對話框——**這次授予會被記住,是後續 Task 5/7 全自動注入的前置條件**。在 TUI 輸入「hello」,等回覆出現後:

Run(另一個終端): `ls runtime/echo/outbox/ && jq -r '.text' runtime/echo/outbox/*-reply.json`
Expected: **恰一個** `*-reply.json`,`text` 以 `ECHO: ` 開頭。若出現兩個,檢查 hook 是否被雙重註冊(user settings 與 project settings 各註冊一次之類)。

記錄:若 hook 未觸發或 JSON 結構與 fixture 假設不符,**先修 fixture 使其符合真實 transcript 格式**,再回 Step 4——fixture 必須以真實觀察為準。驗完離開 CLI(`/exit`),清空 `runtime/echo/outbox/`。

- [ ] **Step 6: Commit**

```bash
git add runtime/ tests/
git commit -m "骨架 Task 3:Stop hook 抽取回覆腳本與 fixture 測試"
```

---

### Task 4: PTY 起 CLI + 輸出 drain

**Files:**
- Create: `skeleton/src/pty.rs`
- Modify: `skeleton/src/main.rs`
- Modify: `skeleton/Cargo.toml`

**Interfaces:**
- Consumes: `runtime/echo/` 工作目錄(Task 3)
- Produces: `pty::spawn_claude(workdir: &str) -> CliPty`,`CliPty { writer: Box<dyn Write + Send>, .. }`(Task 5、7 用 `writer` 注入)

- [ ] **Step 1: 加依賴**

```bash
cargo add --manifest-path skeleton/Cargo.toml portable-pty
```

- [ ] **Step 2: 實作 spawn(整合碼,無單元測試——驗證靠 Step 3 手動)**

`skeleton/src/pty.rs`:

```rust
use portable_pty::{native_pty_system, Child, CommandBuilder, PtyPair, PtySize};
use std::io::{Read, Write};

pub struct CliPty {
    pub writer: Box<dyn Write + Send>,
    _pair: PtyPair,
    _child: Box<dyn Child + Send + Sync>,
}

pub fn spawn_claude(workdir: &str) -> CliPty {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize { rows: 40, cols: 120, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");

    let mut cmd = CommandBuilder::new("claude");
    // 安全約束:portable-pty 預設繼承整個父環境,bot token 絕不能進 CLI 子行程
    cmd.env_remove("CLACKS_BOT_TOKEN");
    cmd.cwd(workdir);
    // 不用 --settings:CLI 自動載入 workdir 的 .claude/settings.json,並用會雙重註冊 hook
    let child = pair.slave.spawn_command(cmd).expect("spawn claude");
    // 骨架簡化:pair.slave 未 drop,drain thread 在 child 退出後收不到 EOF(Ctrl-C 結束無害);
    // Phase 2 收割成 adapter 時必須 spawn 後 drop slave

    let mut reader = pair.master.try_clone_reader().expect("clone pty reader");
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut out = std::io::stdout().lock();
                    let _ = out.write_all(&buf[..n]);
                    let _ = out.flush();
                }
            }
        }
    });

    let writer = pair.master.take_writer().expect("take pty writer");
    CliPty { writer, _pair: pair, _child: child }
}
```

`skeleton/src/main.rs` 暫改為只起 PTY:

```rust
mod pty;
mod pty_input;
mod telegram;

fn main() {
    let _cli = pty::spawn_claude("runtime/echo");
    std::thread::sleep(std::time::Duration::from_secs(600));
}
```

- [ ] **Step 3: 手動驗證**

Run(repo 根目錄): `cargo run --manifest-path skeleton/Cargo.toml`
Expected: 終端出現 claude TUI 畫面(banner、輸入框)。Ctrl-C 結束。

若 TUI 沒出現:第一嫌疑是 PTY 子行程未繼承 PATH/HOME(claude 找不到執行檔或 `~/.claude` 憑證)。修法:spawn 前顯式 `cmd.env("PATH", std::env::var("PATH").unwrap());` 與 `cmd.env("HOME", std::env::var("HOME").unwrap());`,並把此發現記入 Task 8 的 findings 文件。

- [ ] **Step 4: Commit**

```bash
git add skeleton/
git commit -m "骨架 Task 4:portable-pty 起 claude 與輸出 drain"
```

---

### Task 5: 注入 hard-coded prompt → 驗證 outbox 產物

**Files:**
- Modify: `skeleton/src/main.rs`

**Interfaces:**
- Consumes: `pty::spawn_claude`、`pty_input::bracketed_paste`、Task 3 的 outbox 契約

**前置條件**:Task 3 Step 5 已在 `runtime/echo` 手動跑過一次 claude 並授予 trust——否則本 task 的自動注入會打進 trust 對話框而不是 prompt。若換了機器或清過 `~/.claude`,先重做該步驟。

- [ ] **Step 1: main 改為「起 CLI → 等 15 秒 → 注入一句話」**

```rust
mod pty;
mod pty_input;
mod telegram;

use std::io::Write;

fn main() {
    std::fs::remove_dir_all("runtime/echo/outbox").ok();
    let mut cli = pty::spawn_claude("runtime/echo");
    std::thread::sleep(std::time::Duration::from_secs(15));

    cli.writer
        .write_all(&pty_input::bracketed_paste("skeleton probe: reply with one line"))
        .expect("pty write");
    cli.writer.flush().expect("pty flush");

    std::thread::sleep(std::time::Duration::from_secs(120));
}
```

- [ ] **Step 2: 執行並驗證**

Run: `cargo run --manifest-path skeleton/Cargo.toml`,等 TUI 顯示回覆後(120 秒內):

Run(另一終端): `jq -r '.text' runtime/echo/outbox/*-reply.json`
Expected: 印出以 `ECHO: ` 開頭、含 `skeleton probe` 的一行——證明 bracketed paste 多字注入 + Stop hook 端到端成立。

若失敗:確認 TUI 畫面上訊息是否被當成一次輸入送出(bracketed paste 假設)、hook 是否觸發(Task 3 Step 5 的觀察)。把偏差記進 Task 8 要建的 findings 文件。

- [ ] **Step 3: Commit**

```bash
git add skeleton/
git commit -m "骨架 Task 5:bracketed paste 注入與 outbox 產物驗證"
```

---

### Task 6: outbox watcher(notify → mpsc)

**Files:**
- Create: `skeleton/src/outbox.rs`
- Modify: `skeleton/src/main.rs`(掛上 module)
- Modify: `skeleton/Cargo.toml`

**Interfaces:**
- Produces: `outbox::watch_outbox(dir: &Path, tx: Sender<PathBuf>) -> RecommendedWatcher`(Task 7 使用;回傳值必須被 caller 持有,drop 即停止監看)

- [ ] **Step 1: 加依賴**

```bash
cargo add --manifest-path skeleton/Cargo.toml notify
cargo add --manifest-path skeleton/Cargo.toml --dev tempfile
```

- [ ] **Step 2: 寫失敗測試**

`skeleton/src/outbox.rs`:

```rust
use notify::{recommended_watcher, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

pub fn watch_outbox(dir: &Path, tx: Sender<PathBuf>) -> RecommendedWatcher {
    todo!()
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
        let _watcher = watch_outbox(dir.path(), tx);

        std::thread::sleep(Duration::from_millis(800)); // macOS FSEvents stream 啟動有延遲,太短會 flaky
        let file = dir.path().join("123-reply.json");
        std::fs::write(&file, r#"{"text":"hi"}"#).unwrap();

        let got = rx.recv_timeout(Duration::from_secs(3)).expect("watcher event");
        assert_eq!(got.file_name(), file.file_name());
    }

    #[test]
    fn ignores_non_json_files() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let _watcher = watch_outbox(dir.path(), tx);

        std::thread::sleep(Duration::from_millis(800)); // macOS FSEvents stream 啟動有延遲,太短會 flaky
        std::fs::write(dir.path().join("junk.tmp"), "x").unwrap();

        assert!(rx.recv_timeout(Duration::from_secs(1)).is_err());
    }
}
```

`skeleton/src/main.rs` 開頭加 `mod outbox;`。

- [ ] **Step 3: 跑測試確認失敗**

Run: `cargo test --manifest-path skeleton/Cargo.toml outbox`
Expected: FAIL(`not yet implemented`)

- [ ] **Step 4: 最小實作**

```rust
pub fn watch_outbox(dir: &Path, tx: Sender<PathBuf>) -> RecommendedWatcher {
    std::fs::create_dir_all(dir).expect("create outbox dir");
    let mut watcher = recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
        if let Ok(event) = res {
            if matches!(event.kind, EventKind::Create(_)) {
                for path in event.paths {
                    if path.extension().is_some_and(|e| e == "json") {
                        let _ = tx.send(path);
                    }
                }
            }
        }
    })
    .expect("create watcher");
    watcher.watch(dir, RecursiveMode::NonRecursive).expect("watch outbox dir");
    watcher
}
```

- [ ] **Step 5: 跑測試確認通過**

Run: `cargo test --manifest-path skeleton/Cargo.toml`
Expected: 全部通過(6 tests)

- [ ] **Step 6: Commit**

```bash
git add skeleton/
git commit -m "骨架 Task 6:notify 監看 outbox 目錄"
```

---

### Task 7: 全管線接線(骨架完成線)

**Files:**
- Modify: `skeleton/src/main.rs`

**Interfaces:**
- Consumes: 前六個 task 的全部 Produces

- [ ] **Step 1: 接線**

`skeleton/src/main.rs`:

```rust
mod outbox;
mod pty;
mod pty_input;
mod telegram;

use std::io::Write;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

fn main() {
    std::fs::remove_dir_all("runtime/echo/outbox").ok();

    let tg = telegram::TelegramClient::from_env();
    let mut cli = pty::spawn_claude("runtime/echo");

    let (outbox_tx, outbox_rx) = mpsc::channel();
    let _watcher = outbox::watch_outbox(Path::new("runtime/echo/outbox"), outbox_tx);

    println!("\n[skeleton] waiting 15s for CLI boot");
    std::thread::sleep(Duration::from_secs(15));

    let (msg_tx, msg_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let poller = telegram::TelegramClient::from_env();
        let mut offset = 0i64;
        loop {
            let updates = poller.get_updates(offset);
            offset = telegram::next_offset(&updates, offset);
            for u in updates {
                if let Some(m) = u.message {
                    if let Some(text) = m.text {
                        let _ = msg_tx.send((m.chat.id, text));
                    }
                }
            }
        }
    });

    for (chat_id, text) in msg_rx {
        println!("\n[skeleton] chat {chat_id} -> inject");
        cli.writer
            .write_all(&pty_input::bracketed_paste(&text))
            .expect("pty write");
        cli.writer.flush().expect("pty flush");

        match outbox_rx.recv_timeout(Duration::from_secs(120)) {
            Ok(path) => {
                std::thread::sleep(Duration::from_millis(800)); // macOS FSEvents stream 啟動有延遲,太短會 flaky
                let raw = std::fs::read_to_string(&path).expect("read outbox file");
                let v: serde_json::Value = serde_json::from_str(&raw).expect("outbox json");
                let reply = v["text"].as_str().unwrap_or("(empty)");
                tg.send_message(chat_id, reply);
            }
            Err(_) => tg.send_message(chat_id, "[skeleton] timeout"),
        }
    }
}
```

(200ms sleep 是「Create 事件早於內容寫完」的骨架級簡化;正式版在 Phase 2+ 用 rename-into-place 解。佇列即 channel 本身:CLI 忙碌時訊息在 `msg_rx` 排隊,主迴圈逐則處理。)

- [ ] **Step 2: 端到端驗證(骨架完成定義)**

Run: `CLACKS_BOT_TOKEN=<token> cargo run --manifest-path skeleton/Cargo.toml`

手機連發兩則:「skeleton one」、「skeleton two」。
Expected:兩則都在 150 秒內收到 `ECHO: ` 開頭的回覆,且順序正確(第二則在第一則處理完才注入)。console 可看到 TUI 全程畫面。

- [ ] **Step 3: Commit**

```bash
git add skeleton/
git commit -m "骨架 Task 7:Telegram→PTY→hook→回覆 全管線接線"
```

---

### Task 8: `/clear` 行為探測(實驗,產出是文件)

**Files:**
- Create: `docs/superpowers/notes/2026-07-17-skeleton-findings.md`

- [ ] **Step 1: 建 findings 文件骨架**

```markdown
# Walking Skeleton 整合發現

> 本文件是 Phase 2 port 語意的實證依據。每項發現寫:操作、觀察、對設計的影響。

## Stop hook 觸發時機

(Task 3/5/7 過程中的觀察補充於此)

## /clear 行為

## outbox 檔案事件語義

(FSEvents 對 create/modify 的合併行為、事件到達 vs 內容寫完的時序——Phase 2 port 語意的依據)

## sandbox-exec

```

- [ ] **Step 2: 實驗**

跑著 Task 7 的骨架,從手機發送 `/clear`(它會被原文注入 CLI)。觀察並記錄:

1. TUI 是否執行了 clear?
2. `runtime/echo/outbox/` 是否出現新檔案(= Stop hook 是否在 /clear 後觸發)?
3. 若觸發,`text` 內容是什麼(空字串?上一輪回覆?)?

Expected(驗證方式):findings 文件「/clear 行為」一節寫下三個問題的實測答案,並附「對 SessionKeeper 設計的影響」一句(例:若 /clear 觸發 hook,Rust 端必須能區分「回覆產物」與「/clear 產物」)。

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/notes/
git commit -m "骨架 Task 8:/clear 行為實測記錄"
```

---

### Task 9: sandbox-exec 探測(實驗,產出是文件 + profile 草稿)

**Files:**
- Create: `runtime/echo/sandbox.sb`
- Modify: `docs/superpowers/notes/2026-07-17-skeleton-findings.md`

**已知設計矛盾,本實驗必須回答**:設計文件寫 taster「無網路」,但互動式 claude CLI 本身必須連 Anthropic API 才能運作。「無網路」需要修正為「僅允許 CLI 自身的 API 連線」或改用其他隔離手段——用實測決定。

- [ ] **Step 1: 寫最小 profile(允許網路,限制檔案寫入)**

`runtime/echo/sandbox.sb`:

```scheme
(version 1)
(allow default)
(deny file-write*)
(allow file-write*
  (subpath (param "WORKDIR"))
  (subpath (param "HOME_CLAUDE"))
  (subpath "/private/tmp")
  (subpath "/private/var/folders"))
```

- [ ] **Step 2: 手動驗證 CLI 在 sandbox 下可運作**

```bash
cd runtime/echo
sandbox-exec -f sandbox.sb \
  -D WORKDIR="$(pwd)" \
  -D HOME_CLAUDE="$HOME/.claude" \
  claude
```

在 TUI 輸入「sandbox probe」。
Expected: CLI 正常回覆,`outbox/` 出現產物。若啟動失敗或功能異常,逐步放寬 profile 並記錄每一條必要權限。

- [ ] **Step 3: 記錄發現**

findings 文件「sandbox-exec」一節寫:可行性結論、最小必要權限清單、以及「無網路」設計假設的修正建議(含是否需要改設計文件的安全模型表)。

Expected(驗證方式):findings 文件該節有實測結論;若設計需修正,節末有明確的「設計文件待改項」清單。

- [ ] **Step 4: Commit**

```bash
git add runtime/echo/sandbox.sb docs/superpowers/notes/
git commit -m "骨架 Task 9:sandbox-exec 可行性實測與 profile 草稿"
```

---

## 骨架完成後(不在本 plan 範圍)

Task 1-9 全過 = Phase 1 完成。下一步是寫 Phase 2 plan(從 findings 提煉 `ports.rs`、骨架碼整理成 adapters),**必須以 `docs/superpowers/notes/2026-07-17-skeleton-findings.md` 的實證為輸入**,不得沿用本 plan 的假設。
