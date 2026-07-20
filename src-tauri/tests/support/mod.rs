//! Fake ports:orchestrator 測試替身(architecture.md port 表「測試替身」欄)。
//! 各整合測試 crate 以 `mod support;` 各自編譯一份;selftest 見 fakes_selftest.rs
#![allow(dead_code)] // 各測試 crate 只用到部分替身

use clacks::ports::{
    Artifact, CliError, CliSession, Clock, GatewayError, IncomingMessage, MessageStore,
    StoreError, TelegramGateway, Update, WaitError,
};
use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

// ---------- Telegram ----------

#[derive(Default)]
pub struct FakeGateway {
    pub poll_script: RefCell<VecDeque<Result<Vec<Update>, GatewayError>>>,
    pub polled_offsets: RefCell<Vec<i64>>,
    pub sent: RefCell<Vec<(i64, String)>>,
    /// Some 時下一次 send_reply 失敗(單發)
    pub send_error: RefCell<Option<String>>,
}

impl FakeGateway {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn script_poll(&self, result: Result<Vec<Update>, GatewayError>) {
        self.poll_script.borrow_mut().push_back(result);
    }
}

impl TelegramGateway for FakeGateway {
    fn poll_updates(&self, offset: i64) -> Result<Vec<Update>, GatewayError> {
        self.polled_offsets.borrow_mut().push(offset);
        self.poll_script
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Ok(vec![]))
    }

    fn send_reply(&self, chat_id: i64, text: &str) -> Result<(), GatewayError> {
        if let Some(message) = self.send_error.borrow_mut().take() {
            return Err(GatewayError(message));
        }
        self.sent.borrow_mut().push((chat_id, text.to_string()));
        Ok(())
    }

    fn webhook_url(&self) -> Result<Option<String>, GatewayError> {
        Ok(None)
    }
}

// ---------- CLI ----------

pub struct ScriptedCli {
    pub artifacts: VecDeque<Result<Artifact, WaitError>>,
    pub messages: Vec<String>,
    pub controls: Vec<String>,
    pub raw_writes: Vec<Vec<u8>>,
    /// true 時下一次 inject_message 失敗(單發)
    pub fail_next_inject: bool,
    /// respawn 呼叫次數(安全義務測試斷言用)
    pub respawns: u32,
    /// >0 時下 N 次 respawn 失敗(每次呼叫遞減);耗盡後恢復正常 respawn
    pub fail_respawns: u32,
    /// wait_idle 呼叫次數(注入前 idle 編排測試斷言用)
    pub idle_waits: u32,
    /// >0 時下 N 次 wait_idle 回 Timeout(測 best-effort 續注入路徑);耗盡後正常
    pub idle_timeouts: u32,
}

impl ScriptedCli {
    pub fn new(artifacts: Vec<Result<Artifact, WaitError>>) -> Self {
        Self {
            artifacts: artifacts.into(),
            messages: vec![],
            controls: vec![],
            raw_writes: vec![],
            fail_next_inject: false,
            respawns: 0,
            fail_respawns: 0,
            idle_waits: 0,
            idle_timeouts: 0,
        }
    }
}

impl CliSession for ScriptedCli {
    fn inject_message(&mut self, text: &str) -> Result<(), CliError> {
        if self.fail_next_inject {
            self.fail_next_inject = false;
            return Err(CliError("scripted inject failure".to_string()));
        }
        self.messages.push(text.to_string());
        Ok(())
    }

    fn inject_control(&mut self, command: &str) -> Result<(), CliError> {
        self.controls.push(command.to_string());
        Ok(())
    }

    fn wait_artifact(&mut self, _timeout: Duration) -> Result<Artifact, WaitError> {
        // 腳本耗盡 = 沒有產物 = timeout(與真 adapter 的等待語意一致)
        self.artifacts.pop_front().unwrap_or(Err(WaitError::Timeout))
    }

    fn write_raw(&mut self, bytes: &[u8]) -> Result<(), CliError> {
        self.raw_writes.push(bytes.to_vec());
        Ok(())
    }

    fn respawn(&mut self) -> Result<(), CliError> {
        if self.fail_respawns > 0 {
            self.fail_respawns -= 1;
            return Err(CliError("scripted respawn failure".to_string()));
        }
        self.respawns += 1;
        // 替身的「全新 session」語意:清空已注入紀錄(消毒者無記憶 = 乾淨)
        self.messages.clear();
        self.controls.clear();
        Ok(())
    }

    fn wait_idle(&mut self, _quiet_for: Duration, _timeout: Duration) -> Result<(), WaitError> {
        self.idle_waits += 1;
        if self.idle_timeouts > 0 {
            self.idle_timeouts -= 1;
            return Err(WaitError::Timeout);
        }
        Ok(())
    }
}

/// 便利建構:hook 產物(path 不參與語意,只有 raw 重要)
pub fn ok_artifact(raw: &str) -> Result<Artifact, WaitError> {
    Ok(Artifact { path: PathBuf::from("fake-artifact.json"), raw: raw.to_string() })
}

// ---------- Store / Clock ----------

#[derive(Default)]
pub struct InMemoryStore {
    seen: HashSet<i64>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl MessageStore for InMemoryStore {
    fn first_seen(&mut self, update_id: i64) -> Result<bool, StoreError> {
        Ok(self.seen.insert(update_id))
    }
}

/// 去重狀態不可得時 orchestrator 必須 fail-closed(不處理)——供該測試使用
pub struct FailingStore;

impl MessageStore for FailingStore {
    fn first_seen(&mut self, _update_id: i64) -> Result<bool, StoreError> {
        Err(StoreError("scripted store failure".to_string()))
    }
}

pub struct ManualClock {
    now: RefCell<SystemTime>,
}

impl ManualClock {
    pub fn new(start: SystemTime) -> Self {
        Self { now: RefCell::new(start) }
    }

    pub fn advance(&self, delta: Duration) {
        let mut now = self.now.borrow_mut();
        *now += delta;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> SystemTime {
        *self.now.borrow()
    }
}

// ---------- 共用測試素材 ----------

/// 把 taster 的 verdict JSON 包成 hook 產物 {"text": "<json>"}(嵌套跳脫交給 serde)
pub fn taster_artifact(verdict_json: &str) -> String {
    serde_json::json!({ "text": verdict_json }).to_string()
}

pub fn text_update(update_id: i64, chat_id: i64, text: &str) -> Update {
    Update {
        update_id,
        message: Some(IncomingMessage { chat_id, text: Some(text.to_string()) }),
    }
}

/// 記錄型 sleeper:回傳 (紀錄, closure);closure 交給 Orchestrator::new
pub fn recording_sleeper() -> (Rc<RefCell<Vec<Duration>>>, Box<dyn FnMut(Duration)>) {
    let slept = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&slept);
    (slept, Box::new(move |duration| recorder.borrow_mut().push(duration)))
}
