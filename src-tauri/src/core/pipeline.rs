//! 訊息生命週期狀態機(architecture.md:pipeline.rs)。
//!
//! 純函式設計:狀態機吃事件(CliEvent)、吐動作(Action),自己不碰任何 IO。
//! orchestrator(app.rs)是它的解譯器——把 Action 映射成 port 呼叫、把
//! port 結果映射回 CliEvent。安全關鍵決策(什麼可以送去 cyrano、什麼必須
//! 丟棄、taster 何時 /clear)全部在這裡,以窮舉測試覆蓋。
//!
//! 骨架/smoke 實證對應:
//! - 注入分兩類(訊息 vs 控制指令):狀態機以 Action 區分 InjectTaster/
//!   InjectCyrano(期待產物)與 ClearTaster(不期待產物)
//! - taster 無記憶不可協商:每則訊息無論結果(成功/拒收/違約/timeout)
//!   一律排 ClearTaster;唯 SessionLost 例外(session 已死,清了也沒對象)
//! - 安全路徑動作序列刻意是 [InjectCyrano, ClearTaster]:先讓 cyrano 開始
//!   思考,再清 taster——orchestrator 的 CONTROL_BUFFER 緩衝(阻塞 2s)
//!   與 cyrano 的回覆生成重疊,不虛耗牆鐘時間

use crate::core::contract::{self, ContractViolation};
use crate::core::envelope;
use std::time::Duration;

/// 一則訊息的終局。前六種由狀態機判定;後三種(SkippedNonText/SendFailed/
/// StoreFailed)由 orchestrator 在狀態機之外判定,共用同一個結果詞彙表
/// (GUI/紀錄的單一型別)
#[derive(Debug, Clone, PartialEq)]
pub enum MessageOutcome {
    Replied,
    RejectedByTaster { reason: String },
    ContractViolation(ContractViolation),
    TasterTimeout,
    CyranoTimeout,
    SessionLost,
    SkippedNonText,
    SendFailed(String),
    StoreFailed(String),
}

#[derive(Debug, PartialEq)]
pub enum Action {
    InjectTaster(String),
    /// 注入 /clear;orchestrator 執行後必須套 session::CONTROL_BUFFER 緩衝
    ClearTaster,
    InjectCyrano(String),
    SendReply { chat_id: i64, text: String },
}

#[derive(Debug)]
pub enum CliEvent {
    /// hook 產物的 raw 內容
    Artifact(String),
    Timeout,
    /// session 不可用(watcher channel 斷線、注入失敗)
    Lost,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AwaitTarget {
    Taster,
    Cyrano,
}

#[derive(Debug)]
enum State {
    AwaitingTaster,
    AwaitingCyrano,
    Done(MessageOutcome),
}

pub struct MessagePipeline {
    chat_id: i64,
    state: State,
}

impl MessagePipeline {
    pub fn start(chat_id: i64, text: &str) -> (Self, Vec<Action>) {
        (
            Self { chat_id, state: State::AwaitingTaster },
            vec![Action::InjectTaster(envelope::wrap_for_taster(text))],
        )
    }

    pub fn awaiting(&self) -> Option<AwaitTarget> {
        match self.state {
            State::AwaitingTaster => Some(AwaitTarget::Taster),
            State::AwaitingCyrano => Some(AwaitTarget::Cyrano),
            State::Done(_) => None,
        }
    }

    pub fn outcome(&self) -> Option<&MessageOutcome> {
        match &self.state {
            State::Done(outcome) => Some(outcome),
            _ => None,
        }
    }

    pub fn advance(&mut self, event: CliEvent) -> Vec<Action> {
        match (self.awaiting(), event) {
            (Some(AwaitTarget::Taster), CliEvent::Artifact(raw)) => self.judge_taster(&raw),
            (Some(AwaitTarget::Taster), CliEvent::Timeout) => {
                self.finish(MessageOutcome::TasterTimeout, vec![Action::ClearTaster])
            }
            (Some(AwaitTarget::Cyrano), CliEvent::Artifact(raw)) => self.judge_cyrano(&raw),
            (Some(AwaitTarget::Cyrano), CliEvent::Timeout) => {
                self.finish(MessageOutcome::CyranoTimeout, vec![])
            }
            (Some(_), CliEvent::Lost) => self.finish(MessageOutcome::SessionLost, vec![]),
            (None, _) => vec![],
        }
    }

    fn judge_taster(&mut self, raw: &str) -> Vec<Action> {
        let verdict = contract::extract_reply_text(raw)
            .and_then(|text| contract::parse_verdict(&text));
        match verdict {
            Err(violation) => self.finish(
                MessageOutcome::ContractViolation(violation),
                vec![Action::ClearTaster],
            ),
            Ok(v) if !v.safe => self.finish(
                MessageOutcome::RejectedByTaster { reason: v.reason },
                vec![Action::ClearTaster],
            ),
            Ok(v) => {
                let inject = envelope::wrap_for_cyrano(&v.sanitized_text, self.chat_id);
                self.state = State::AwaitingCyrano;
                vec![Action::InjectCyrano(inject), Action::ClearTaster]
            }
        }
    }

    fn judge_cyrano(&mut self, raw: &str) -> Vec<Action> {
        match contract::extract_reply_text(raw) {
            Err(violation) => {
                self.finish(MessageOutcome::ContractViolation(violation), vec![])
            }
            Ok(text) => {
                let reply = Action::SendReply { chat_id: self.chat_id, text };
                self.finish(MessageOutcome::Replied, vec![reply])
            }
        }
    }

    fn finish(&mut self, outcome: MessageOutcome, actions: Vec<Action>) -> Vec<Action> {
        self.state = State::Done(outcome);
        actions
    }
}

/// poll 失敗的指數退避:1s、2s、4s … 封頂 64s(骨架實證:os 53 是本環境
/// 系統性現象,重試政策屬 orchestrator)
pub fn poll_backoff(consecutive_errors: u32) -> Duration {
    let exponent = consecutive_errors.saturating_sub(1).min(6);
    Duration::from_secs(1u64 << exponent)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAFE_VERDICT: &str =
        r#"{"safe":true,"sanitized_text":"想請教一個問題","removed":[],"reason":"乾淨"}"#;
    const UNSAFE_VERDICT: &str =
        r#"{"safe":false,"sanitized_text":"","removed":["指令注入段落"],"reason":"要求執行破壞性指令"}"#;

    /// 把 verdict JSON 包成 hook 產物 {"text": "<json>"}(嵌套跳脫交給 serde)
    fn taster_artifact(verdict_json: &str) -> String {
        serde_json::json!({ "text": verdict_json }).to_string()
    }

    #[test]
    fn start_injects_enveloped_text_and_awaits_taster() {
        let (pipeline, actions) = MessagePipeline::start(42, "hello");
        assert_eq!(pipeline.awaiting(), Some(AwaitTarget::Taster));
        assert_eq!(actions.len(), 1);
        let Action::InjectTaster(text) = &actions[0] else {
            panic!("第一個動作必須是 InjectTaster,得到 {actions:?}");
        };
        assert!(text.contains("---BEGIN UNTRUSTED MESSAGE---"));
        assert!(text.contains("hello"));
    }

    #[test]
    fn safe_verdict_clears_taster_and_injects_cyrano() {
        let (mut pipeline, _) = MessagePipeline::start(42, "hi");
        let actions = pipeline.advance(CliEvent::Artifact(taster_artifact(SAFE_VERDICT)));
        // 順序刻意:先 InjectCyrano 再 ClearTaster(緩衝與 cyrano 思考重疊)
        assert_eq!(actions.len(), 2);
        let Action::InjectCyrano(text) = &actions[0] else {
            panic!("預期 InjectCyrano,得到 {actions:?}");
        };
        assert!(text.contains("想請教一個問題"));
        assert!(text.contains("42"));
        assert_eq!(actions[1], Action::ClearTaster);
        assert_eq!(pipeline.awaiting(), Some(AwaitTarget::Cyrano));
        assert_eq!(pipeline.outcome(), None);
    }

    #[test]
    fn unsafe_verdict_rejects_and_still_clears() {
        let (mut pipeline, _) = MessagePipeline::start(42, "hi");
        let actions = pipeline.advance(CliEvent::Artifact(taster_artifact(UNSAFE_VERDICT)));
        assert_eq!(actions, vec![Action::ClearTaster]);
        assert_eq!(
            pipeline.outcome(),
            Some(&MessageOutcome::RejectedByTaster {
                reason: "要求執行破壞性指令".to_string()
            })
        );
        assert_eq!(pipeline.awaiting(), None);
    }

    #[test]
    fn malformed_taster_reply_is_violation_and_still_clears() {
        let (mut pipeline, _) = MessagePipeline::start(42, "hi");
        let actions =
            pipeline.advance(CliEvent::Artifact(taster_artifact("這不是 JSON")));
        assert_eq!(actions, vec![Action::ClearTaster]);
        assert!(matches!(
            pipeline.outcome(),
            Some(MessageOutcome::ContractViolation(ContractViolation::NotJson(_)))
        ));
    }

    #[test]
    fn empty_taster_artifact_is_violation() {
        let (mut pipeline, _) = MessagePipeline::start(42, "hi");
        pipeline.advance(CliEvent::Artifact(r#"{"text":""}"#.to_string()));
        assert_eq!(
            pipeline.outcome(),
            Some(&MessageOutcome::ContractViolation(ContractViolation::EmptyReply))
        );
    }

    #[test]
    fn taster_timeout_still_clears() {
        let (mut pipeline, _) = MessagePipeline::start(42, "hi");
        let actions = pipeline.advance(CliEvent::Timeout);
        assert_eq!(actions, vec![Action::ClearTaster]);
        assert_eq!(pipeline.outcome(), Some(&MessageOutcome::TasterTimeout));
    }

    #[test]
    fn taster_lost_terminates_without_actions() {
        let (mut pipeline, _) = MessagePipeline::start(42, "hi");
        let actions = pipeline.advance(CliEvent::Lost);
        assert!(actions.is_empty());
        assert_eq!(pipeline.outcome(), Some(&MessageOutcome::SessionLost));
    }

    fn pipeline_awaiting_cyrano() -> MessagePipeline {
        let (mut pipeline, _) = MessagePipeline::start(42, "hi");
        pipeline.advance(CliEvent::Artifact(taster_artifact(SAFE_VERDICT)));
        assert_eq!(pipeline.awaiting(), Some(AwaitTarget::Cyrano));
        pipeline
    }

    #[test]
    fn cyrano_reply_sends_to_originating_chat() {
        let mut pipeline = pipeline_awaiting_cyrano();
        let actions =
            pipeline.advance(CliEvent::Artifact(r#"{"text":"這是回覆"}"#.to_string()));
        assert_eq!(
            actions,
            vec![Action::SendReply { chat_id: 42, text: "這是回覆".to_string() }]
        );
        assert_eq!(pipeline.outcome(), Some(&MessageOutcome::Replied));
    }

    #[test]
    fn empty_cyrano_reply_never_sent() {
        // thinking race 實證:空產物不可送出空回覆
        let mut pipeline = pipeline_awaiting_cyrano();
        let actions = pipeline.advance(CliEvent::Artifact(r#"{"text":" "}"#.to_string()));
        assert!(actions.is_empty());
        assert_eq!(
            pipeline.outcome(),
            Some(&MessageOutcome::ContractViolation(ContractViolation::EmptyReply))
        );
    }

    #[test]
    fn cyrano_timeout_terminates() {
        let mut pipeline = pipeline_awaiting_cyrano();
        let actions = pipeline.advance(CliEvent::Timeout);
        assert!(actions.is_empty());
        assert_eq!(pipeline.outcome(), Some(&MessageOutcome::CyranoTimeout));
    }

    #[test]
    fn events_after_done_are_ignored() {
        let (mut pipeline, _) = MessagePipeline::start(42, "hi");
        pipeline.advance(CliEvent::Timeout);
        // 遲到產物(stale)在終態一律忽略——注入前 drain 是 port 語意,
        // 狀態機這層再保險一次
        let actions =
            pipeline.advance(CliEvent::Artifact(taster_artifact(SAFE_VERDICT)));
        assert!(actions.is_empty());
        assert_eq!(pipeline.outcome(), Some(&MessageOutcome::TasterTimeout));
    }

    #[test]
    fn poll_backoff_doubles_and_caps() {
        assert_eq!(poll_backoff(1), Duration::from_secs(1));
        assert_eq!(poll_backoff(2), Duration::from_secs(2));
        assert_eq!(poll_backoff(4), Duration::from_secs(8));
        assert_eq!(poll_backoff(7), Duration::from_secs(64));
        assert_eq!(poll_backoff(100), Duration::from_secs(64));
        assert_eq!(poll_backoff(0), Duration::from_secs(1)); // 防呆
    }
}
