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

    /// 重啟 session:teardown 當前 child(Phase 3 kill+wait 保證無殘留)後起新的。
    ///
    /// - taster 重啟 = 全新 PTY + 全新 session 檔 = **乾淨**(消毒者無記憶,安全義務)
    /// - cyrano 重啟以 `claude --continue` 續談(設計文件錯誤處理)——**真機驗證項**:
    ///   `--continue` 能否恢復對話待真機證實,失敗 fallback 為全新 session 並記錄
    ///
    /// 失敗(建新 session 失敗)時 self 維持原狀,回 Err 供 orchestrator 記錄並於下則訊息重試
    fn respawn(&mut self) -> Result<(), CliError>;
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
