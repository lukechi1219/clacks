# Phase 6 — Stop Hook 讀檔競態 + 人工輸入延遲 修法 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修掉 Phase 5 Task 12 真機測試記錄的兩項已知限制——Stop hook 讀檔競態(內容誤送/漏送)與人工輸入 30 秒延遲。

**Architecture:** 兩項修法互相獨立、各自可單獨驗證。(1)`extract-reply.sh` 加入以 transcript entry `uuid` 為新鮮度依據的重試迴圈,狀態記在 outbox 目錄下的隱藏檔 `.last-uuid`(不含 `.json` 副檔名,不會被 `watch_outbox` 撿到)。(2)`telegram.rs` 的 `getUpdates` 長輪詢逾時參數從 30 秒改成 5 秒。

**Tech Stack:** Rust(既有 `reqwest::blocking` + 標準庫 `TcpListener` 測試替身)、Bash + `jq`(既有 hook 腳本測試慣例,`tests/hook/test_extract_reply.sh`)。

## Global Constraints

- 兩項修法(hook 重試迴圈、getUpdates 逾時)彼此獨立,可分別完成、分別 commit,不互相依賴。
- `templates/echo/`、`templates/taster/`、`templates/cyrano/` 三份 `extract-reply.sh` 內容必須維持逐位元組相同(現況即是如此,修法不得破壞這個不變量)。
- 新增的狀態檔案 `$outbox/.last-uuid` **不得**以 `.json` 結尾——`adapters/outbox.rs:25` 的 watcher 過濾條件僅檢查副檔名是否為 `json`,若誤用 `.json` 結尾的檔名會被誤判為回覆檔案。
- 不修改 `wait_idle`、`IDLE_QUIET`、`IDLE_SETTLE_TIMEOUT`(`src-tauri/src/core/session.rs`)——本次兩項修法與其正交。
- HTTP client 的 40 秒逾時(`telegram.rs:59`)不需要跟著調整,只需大於 long-poll 的 `timeout` 參數即可,5 秒 < 40 秒,現有緩衝依然充足。
- 既有 `tests/hook/test_extract_reply.sh` 的既有兩個 fixture(`fixture-transcript.jsonl`、`fixture-thinking-race.jsonl`,皆無 `uuid` 欄位)必須在修法後繼續通過——新的 jq 查詢邏輯要對缺 `uuid` 的 entry 容錯(視為空字串)。
- 不做人工輸入獨立通道重構、不合併/抽出共用 hook 腳本檔案——皆已在 spec 階段確認排除,超出本次範圍。

---

### Task 1: 縮短 `getUpdates` 長輪詢逾時

**Files:**
- Modify: `src-tauri/src/adapters/telegram.rs:81`(改動點)、`src-tauri/src/adapters/telegram.rs` 頂部附近新增常數
- Test: `src-tauri/src/adapters/telegram.rs`(同檔案 `#[cfg(test)] mod tests`)

**Interfaces:**
- 不新增/不變更任何 public 函式簽章;`poll_updates`、`TelegramHttp::new` 行為不變,僅內部查詢字串的 `timeout` 數值改變。

- [ ] **Step 1: 寫失敗測試——斷言送出的請求 query string 含 `timeout=5`**

在 `src-tauri/src/adapters/telegram.rs` 的 `mod tests` 區塊(既有測試之後,`}` 結尾前)新增:

```rust
    // Phase 6:人工輸入通道與 30s long-poll 共用同一 thread,worst-case 延遲
    // 真機量測 ~28-30s(findings 2026-07-20/21)。縮短逾時值直接壓低這個上限。
    #[test]
    fn poll_updates_uses_shortened_longpoll_timeout() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let addr = listener.local_addr().expect("read local addr");

        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept connection");
            let mut received = Vec::new();
            let mut chunk = [0u8; 512];
            loop {
                let n = stream.read(&mut chunk).expect("read request");
                if n == 0 {
                    break;
                }
                received.extend_from_slice(&chunk[..n]);
                if received.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let body = b"{\"ok\":true,\"result\":[]}";
            let response = format!(
                "HTTP/1.1 200 OK\r\nconnection: close\r\ncontent-length: {}\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).expect("write headers");
            stream.write_all(body).expect("write body");
            stream.flush().expect("flush");
            received
        });

        let client = TelegramHttp::new("SECRET123TOKEN".to_string(), format!("http://{addr}"));
        client.poll_updates(0).expect("poll_updates should succeed against stub");

        let received = server.join().expect("stub server thread panicked");
        let request_line = String::from_utf8_lossy(&received);
        let first_line = request_line.lines().next().unwrap_or_default();

        assert!(
            first_line.contains("timeout=5") && !first_line.contains("timeout=30"),
            "expected shortened long-poll timeout in request line, got: {first_line}"
        );
    }
```

- [ ] **Step 2: 執行測試,確認 FAIL**

Run: `cargo test --manifest-path src-tauri/Cargo.toml adapters::telegram::tests::poll_updates_uses_shortened_longpoll_timeout -- --exact`
Expected: FAIL——`first_line` 含 `timeout=30`,斷言 `!first_line.contains("timeout=30")` 不成立。

- [ ] **Step 3: 實作——把逾時值改成具名常數 `5`**

修改 `src-tauri/src/adapters/telegram.rs` 第 76-87 行附近的 `poll_updates`,在 `impl TelegramGateway for TelegramHttp` 區塊之前(或緊接 `impl TelegramHttp` 內)新增常數,並在 `query` 呼叫改用它:

```rust
// Phase 6:人工輸入通道與此長輪詢共用同一 pipeline thread(gui.rs),
// worst-case 延遲即此逾時值;真機量測確認縮短後延遲隨之下降(findings)。
const GETUPDATES_LONGPOLL_SECS: &str = "5";
```

把原本的:

```rust
            .query(&[("offset", offset.to_string()), ("timeout", "30".to_string())])
```

改成:

```rust
            .query(&[
                ("offset", offset.to_string()),
                ("timeout", GETUPDATES_LONGPOLL_SECS.to_string()),
            ])
```

- [ ] **Step 4: 執行測試,確認 PASS**

Run: `cargo test --manifest-path src-tauri/Cargo.toml adapters::telegram::tests::poll_updates_uses_shortened_longpoll_timeout -- --exact`
Expected: PASS

- [ ] **Step 5: 執行整個 telegram 測試模組,確認無回歸**

Run: `cargo test --manifest-path src-tauri/Cargo.toml adapters::telegram::tests`
Expected: 全數 PASS(既有 4 個 + 新增 1 個 = 5 個測試)

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/adapters/telegram.rs
git commit -m "fix: getUpdates 長輪詢逾時 30s→5s,壓低人工輸入通道 worst-case 延遲"
```

---

### Task 2: 擴充 hook 測試——重現讀檔競態 + 三腳本一致性守衛(RED)

**Files:**
- Modify: `tests/hook/test_extract_reply.sh`

**Interfaces:**
- 消費既有 `templates/echo/.claude/hooks/extract-reply.sh`(尚未修改,行為與 Phase 2 落地時相同)。
- 本任務只新增測試,不動任何 hook 腳本;新測試中兩個「重現競態」案例預期在本任務結束時仍是 FAIL(下個任務才會修到 PASS),第三個「三腳本一致性」案例預期本任務結束時就是 PASS(現況三份腳本本來就相同,這條是防未來分歧的守衛,不是本次修法要修的紅燈)。

- [ ] **Step 1: 在 `tests/hook/test_extract_reply.sh` 新增兩個重現競態的測試函式**

在既有 `check()` 函式定義之後、`check tests/hook/fixture-transcript.jsonl ...` 呼叫之前,插入:

```bash
# Phase 6 findings(2026-07-21):Stop 事件觸發 hook 讀檔的當下,CLI 可能還沒把
# 當輪 entry 寫進磁碟上的 transcript 檔。以下兩個函式各自重現一種真機觀察到
# 的分岔:(1)續接 session 讀到「上一輪」內容重複誤送(2)全新 session 讀到
# 空字串(taster 逐則 /clear 情境)。兩者都用背景 delayed-write 模擬磁碟落地
# 的時間差。
check_retry_stale_duplicate() {
  local label="stale-duplicate-uuid-guard"
  local workdir transcript outfile actual last_uuid_written bgpid
  workdir=$(mktemp -d)
  mkdir -p "$workdir/outbox"
  transcript="$workdir/transcript.jsonl"

  # 上一輪(A)已經送出過,對應 uuid 預先寫進 .last-uuid,模擬 hook 上次成功
  # 寫出 A、狀態已落地
  printf 'A-UUID\n' > "$workdir/outbox/.last-uuid"
  printf '{"type":"assistant","uuid":"A-UUID","message":{"content":[{"type":"text","text":"reply A"}]}}\n' > "$transcript"

  # 背景延遲 0.3 秒後才把當輪(B)entry 落地——重試預算(下個任務會實作)是
  # 20 次 * 0.1 秒 = 2 秒,0.3 秒落在預算內
  (
    sleep 0.3
    printf '{"type":"assistant","uuid":"B-UUID","message":{"content":[{"type":"text","text":"reply B"}]}}\n' >> "$transcript"
  ) &
  bgpid=$!

  printf '{"transcript_path": "%s"}' "$transcript" \
    | CLAUDE_PROJECT_DIR="$workdir" templates/echo/.claude/hooks/extract-reply.sh

  wait "$bgpid"

  outfile=$(find "$workdir/outbox" -name '*-reply.json' 2>/dev/null | head -1)
  actual=$(jq -r '.text' "$outfile")
  last_uuid_written=$(cat "$workdir/outbox/.last-uuid" 2>/dev/null || echo "")

  if [ "$actual" = "reply B" ] && [ "$last_uuid_written" = "B-UUID" ]; then
    echo "PASS($label)"
  else
    echo "FAIL($label): got text='$actual' last-uuid='$last_uuid_written'"
    fail=1
  fi

  rm -rf "$workdir"
}

check_retry_empty_until_ready() {
  local label="empty-until-first-write-lands"
  local workdir transcript outfile actual bgpid
  workdir=$(mktemp -d)
  mkdir -p "$workdir/outbox"
  transcript="$workdir/transcript.jsonl"

  # 全新 session:transcript 一開始沒有任何 assistant entry(對應 taster 逐則
  # /clear 的情境),背景延遲後才寫入第一筆
  : > "$transcript"
  (
    sleep 0.3
    printf '{"type":"assistant","uuid":"C-UUID","message":{"content":[{"type":"text","text":"reply C"}]}}\n' >> "$transcript"
  ) &
  bgpid=$!

  printf '{"transcript_path": "%s"}' "$transcript" \
    | CLAUDE_PROJECT_DIR="$workdir" templates/echo/.claude/hooks/extract-reply.sh

  wait "$bgpid"

  outfile=$(find "$workdir/outbox" -name '*-reply.json' 2>/dev/null | head -1)
  actual=$(jq -r '.text' "$outfile")

  if [ "$actual" = "reply C" ]; then
    echo "PASS($label)"
  else
    echo "FAIL($label): expected 'reply C', got '$actual'"
    fail=1
  fi

  rm -rf "$workdir"
}

check_role_scripts_identical() {
  local label="role-hooks-byte-identical"
  if diff -q templates/echo/.claude/hooks/extract-reply.sh templates/taster/.claude/hooks/extract-reply.sh >/dev/null \
     && diff -q templates/echo/.claude/hooks/extract-reply.sh templates/cyrano/.claude/hooks/extract-reply.sh >/dev/null; then
    echo "PASS($label)"
  else
    echo "FAIL($label): role hook scripts diverged"
    fail=1
  fi
}
```

在檔案最後(既有兩行 `check ...` 呼叫之後、`exit "$fail"` 之前)新增呼叫:

```bash
check_retry_stale_duplicate
check_retry_empty_until_ready
check_role_scripts_identical
```

- [ ] **Step 2: 執行測試,確認兩個新競態案例 FAIL、一致性守衛 PASS**

Run: `bash tests/hook/test_extract_reply.sh`
Expected 輸出包含:
```
FAIL(stale-duplicate-uuid-guard): got text='reply A' last-uuid=''
FAIL(empty-until-first-write-lands): expected 'reply C', got ''
PASS(role-hooks-byte-identical)
```
(既有 `PASS(basic)`、`PASS(thinking-race)` 也應照舊出現)整體 exit code 非 0(因為前兩個 FAIL)。

- [ ] **Step 3: Commit**

```bash
git add tests/hook/test_extract_reply.sh
git commit -m "test: 新增 Stop hook 讀檔競態重現案例(RED,重試迴圈尚未實作)"
```

---

### Task 3: 修正 `templates/echo/.claude/hooks/extract-reply.sh`——uuid 重試迴圈(GREEN)

**Files:**
- Modify: `templates/echo/.claude/hooks/extract-reply.sh`

**Interfaces:**
- 消費 Task 2 新增的測試(`tests/hook/test_extract_reply.sh` 的 `check_retry_stale_duplicate`、`check_retry_empty_until_ready`、`check_role_scripts_identical`)。
- 本任務只改 `templates/echo/` 這一份;`taster`/`cyrano` 兩份留給 Task 4,此任務結束時 `check_role_scripts_identical` 預期會變成 FAIL(三份暫時不同),這是預期中的中間態,Task 4 會修回 PASS。

- [ ] **Step 1: 把 `templates/echo/.claude/hooks/extract-reply.sh` 整份改成以下內容**

```bash
#!/usr/bin/env bash
set -euo pipefail

input=$(cat)
transcript=$(printf '%s' "$input" | jq -r '.transcript_path')

outbox="${CLAUDE_PROJECT_DIR:?}/outbox"
mkdir -p "$outbox"

# Stop hook 讀檔競態(findings 2026-07-21):Stop 事件觸發 hook 讀檔的當下,CLI
# 可能還沒把當輪 entry 寫進磁碟上的 transcript 檔。用「候選 entry 的 uuid 是否
# 與上次成功送出的 uuid 相同」偵測是否抓到舊內容;文字為空(全新 session 首次
# 寫入還沒落地)也視為未就緒,一併重試。狀態記在 outbox 目錄下的隱藏檔
# .last-uuid(不可加 .json 副檔名——會被 adapters/outbox.rs 的 watcher 誤判為
# 回覆檔案)。
#
# 校準值(若真機測試發現不足需調大,數字本身是估計值不是精確量測):
RETRY_INTERVAL=0.1
RETRY_MAX_ATTEMPTS=20

last_uuid_file="$outbox/.last-uuid"
last_uuid=""
if [ -f "$last_uuid_file" ]; then
  last_uuid=$(cat "$last_uuid_file")
fi

candidate_uuid=""
candidate_text=""
attempt=0
while [ "$attempt" -lt "$RETRY_MAX_ATTEMPTS" ]; do
  if [ -f "$transcript" ]; then
    # thinking-race 實證(skeleton findings, Task 9):最後一個 assistant entry
    # 可能只有 thinking 區塊(text 尚未 flush)——取「最後一個含 text 區塊的
    # entry」而非單純 last;全都沒有 text 時 uuid/text 皆為空字串。既有 fixture
    # 沒有 uuid 欄位,以 `// ""` 容錯避免 null 造成後續比對出錯。
    candidate=$(jq -rs '
      [.[] | select(.type == "assistant")
           | select(([.message.content[]? | select(.type == "text")] | length) > 0)
           | {uuid: (.uuid // ""), text: ([.message.content[]? | select(.type == "text") | .text] | join("\n"))}]
      | last // {uuid: "", text: ""}
    ' "$transcript" 2>/dev/null || printf '{"uuid":"","text":""}')
  else
    candidate='{"uuid":"","text":""}'
  fi
  candidate_uuid=$(printf '%s' "$candidate" | jq -r '.uuid')
  candidate_text=$(printf '%s' "$candidate" | jq -r '.text')

  if [ -n "$candidate_text" ] && [ "$candidate_uuid" != "$last_uuid" ]; then
    break
  fi
  attempt=$((attempt + 1))
  sleep "$RETRY_INTERVAL"
done

reply="$candidate_text"

# rename-into-place(skeleton findings, final review):.partial 寫完再 mv
# 成 .json(同目錄 rename 原子),watcher 收到事件時內容保證完整;
# .partial 不以 .json 結尾,不會被 watcher 的副檔名過濾撿走
final="$outbox/$(date +%s)-$$-reply.json"
tmp="$final.partial"
printf '{"text": %s}\n' "$(printf '%s' "$reply" | jq -Rs .)" > "$tmp"
mv "$tmp" "$final"

if [ -n "$reply" ]; then
  printf '%s' "$candidate_uuid" > "$last_uuid_file"
fi
```

- [ ] **Step 2: 執行測試,確認新增的兩個競態案例轉為 PASS,既有兩個 fixture 案例仍 PASS**

Run: `bash tests/hook/test_extract_reply.sh`
Expected 輸出包含:
```
PASS(basic)
PASS(thinking-race)
PASS(stale-duplicate-uuid-guard)
PASS(empty-until-first-write-lands)
FAIL(role-hooks-byte-identical): role hook scripts diverged
```
(`role-hooks-byte-identical` 此時預期 FAIL,是本任務結束時的中間態,下個任務會修掉)整體 exit code 非 0。

- [ ] **Step 3: Commit**

```bash
git add templates/echo/.claude/hooks/extract-reply.sh
git commit -m "fix: extract-reply.sh(echo)加入 uuid 重試迴圈,修 Stop hook 讀檔競態"
```

---

### Task 4: 同步套用到 `taster`/`cyrano` 腳本

**Files:**
- Modify: `templates/taster/.claude/hooks/extract-reply.sh`
- Modify: `templates/cyrano/.claude/hooks/extract-reply.sh`

**Interfaces:**
- 內容與 Task 3 修好的 `templates/echo/.claude/hooks/extract-reply.sh` 逐位元組相同,無新介面。

- [ ] **Step 1: 把 Task 3 修好的 `templates/echo/.claude/hooks/extract-reply.sh` 內容複製到另外兩份**

```bash
cp templates/echo/.claude/hooks/extract-reply.sh templates/taster/.claude/hooks/extract-reply.sh
cp templates/echo/.claude/hooks/extract-reply.sh templates/cyrano/.claude/hooks/extract-reply.sh
```

- [ ] **Step 2: 執行完整 hook 測試,確認全數 PASS(含一致性守衛)**

Run: `bash tests/hook/test_extract_reply.sh`
Expected 輸出:
```
PASS(basic)
PASS(thinking-race)
PASS(stale-duplicate-uuid-guard)
PASS(empty-until-first-write-lands)
PASS(role-hooks-byte-identical)
```
exit code 0。

- [ ] **Step 3: 確認執行權限與原檔一致(cp 應已保留,仍需明確驗證)**

Run: `ls -l templates/echo/.claude/hooks/extract-reply.sh templates/taster/.claude/hooks/extract-reply.sh templates/cyrano/.claude/hooks/extract-reply.sh`
Expected: 三個檔案的權限位元(如 `-rwxr-xr-x`)一致,皆可執行(`x` 位元存在)。若任一份權限不同,執行 `chmod --reference=templates/echo/.claude/hooks/extract-reply.sh templates/taster/.claude/hooks/extract-reply.sh templates/cyrano/.claude/hooks/extract-reply.sh` 修正。

- [ ] **Step 4: Commit**

```bash
git add templates/taster/.claude/hooks/extract-reply.sh templates/cyrano/.claude/hooks/extract-reply.sh
git commit -m "fix: extract-reply.sh(taster/cyrano)同步 uuid 重試迴圈修法"
```

---

## Deploy 提醒(非本 plan 任務,執行者完成 Task 1-4 後應告知使用者)

`templates/` 是版控正本;實際運行的 CLI 工作目錄在 repo 外的 `../clacks-runtime/{taster,cyrano}/`(見 architecture.md 目錄結構說明)。本 plan 的變更**不會自動同步**到 runtime 目錄——這四個 task 完成後,若要在真機驗證或實際跑 GUI,需要使用者自行把 `templates/{taster,cyrano}/.claude/hooks/extract-reply.sh` 的新內容部署過去(`cp -R` 或既有的部署腳本,依專案慣例,本 plan 不代為執行,因為那會覆蓋 runtime 目錄裡可能存在的、repo 外的狀態)。
