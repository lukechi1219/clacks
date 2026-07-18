//! orchestrator:core 狀態機的解譯器。只依賴 core + ports(architecture.md
//! 依賴規則 2)——把 Action 映射成 port 呼叫、把 port 結果映射回 CliEvent。
//! 政策(timeout 值、控制緩衝、退避)由 core 常數/函式供給,IO 由 ports 供給,
//! 本檔只剩接線;正確性以 fake ports 整合測試覆蓋(tests/orchestrator.rs)。

use crate::core::pipeline::{
    poll_backoff, Action, AwaitTarget, CliEvent, MessageOutcome, MessagePipeline,
};
use crate::core::session;
use crate::ports::{
    CliError, CliSession, GatewayError, MessageStore, TelegramGateway, Update, WaitError,
};
use std::time::Duration;

pub struct PipelineConfig {
    pub artifact_timeout: Duration,
    pub control_buffer: Duration,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            artifact_timeout: session::ARTIFACT_TIMEOUT,
            control_buffer: session::CONTROL_BUFFER,
        }
    }
}

pub struct Orchestrator<'a> {
    gateway: &'a dyn TelegramGateway,
    taster: &'a mut dyn CliSession,
    cyrano: &'a mut dyn CliSession,
    store: &'a mut dyn MessageStore,
    config: PipelineConfig,
    /// 注入的 sleep(測試用記錄替身;生產給 std::thread::sleep)。
    /// 不設第 5 個 port——睡眠不是領域介面,是解譯器的執行細節
    sleep: Box<dyn FnMut(Duration) + 'a>,
}

enum ExecError {
    /// CLI 注入失敗(session 視同不可用)
    Cli,
    /// 回覆送出失敗
    Send(String),
}

impl<'a> Orchestrator<'a> {
    pub fn new(
        gateway: &'a dyn TelegramGateway,
        taster: &'a mut dyn CliSession,
        cyrano: &'a mut dyn CliSession,
        store: &'a mut dyn MessageStore,
        config: PipelineConfig,
        sleep: Box<dyn FnMut(Duration) + 'a>,
    ) -> Self {
        Self { gateway, taster, cyrano, store, config, sleep }
    }

    /// 一則 update 走完整管線。None = update_id 已見過(去重跳過)
    pub fn process_update(&mut self, update: &Update) -> Option<MessageOutcome> {
        match self.store.first_seen(update.update_id) {
            // fail-closed:去重狀態不明時不處理(重覆回覆的風險大於漏回)
            Err(error) => return Some(MessageOutcome::StoreFailed(error.0)),
            Ok(false) => return None,
            Ok(true) => {}
        }
        let Some(message) = update.message.as_ref() else {
            return Some(MessageOutcome::SkippedNonText);
        };
        let Some(text) = message.text.as_deref() else {
            // 非文字訊息政策(Phase 3 最小版):跳過不回覆、去重已記錄。
            // 制式回覆 / 取 caption 是 Phase 4+ 設計項(findings:nexus 非文字盲點)
            return Some(MessageOutcome::SkippedNonText);
        };

        let (mut pipeline, mut actions) = MessagePipeline::start(message.chat_id, text);
        loop {
            let mut cli_failed = false;
            for action in std::mem::take(&mut actions) {
                match self.exec(action) {
                    Ok(()) => {}
                    Err(ExecError::Send(error)) => {
                        return Some(MessageOutcome::SendFailed(error));
                    }
                    Err(ExecError::Cli) => {
                        cli_failed = true;
                        break;
                    }
                }
            }
            if cli_failed {
                actions = pipeline.advance(CliEvent::Lost);
                continue;
            }
            if let Some(outcome) = pipeline.outcome() {
                return Some(outcome.clone());
            }
            let target = pipeline.awaiting().expect("非終態必有等待對象");
            let event = self.wait_on(target);
            actions = pipeline.advance(event);
        }
    }

    /// 一輪 poll:取 updates、逐則處理、回 (下一個 offset, 各則結果)。
    /// 瞬時網路錯誤如實上報——重試/退避是 run_forever 的職責。
    /// offset 計算與 adapters::telegram::next_offset 同構但獨立實作:
    /// orchestrator 不得 import adapter(依賴規則);smoke bin 的 helper
    /// 於 Phase 4 接線時退役
    pub fn poll_once(
        &mut self,
        offset: i64,
    ) -> Result<(i64, Vec<MessageOutcome>), GatewayError> {
        let updates = self.gateway.poll_updates(offset)?;
        let mut next_offset = offset;
        let mut outcomes = Vec::new();
        for update in &updates {
            next_offset = next_offset.max(update.update_id + 1);
            if let Some(outcome) = self.process_update(update) {
                outcomes.push(outcome);
            }
        }
        Ok((next_offset, outcomes))
    }

    /// 常駐迴圈:poller 永不靜默死亡(骨架 os 53 實證——thread panic 曾讓
    /// 管線無聲終結)。每次失敗都記錄 + 指數退避,永遠重試。
    /// 無法整合測試(不返回);邏輯全數委派給已測的 poll_once 與 poll_backoff
    pub fn run_forever(&mut self, mut offset: i64) -> ! {
        let mut consecutive_errors = 0u32;
        loop {
            match self.poll_once(offset) {
                Ok((next_offset, outcomes)) => {
                    consecutive_errors = 0;
                    offset = next_offset;
                    for outcome in &outcomes {
                        eprintln!("[clacks] 訊息結果:{outcome:?}");
                    }
                }
                Err(error) => {
                    consecutive_errors += 1;
                    let delay = poll_backoff(consecutive_errors);
                    eprintln!(
                        "[clacks] poll 失敗(連續第 {consecutive_errors} 次):{};{delay:?} 後重試",
                        error.0
                    );
                    (self.sleep)(delay);
                }
            }
        }
    }

    fn exec(&mut self, action: Action) -> Result<(), ExecError> {
        match action {
            Action::InjectTaster(text) => {
                self.taster.inject_message(&text).map_err(cli_failure)
            }
            Action::ClearTaster => {
                self.taster.inject_control("/clear").map_err(cli_failure)?;
                // smoke 實證競態的落點(Global Constraints 5):控制指令處理
                // 期間注入會被 TUI 丟棄——強制緩衝後才允許下一次注入
                (self.sleep)(self.config.control_buffer);
                Ok(())
            }
            Action::InjectCyrano(text) => {
                self.cyrano.inject_message(&text).map_err(cli_failure)
            }
            Action::SendReply { chat_id, text } => self
                .gateway
                .send_reply(chat_id, &text)
                .map_err(|error| ExecError::Send(error.0)),
        }
    }

    fn wait_on(&mut self, target: AwaitTarget) -> CliEvent {
        // 顯式 reborrow(&mut *):不從 &mut self 把 &'a mut 欄位 move 出來
        let session: &mut dyn CliSession = match target {
            AwaitTarget::Taster => &mut *self.taster,
            AwaitTarget::Cyrano => &mut *self.cyrano,
        };
        match session.wait_artifact(self.config.artifact_timeout) {
            Ok(artifact) => CliEvent::Artifact(artifact.raw),
            Err(WaitError::Timeout) => CliEvent::Timeout,
            Err(WaitError::Disconnected | WaitError::Io(_)) => CliEvent::Lost,
        }
    }
}

fn cli_failure(error: CliError) -> ExecError {
    // CliError 字串由 port 契約保證無 token;先 eprintln 供觀測(GUI 期換事件流)
    eprintln!("[clacks] CLI 注入失敗:{}", error.0);
    ExecError::Cli
}
