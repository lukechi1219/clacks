//! Clock adapter: 生產時鐘。消費端(timeout 記帳、GUI 狀態列)於 Phase 4 接上

use crate::ports::Clock;
use std::time::SystemTime;

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}
