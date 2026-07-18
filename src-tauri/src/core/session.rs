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
}
