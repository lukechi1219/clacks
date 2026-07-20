//! session 維護決策(architecture.md:/clear、/compact 觸發決策——輸入估算值,
//! 輸出決定)。「/clear 在每則 taster 訊息後」的決策本身編在 pipeline.rs 的
//! 動作序列;本檔放跨元件共用的節奏常數與門檻函式。

use std::time::Duration;

/// 控制指令(/clear、/compact)注入後、下一次注入前的強制緩衝。
///
/// smoke 真機實證(findings「Phase 2 smoke」):/clear 尚在處理時注入的
/// paste 信封被 TUI 丟棄,殘留的 \r 送出空 prompt,模型收到空輸入自由發揮。
/// 量級未量測(Phase 4 量測項)——先取保守值;落點在 orchestrator 的
/// exec(ClearTaster)(Global Constraints 5)
pub const CONTROL_BUFFER: Duration = Duration::from_secs(2);

/// 設計文件預設:注入後 5 分鐘未見 hook 產物 → 該訊息判 failed
pub const ARTIFACT_TIMEOUT: Duration = Duration::from_secs(300);

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

/// cyrano transcript(JSONL)大小門檻,超過即應在 Idle 時注入 /compact。
/// 粗估值,待真實 transcript 量測校正;/compact 佈線屬 Phase 4
/// (需 port 擴充提供估算輸入),本 phase 先落純決策
pub const COMPACT_THRESHOLD_BYTES: u64 = 512 * 1024;

pub fn should_compact(transcript_bytes: u64) -> bool {
    transcript_bytes >= COMPACT_THRESHOLD_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn below_threshold_no_compact() {
        assert!(!should_compact(COMPACT_THRESHOLD_BYTES - 1));
    }

    #[test]
    fn at_threshold_compacts() {
        assert!(should_compact(COMPACT_THRESHOLD_BYTES));
    }

    #[test]
    fn idle_quiet_is_shorter_than_settle_timeout() {
        // 靜默視窗必須遠小於就緒上限,否則永遠等不到一個完整靜默窗
        assert!(IDLE_QUIET < IDLE_SETTLE_TIMEOUT);
    }
}
