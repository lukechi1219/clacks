# Clacks

Tauri 桌面 app:Telegram 訊息經雙 Claude CLI 管線處理——taster(消毒者,零工具無記憶)擋 prompt injection,cyrano(回應者,唯讀白名單)代筆回覆,Rust 後端是唯一中介(bot token 不出 Rust)。

## 必讀文件

@architecture.md

- 產品設計與安全模型:[docs/superpowers/specs/2026-07-17-clacks-design.md](docs/superpowers/specs/2026-07-17-clacks-design.md)

## 規劃守則

寫 design 或 implementation plan 時:

- **任務拆解到低階 AI 模型也能獨立完成的粒度**:每個小區塊有明確的輸入、輸出、完成定義,不需要跨任務的隱含脈絡就能動工;一個區塊改不動超過 2-3 個檔案就該再拆
- **每項任務寫清楚驗證方式**:具體到可執行——要跑哪個指令、看到什麼輸出才算過(如 `cargo test core::contract`、「fake CLI 回傳畸形 JSON 時訊息標 failed」),不接受「確認功能正常」這種寫法

## 開發守則

- 遵守 architecture.md 的依賴規則:core 不碰 IO,orchestrator 不 import adapter,composition root 是唯一組裝點
- 安全關鍵邏輯(JSON 契約驗證、信封包裝、狀態機)一律放 `src-tauri/src/core/`,以純函式 + 單元測試覆蓋
- 不使用 `claude -p` / Agent SDK(成本約束):只跑訂閱制互動式 CLI(PTY 模式)
- 授權:MIT + Apache-2.0 雙授權;新增依賴前確認授權相容
