# Clacks — 架構文件

> 日期:2026-07-17
> 前置文件:[docs/superpowers/specs/2026-07-17-clacks-design.md](docs/superpowers/specs/2026-07-17-clacks-design.md)(產品設計與安全模型)
> 本文件回答:程式碼如何分層、依賴方向、以及為什麼

## 架構風格:Clean Architecture Lite

本專案採用 **hexagonal(ports & adapters)的依賴規則**,但**不採用**完整 DDD 儀式(bounded contexts、AggregateRoot、domain event bus)。

判斷依據:clacks 的複雜度在**整合點**(PTY、Telegram API、檔案監看、行程生命週期),不在領域規則。領域邏輯很薄——狀態機、JSON 契約驗證、去重、session 維護決策——但這層薄邏輯正是安全模型的執法者,**最值得純函式測試**。因此:

- 在整合點插 trait(port),讓核心邏輯與 IO 解耦、可測
- 跳過為「多 bounded context、多消費端」設計的結構——clacks 只有一條 pipeline、一個消費者

參考案例:`../meumetric` 是完整 hexagonal + DDD 的實作(7 個 bounded context、shared-kernel、event bus)。該專案的複雜度在領域規則(訓練引擎、財務計算),且 4 個 app 共用同一套邏輯,完整分層划算。clacks 規模不及其十分之一,照抄即是 over-engineering。

## 實作策略:Walking Skeleton 優先

實作順序不是純 inside-out,而是先讓一條最細的端到端細線穿過所有高風險整合點:

1. **Phase 1 — Walking skeleton**:hard-coded echo pipeline(Telegram → PTY 注入 → Stop hook → 回覆),一次驗證四個高風險假設:PTY bracketed paste 注入、Stop hook 觸發時機、`/clear` 行為、sandbox-exec 相容性。骨架位於 `skeleton/`(獨立 bin crate,不建 Tauri)
2. **Phase 2 — 提煉 ports**:從骨架觀察到的**真實行為**定義 `ports.rs` 的 trait 語意;骨架碼整理成第一版 adapters(只搬運,不加邏輯)
3. **Phase 3+ — inside-out**:core 純函式 → orchestrator + fake ports → 替換骨架接線 → 第二個 CLI、雙 agent 分工、儲存、GUI 逐層長上去

### 骨架期護欄(不可協商)

- 骨架範圍 = 最細可跑細線,不是「把整合層做完」
- **骨架期 adapter 保持愚蠢**:只做搬運(注入、監看、收發),任何決策邏輯(重試、timeout 政策、何時 `/clear`、何時算失敗)一律留白,等 core 來填。骨架裡 `expect`/panic 是合法的
- 骨架的整合發現(hook 實際觸發時機、sandbox 實際限制)必須寫回 `docs/superpowers/notes/`——它們是 Phase 2 port 語意的實證依據

每個 phase 有自己的 implementation plan(`docs/superpowers/plans/`),前一階段的證據落地後才寫下一份,避免在猜測上做細部規劃。

## 分層與目錄結構

```
clacks/
├── src-tauri/src/
│   ├── core/            # 純邏輯層 — 不碰 IO、不依賴 tokio/tauri
│   │   ├── pipeline.rs       # 訊息生命週期狀態機(Idle/Busy/Failed、timeout 決策)
│   │   ├── contract.rs       # taster JSON 契約型別 + 嚴格 schema 驗證
│   │   ├── envelope.rs       # 不可信訊息的信封包裝(消毒指示模板)
│   │   └── session.rs        # /clear、/compact 觸發決策(輸入估算值,輸出決定)
│   ├── ports.rs         # trait 定義 — core 與外界的唯一介面
│   ├── adapters/        # port 的具體實作
│   │   ├── telegram.rs       # TelegramGateway:teloxide/reqwest 打 Bot API
│   │   ├── pty.rs            # CliSession:portable-pty 管理互動式 claude
│   │   ├── outbox.rs         # notify 監看 outbox/,轉成事件餵給 orchestrator
│   │   └── store.rs          # MessageStore:rusqlite(update_id 去重、訊息歷史)
│   ├── app.rs           # orchestrator:驅動 pipeline,只依賴 core + ports
│   └── main.rs          # composition root:唯一的組裝點(建 adapter、注入、啟動)
├── src/                 # React 前端 — 不分層(兩塊 xterm.js pane + 狀態顯示,保持薄)
├── templates/           # runtime 工作目錄的版控正本(角色 CLAUDE.md、settings、hooks);部署 = cp -R 到 ../clacks-runtime/
└── (repo 外)../clacks-runtime/   # taster/、cyrano/ 工作目錄(CLAUDE.md 角色指示、settings、outbox)
                         # 必須在 repo 目錄樹外:祖先 CLAUDE.md 遍歷跨 git 邊界,嵌套會污染
                         # 隔離 CLI 的 context(實證見 docs/superpowers/notes/2026-07-17-skeleton-findings.md)
```

## 依賴規則(不可協商)

1. **core → 無**:不 import tokio、tauri、任何 adapter。只有 std + serde
2. **app(orchestrator)→ core + ports**:永不直接 import adapter
3. **adapters → ports + core 型別**:實作 trait,彼此不互相依賴
4. **main(composition root)是唯一知道具體型別的地方**:建構 adapter、注入 orchestrator

違反規則的典型徵兆:core 裡出現 `async fn` 以外的 tokio 型別、orchestrator 裡出現 `rusqlite::` 或 `portable_pty::` 路徑。

## Ports 清單(刻意控制在 5 個以內)

| Port | 職責 | 生產實作 | 測試替身 |
|---|---|---|---|
| `TelegramGateway` | 拉 updates、送回覆 | teloxide / reqwest | 記錄呼叫的 fake |
| `CliSession` | 注入 prompt、觀察 outbox 產物、重啟 | portable-pty + notify | 腳本化回應的 fake |
| `MessageStore` | update_id 去重、訊息狀態落地 | rusqlite | in-memory HashMap |
| `Clock` | 現在時刻(timeout 判斷用) | `SystemTime` | 手動撥針的 fake |

Rust 的 async trait 有 boilerplate 稅(`async_trait` 或 RPITIT);port 數量是刻意的預算,新增第 6 個 port 前先問「能不能併進現有 port」。

## 測試策略對應

設計文件的測試策略直接映射到分層:

| 測試 | 層 | 依賴 |
|---|---|---|
| JSON 契約驗證、狀態機轉移、去重 | core 單元測試 | 零(純函式) |
| orchestrator 流程(timeout、重試、佇列) | app + fake ports | 測試替身 |
| Hook 腳本抽取回覆 | 腳本測試 | 假 transcript JSONL |
| 端到端 pipeline | 全部真實 adapter | mock Telegram server + 真實 CLI |

安全關鍵路徑(契約驗證、信封包裝)必須留在 core:純函式 + 窮舉測試,不給 IO 摻雜的機會。

## 刻意不採用

| 不採用 | 理由 | 何時重新考慮 |
|---|---|---|
| bounded contexts / modules 切分 | 只有一條 pipeline,一個領域 | 出現第二個獨立領域(如帳號管理、多 bot) |
| domain event bus | 事件流是線性的,直接函式呼叫即可 | 出現多個彼此解耦的事件消費者 |
| AggregateRoot / DomainEvent 基底類 | Rust newtype + enum 原生表達力已足夠 | (不太可能) |
| 前端分層 | UI 只是 render PTY 輸出 + 狀態 | 前端出現自己的業務邏輯 |

## 名詞速查

taster(消毒者 CLI)、cyrano(回應者 CLI)、Woodpecker(威脅模型:訊息即攻擊 payload)——完整脈絡見設計文件。
