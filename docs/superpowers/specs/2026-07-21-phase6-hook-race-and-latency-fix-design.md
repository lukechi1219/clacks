# Phase 6 — Stop hook 讀檔競態 + 人工輸入延遲 修法設計文件

> 日期:2026-07-21
> 前置文件:[architecture.md](../../../architecture.md)、[docs/superpowers/notes/2026-07-17-skeleton-findings.md](../notes/2026-07-17-skeleton-findings.md)(Phase 5 Task 12 真機發現,本文件兩項修法的問題根源)
> 狀態:已與使用者討論定案,待寫 implementation plan

## 背景

Phase 5 真機測試(Task 12)記錄了兩項已知限制,均已裁決「先記錄、暫不修」。本文件為這兩項的正式修法設計:

1. **Stop hook 讀檔競態**(較高風險):`extract-reply.sh` 在 CLI 把當輪內容寫進磁碟上的 transcript 檔**之前**就被 Stop 事件觸發讀檔,依 session 是否有先前內容分岔成兩種錯誤——`EmptyReply`(taster,fail-closed 安全但訊息漏失)或**內容誤送**(cyrano,答非所問的錯誤內容被實際送出,風險較高,因為使用者收到的是「看似合法但錯誤」的回覆)。連發訊息與 respawn 後首次注入皆會觸發,是同一根因。
2. **人工輸入 30s 延遲**:人工介入通道與 Telegram `getUpdates` 30 秒長輪詢共用同一 pipeline thread,worst-case 延遲真機量測確認約 28 秒,牴觸本 phase 設計動機之一(pane 手動輸入取代 headless 的 pre-seed 死鎖)。

兩項修法彼此獨立,可分開實作、分開驗證。

## 修法 A:Stop hook 重試迴圈(uuid 比對)

### 現況程式碼

`templates/taster/.claude/hooks/extract-reply.sh` 與 `templates/cyrano/.claude/hooks/extract-reply.sh` 兩份**內容完全相同**(需同步修改):

```bash
reply=$(jq -rs '
  [.[] | select(.type == "assistant")
       | [.message.content[]? | select(.type == "text") | .text]
       | select(length > 0)]
  | last // []
  | join("\n")
' "$transcript")
```

抓「最後一個含 text 區塊的 assistant entry」的文字,寫進 outbox。問題:這個查詢執行的當下,CLI 可能還沒把當輪 entry 寫進磁碟。

### 設計:uuid 而非文字內容作為新鮮度判斷依據

選用 uuid 比對而非原始文字比對(比對文字有假陽性風險——連續兩次内容剛好相同的合法回覆會被誤判為「還是舊的」而多等一輪)。Transcript 的 assistant entry 本身帶 `uuid` 欄位(findings 已引用實例:`uuid=3b80d58f`),兩個不同回合的 uuid 必然不同,即使文字相同也不會誤判。

### 狀態檔案

- 位置:`$outbox/.last-uuid`(與 outbox 同目錄,但不受 `watch_outbox` 影響——`adapters/outbox.rs:25` 的過濾條件是 `path.extension().is_some_and(|e| e == "json")`,`.last-uuid` 沒有 `.json` 副檔名,直接被忽略,已讀原始碼確認,非猜測)。
- 內容:單行,最近一次成功寫入 outbox 的 assistant entry uuid。
- 生命週期:每個角色(taster/cyrano)各自獨立一份,跟著各自的 `outbox` 目錄走,不需要按 session 分檔——因為 `wait_idle` 已保證同一時間只有一輪注入/生成在進行,Stop 事件天生循序觸發,不會有並發讀寫這個狀態檔的情況。

### 演算法

```
last_uuid = 讀 $outbox/.last-uuid(不存在則視為空字串)
candidate = {uuid: "", text: ""}

for attempt in 1..=RETRY_MAX_ATTEMPTS:
    candidate = 從 transcript 抽取「最後一個含 text 的 assistant entry」的 {uuid, text}
    if candidate.text 非空 AND candidate.uuid != last_uuid:
        break  # 抓到新鮮內容
    sleep RETRY_INTERVAL

# 迴圈結束(成功或耗盡預算),行為與現況一致:
寫 candidate.text 進 outbox(既有 .partial + mv 邏輯不動)
if candidate.text 非空:
    寫 candidate.uuid 進 $outbox/.last-uuid
```

**耗盡預算時的行為刻意維持現況、不做額外處理**:

- taster 若耗盡預算仍拿到空字串 → 跟現在一樣送空字串 → core 既有 `ContractViolation(EmptyReply)` fail-closed 擋下,不是新行為,只是發生機率因重試而降低。
- cyrano 若耗盡預算仍拿到「未變」的 uuid(即真的還沒抓到新內容) → 送出當時抓到的內容(可能仍是舊回覆)。這與**修法前**的行為完全相同(現況本來就是無條件送出當下抓到的內容),差別只在於多給了幾次重試機會去抓到新鮮內容,不會讓最壞情況變得更差。

### 校準常數(比照 `session.rs` 的 `IDLE_QUIET`/`IDLE_SETTLE_TIMEOUT` 慣例,標記為需真機校準)

- `RETRY_INTERVAL`:每次重試間隔,提案 `0.1`(秒)。
- `RETRY_MAX_ATTEMPTS`:提案 `20`(合計最壞情況 ~2 秒的額外延遲,發生在 Stop 事件之後、outbox 寫入之前,不影響 `wait_idle` 或注入時機)。

兩個值都要在 script 開頭以有註解的常數形式呈現(說明來源是「CLI 生成完成到 transcript 落地磁碟」的真實延遲量級,實際數字待真機校準,若真機測試發現不足需調大,不要在 review 時視為武斷數字)。

### 與現有機制的關係

此修法完全獨立於 Rust 端的 `wait_idle`(那個管的是**注入前**的 PTY 靜默偵測);這裡處理的是**生成完成後**、hook 讀檔與磁碟落地之間的競態,兩個機制不互相依賴、互不影響。

## 修法 B:縮短 `getUpdates` 長輪詢逾時

### 現況程式碼

`src-tauri/src/adapters/telegram.rs:81`:

```rust
.query(&[("offset", offset.to_string()), ("timeout", "30".to_string())])
```

HTTP client 逾時為 40 秒(`telegram.rs:59`),留有 10 秒緩衝空間。

### 設計

`timeout` 查詢參數由 `30` 改為 `5`(字面值,以常數形式命名,如 `GETUPDATES_LONGPOLL_SECS: &str = "5"`,方便未來調整與在程式碼中被找到)。HTTP client 的 40 秒逾時**不需要跟著調**——它只需要大於長輪詢逾時即可,5 秒 < 40 秒,原有緩衝依然充足。

### 影響面

- 人工輸入通道的 worst-case 延遲從 ~28-30 秒降到 ~5 秒(`input_rx` 排空發生在每輪迴圈開頭,`poll_once` 的長輪詢是下一個阻塞點,阻塞時間上限即為此逾時值)。
- Telegram API 呼叫頻率提高(30 秒一次 → 5 秒一次,約 6 倍),對單一 bot 的正常流量而言不構成 rate limit 疑慮(long-poll 本身就是官方建議的省呼叫手段,5 秒仍遠優於傳統短輪詢)。
- 不涉及 orchestrator/session 並發結構變動(findings 中的「獨立通道」方案因需改動 `orchestrator 獨佔 &mut session` 的設計而被使用者否決,本修法刻意避開)。

## 測試策略

### 修法 A(hook 腳本)

- Hook script 目前**沒有**現成的自動化測試(骨架期的 shell script,測試策略對應表寫的是「Hook 腳本抽取回覆 / 腳本測試 / 假 transcript JSONL」)。延續此模式:新增一個腳本測試,餵入一個**分兩階段寫入**的假 transcript(先寫只到前一輪的內容,短暫延遲後再 append 當輪 entry,模擬磁碟寫入延遲),驗證 hook 最終抓到的是新內容而非重複舊內容。
- 需覆蓋:(1)首次執行、無 `.last-uuid` 檔案時正常抓取;(2)候選 uuid 與 `.last-uuid` 相同時會重試,直到抓到新 uuid 或逾時;(3)`.last-uuid` 在成功寫出後確實更新。
- 因為兩份腳本內容相同,測試也應對兩份腳本各跑一次(或以其中一份的邏輯為準,plan 階段再決定是否值得抽成共用腳本——**不在本次設計範圍內主動做**,只有在 plan 階段發現重複帶來明顯維護成本時才考慮,避免超出兩項修法本身的範圍)。

### 修法 B(逾時常數)

- `telegram.rs` 已有既存的 mock/測試替身模式(`base_url` 可注入不可達位址做 redaction 測試)。新增一個測試斷言 `poll_updates` 送出的查詢字串包含新的逾時數值,防止未來誤改回舊值。
- 真機驗證:比照 Task 12 的方法,對存活中的人工輸入情境量測實際延遲,確認落在新逾時值附近而非仍卡在舊的 ~30 秒。

## 刻意不做

- 不把人工輸入通道改成獨立、更短週期的通道(findings 記錄的候選 2)——需要變動 `orchestrator` 獨佔 `&mut session` 的設計以支援跨執行緒共享,規模超出本次兩項修法的範圍,使用者已於規劃階段確認排除。
- 不把兩份 hook script 抽成共用檔案——除非 plan 階段執行時發現重複改動兩次的成本明顯高於抽共用的成本,否則維持現況(兩份獨立、內容一致)的簡單性優先。
- 不改動 `wait_idle`、`IDLE_QUIET`、`IDLE_SETTLE_TIMEOUT` 等既有機制——本次兩項修法與其正交,不涉及修改。
