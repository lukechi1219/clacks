//! 純邏輯層:不碰 IO、不依賴 tokio/tauri,只有 std + serde(architecture.md 依賴規則 1)。
//! 安全關鍵路徑(契約驗證、信封包裝、狀態機)全在此層,以純函式 + 窮舉測試覆蓋。

pub mod contract;
pub mod envelope;
pub mod pipeline;
pub mod session;
