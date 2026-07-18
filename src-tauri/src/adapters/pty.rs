use crate::ports::{Artifact, CliError, CliSession, WaitError};
use notify::RecommendedWatcher;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use super::outbox::watch_outbox;

/// claude CLI 啟動所需的最小環境白名單。
/// 一級安全需求(骨架實證):CommandBuilder::new 以父行程全環境為基底,
/// CLACKS_BOT_TOKEN 不排除就直接進 CLI 子行程。白名單以外一律不繼承——
/// secret 排除靠結構(env_clear + allowlist),不靠逐項 env_remove
/// (漏列新 secret 即洩漏)。
/// HOME:CLI 讀 ~/.claude(OAuth、設定);PATH:找得到 claude;
/// TERM:TUI 渲染;其餘為 shell 與 locale 慣例最小集
const ENV_ALLOWLIST: &[&str] = &[
    "PATH", "HOME", "TERM", "LANG", "LC_ALL", "USER", "SHELL", "TMPDIR",
];

pub fn minimal_env(
    parent: impl Iterator<Item = (String, String)>,
) -> Vec<(String, String)> {
    parent
        .filter(|(key, _)| ENV_ALLOWLIST.contains(&key.as_str()))
        .collect()
}

/// 真機實證(E2E):\r 緊跟 201~ 同次寫入不觸發 TUI 送出——
/// 本函式只產生 paste 信封,\r 由 caller 延遲後單獨寫
pub fn bracketed_paste(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() + 12);
    bytes.extend_from_slice(b"\x1b[200~");
    bytes.extend_from_slice(text.as_bytes());
    bytes.extend_from_slice(b"\x1b[201~");
    bytes
}

pub struct ClaudePtySession {
    writer: Box<dyn Write + Send>,
    artifact_rx: Receiver<PathBuf>,
    _watcher: RecommendedWatcher,
    _master: Box<dyn MasterPty + Send>,
    _child: Box<dyn Child + Send + Sync>,
}

impl ClaudePtySession {
    /// output:PTY 輸出的去向(骨架/smoke 用 stdout;GUI 期換成事件流)。
    /// workdir 必須在 repo 目錄樹外(祖先 CLAUDE.md 污染,骨架實證)——
    /// 呼叫端(composition root)負責給對路徑
    pub fn spawn(
        workdir: &Path,
        mut output: Box<dyn Write + Send>,
    ) -> Result<Self, CliError> {
        let pty_system = native_pty_system();
        let portable_pty::PtyPair { slave, master } = pty_system
            .openpty(PtySize { rows: 40, cols: 120, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| CliError(e.to_string()))?;

        let mut cmd = CommandBuilder::new("claude");
        // 一級安全約束落點:清空繼承環境,只放行白名單(token 結構性排除)
        cmd.env_clear();
        for (key, value) in minimal_env(std::env::vars()) {
            cmd.env(key, value);
        }
        cmd.cwd(workdir);
        // 不用 --settings:CLI 自動載入 workdir 的 .claude/settings.json,並用會雙重註冊 hook
        let child = slave
            .spawn_command(cmd)
            .map_err(|e| CliError(e.to_string()))?;
        drop(slave); // 骨架教訓:slave 不 drop,child 退出後 drain thread 收不到 EOF

        let mut reader = master
            .try_clone_reader()
            .map_err(|e| CliError(e.to_string()))?;
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let _ = output.write_all(&buf[..n]);
                        let _ = output.flush();
                    }
                }
            }
        });

        let (artifact_tx, artifact_rx) = mpsc::channel();
        let watcher = watch_outbox(&workdir.join("outbox"), artifact_tx)
            .map_err(|e| CliError(e.to_string()))?;

        let writer = master
            .take_writer()
            .map_err(|e| CliError(e.to_string()))?;
        Ok(Self {
            writer,
            artifact_rx,
            _watcher: watcher,
            _master: master,
            _child: child,
        })
    }

    fn write_submit(&mut self, text: &str) -> Result<(), CliError> {
        self.write_raw(&bracketed_paste(text))?;
        // 真機實證:paste 信封與 \r 同寫不送出,需延遲後單獨送
        std::thread::sleep(Duration::from_millis(150));
        self.write_raw(b"\r")
    }
}

impl CliSession for ClaudePtySession {
    fn inject_message(&mut self, text: &str) -> Result<(), CliError> {
        // stale 產物 drain:前一則 timeout 後遲到的產物不得誤配給本則(port 語意)
        while self.artifact_rx.try_recv().is_ok() {}
        self.write_submit(text)
    }

    fn inject_control(&mut self, command: &str) -> Result<(), CliError> {
        // 控制指令不產生 outbox 產物(port 語意):不 drain、caller 不得 wait_artifact
        self.write_submit(command)
    }

    fn wait_artifact(&mut self, timeout: Duration) -> Result<Artifact, WaitError> {
        let path = self.artifact_rx.recv_timeout(timeout).map_err(|e| match e {
            RecvTimeoutError::Timeout => WaitError::Timeout,
            RecvTimeoutError::Disconnected => WaitError::Disconnected,
        })?;
        // hook 契約 rename-into-place:事件到達即內容完整(骨架的 200ms sleep 移除)
        let raw = std::fs::read_to_string(&path).map_err(|e| WaitError::Io(e.to_string()))?;
        Ok(Artifact { path, raw })
    }

    fn write_raw(&mut self, bytes: &[u8]) -> Result<(), CliError> {
        self.writer
            .write_all(bytes)
            .and_then(|()| self.writer.flush())
            .map_err(|e| CliError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 一級安全需求(骨架實證:portable-pty 預設繼承全父環境,token 直入子行程)
    #[test]
    fn minimal_env_excludes_secrets_and_unknowns() {
        let parent = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("CLACKS_BOT_TOKEN".to_string(), "SECRET".to_string()),
            ("HOME".to_string(), "/Users/x".to_string()),
            ("RANDOM_VAR".to_string(), "y".to_string()),
        ];
        let env = minimal_env(parent.into_iter());
        assert!(env.iter().any(|(k, _)| k == "PATH"));
        assert!(env.iter().any(|(k, _)| k == "HOME"));
        assert!(!env.iter().any(|(k, _)| k == "CLACKS_BOT_TOKEN"));
        assert!(!env.iter().any(|(k, _)| k == "RANDOM_VAR"));
    }

    // 真機實證(E2E):\r 緊跟 201~ 同次寫入不觸發 TUI 送出——
    // 信封不含 \r,\r 由 write_submit 延遲後單獨寫
    #[test]
    fn wraps_text_in_bracketed_paste_envelope() {
        let bytes = bracketed_paste("hello\nworld");
        assert_eq!(bytes, b"\x1b[200~hello\nworld\x1b[201~");
    }

    #[test]
    fn empty_text_still_produces_envelope() {
        assert_eq!(bracketed_paste(""), b"\x1b[200~\x1b[201~");
    }
}
