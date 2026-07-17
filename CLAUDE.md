# Clacks

Tauri 桌面 app:Telegram 訊息經雙 Claude CLI 管線處理——taster(消毒者,零工具無記憶)擋 prompt injection,cyrano(回應者,唯讀白名單)代筆回覆,Rust 後端是唯一中介(bot token 不出 Rust)。

## 必讀文件

@architecture.md

- 產品設計與安全模型:[docs/superpowers/specs/2026-07-17-clacks-design.md](docs/superpowers/specs/2026-07-17-clacks-design.md)

## 開發守則

- 遵守 architecture.md 的依賴規則:core 不碰 IO,orchestrator 不 import adapter,composition root 是唯一組裝點
- 安全關鍵邏輯(JSON 契約驗證、信封包裝、狀態機)一律放 `src-tauri/src/core/`,以純函式 + 單元測試覆蓋
- 不使用 `claude -p` / Agent SDK(成本約束):只跑訂閱制互動式 CLI(PTY 模式)
- 授權:MIT + Apache-2.0 雙授權;新增依賴前確認授權相容
