use crate::ports::{Artifact, CliError, CliSession, WaitError};
use notify::RecommendedWatcher;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

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
    child: Box<dyn Child + Send + Sync>,
    /// 重啟需要的組態(重啟 = 就地重建同 workdir 的新 session)
    workdir: PathBuf,
    respawn_argv: Vec<String>,
    /// 隔離用 config 目錄(Task 7 裁決:CLAUDE_CONFIG_DIR)。None = 用預設 ~/.claude
    config_dir: Option<PathBuf>,
}

impl ClaudePtySession {
    /// output:PTY 輸出的去向(骨架/smoke 用 stdout;GUI 期換成事件流)。
    /// workdir 必須在 repo 目錄樹外(祖先 CLAUDE.md 污染,骨架實證)——
    /// 呼叫端(composition root)負責給對路徑。重啟仍為純 `claude`(taster 語意)
    pub fn spawn(
        workdir: &Path,
        config_dir: Option<&Path>,
        output: Box<dyn Write + Send>,
    ) -> Result<Self, CliError> {
        Self::spawn_program(&["claude"], vec!["claude".to_string()], workdir, config_dir, output)
    }

    /// cyrano 專用:首次啟動全新對話,重啟以 `claude --continue` 續談。
    /// 真機驗證項(Task 9):`--continue` 能否恢復對話待證實,失敗 fallback = 全新 session
    pub fn spawn_continue(
        workdir: &Path,
        config_dir: Option<&Path>,
        output: Box<dyn Write + Send>,
    ) -> Result<Self, CliError> {
        Self::spawn_program(
            &["claude"],
            vec!["claude".to_string(), "--continue".to_string()],
            workdir,
            config_dir,
            output,
        )
    }

    /// 測試縫:teardown/respawn 測試以 sleep/sh 代替 claude。生產路徑走 spawn/spawn_continue
    fn spawn_program(
        argv: &[&str],
        respawn_argv: Vec<String>,
        workdir: &Path,
        config_dir: Option<&Path>,
        mut output: Box<dyn Write + Send>,
    ) -> Result<Self, CliError> {
        let pty_system = native_pty_system();
        let portable_pty::PtyPair { slave, master } = pty_system
            .openpty(PtySize { rows: 40, cols: 120, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| CliError(e.to_string()))?;

        let mut cmd = CommandBuilder::new(argv[0]);
        cmd.args(&argv[1..]);
        // 一級安全約束落點:清空繼承環境,只放行白名單(token 結構性排除)
        cmd.env_clear();
        for (key, value) in minimal_env(std::env::vars()) {
            cmd.env(key, value);
        }
        // Task 7 裁決:CLAUDE_CONFIG_DIR 隔離 user 層 settings/MCP/plugins。
        // 在 env_clear + 白名單之後顯式設定——受控變數,非繼承(不觸 token 排除紅線)
        if let Some(cfg) = config_dir {
            cmd.env("CLAUDE_CONFIG_DIR", cfg);
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
            child,
            workdir: workdir.to_path_buf(),
            respawn_argv,
            config_dir: config_dir.map(Path::to_path_buf),
        })
    }

    fn write_submit(&mut self, text: &str) -> Result<(), CliError> {
        self.write_raw(&bracketed_paste(text))?;
        // 真機實證:paste 信封與 \r 同寫不送出,需延遲後單獨送
        std::thread::sleep(Duration::from_millis(150));
        self.write_raw(b"\r")
    }

    /// 顯式 teardown(Phase 3 一級任務,使用者裁決):kill + 有界等待 + 升級。
    /// portable-pty 0.9.0 unix kill() 送 SIGHUP——攔截 HUP 的 child 不會死;
    /// Child 亦不保證 kill-on-drop。SIGHUP 未在期限內生效即升級 SIGKILL
    /// (不可攔),wait 收屍避免 zombie
    fn teardown(&mut self) {
        let _ = self.child.kill(); // unix: SIGHUP
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => return, // 已退出並收屍
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => break,
            }
        }
        if let Some(pid) = self.child.process_id() {
            let _ = std::process::Command::new("kill")
                .args(["-9", &pid.to_string()])
                .status();
            let _ = self.child.wait();
        }
    }
}

impl Drop for ClaudePtySession {
    fn drop(&mut self) {
        self.teardown();
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

    fn respawn(&mut self) -> Result<(), CliError> {
        // 先建新 session:成功才替換——失敗則保留舊 session,回 Err 供上層重試
        let respawn_argv = self.respawn_argv.clone();
        let workdir = self.workdir.clone();
        let config_dir = self.config_dir.clone();
        let argv: Vec<&str> = respawn_argv.iter().map(String::as_str).collect();
        let fresh = Self::spawn_program(
            &argv,
            respawn_argv.clone(),
            &workdir,
            config_dir.as_deref(),
            Box::new(std::io::sink()), // 無頭重啟:輸出導向 sink(Phase 5 GUI 改事件流)
        )?;
        // 舊 session 在此被 drop → Phase 3 teardown(kill + 有界等待 + SIGKILL)保證無殘留;
        // 新 session 全新 = 乾淨(teardown 必經 = Global Constraints 3)
        *self = fresh;
        Ok(())
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

    /// pgrep 等價檢查:ps -p 對已 reap 的 pid 回非零 exit code
    fn process_alive(pid: u32) -> bool {
        std::process::Command::new("ps")
            .args(["-p", &pid.to_string()])
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    // teardown 一級任務(2026-07-18 裁決):session 丟棄後不得殘留子行程
    #[test]
    fn drop_kills_child_process() {
        let dir = tempfile::tempdir().unwrap();
        let session = ClaudePtySession::spawn_program(
            &["sleep", "300"],
            vec!["sleep".to_string(), "300".to_string()],
            dir.path(),
            None,
            Box::new(std::io::sink()),
        )
        .unwrap();
        let pid = session.child.process_id().expect("child 應有 pid");
        assert!(process_alive(pid));
        drop(session);
        assert!(!process_alive(pid), "teardown 後不得有殘留行程");
    }

    // Task 10:config_dir = Some 時,子行程環境必須有 CLAUDE_CONFIG_DIR
    #[test]
    fn config_dir_sets_env_for_child() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        let session = ClaudePtySession::spawn_program(
            &["sh", "-c", "printf '%s' \"$CLAUDE_CONFIG_DIR\" > cfg-probe.txt"],
            vec!["sh".to_string()],
            dir.path(),
            Some(cfg.path()),
            Box::new(std::io::sink()),
        )
        .unwrap();
        let probe = dir.path().join("cfg-probe.txt");
        let expected = cfg.path().to_string_lossy().to_string();
        let deadline = Instant::now() + Duration::from_secs(10);
        let got = loop {
            if let Ok(content) = std::fs::read_to_string(&probe) {
                if !content.is_empty() {
                    break content;
                }
            }
            if Instant::now() >= deadline {
                break String::new();
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        drop(session);
        assert_eq!(got, expected);
    }

    // config_dir = None 時,子行程不得有 CLAUDE_CONFIG_DIR(env_clear + 白名單不含它)
    #[test]
    fn no_config_dir_leaves_env_unset() {
        let dir = tempfile::tempdir().unwrap();
        let session = ClaudePtySession::spawn_program(
            &["sh", "-c", "printf 'val=[%s]' \"${CLAUDE_CONFIG_DIR:-UNSET}\" > cfg-probe.txt"],
            vec!["sh".to_string()],
            dir.path(),
            None,
            Box::new(std::io::sink()),
        )
        .unwrap();
        let probe = dir.path().join("cfg-probe.txt");
        let deadline = Instant::now() + Duration::from_secs(10);
        let got = loop {
            if let Ok(content) = std::fs::read_to_string(&probe) {
                if !content.is_empty() {
                    break content;
                }
            }
            if Instant::now() >= deadline {
                break String::new();
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        drop(session);
        assert_eq!(got, "val=[UNSET]");
    }

    // portable-pty 的 kill() 送 SIGHUP(0.9.0 實證)——攔截 HUP 的行程
    // 必須被升級的 SIGKILL 收掉。sh 迴圈裡的 sleep 1 孫行程會在 1s 內
    // 自然退出,不列入斷言
    #[test]
    fn drop_escalates_to_sigkill_when_sighup_trapped() {
        let dir = tempfile::tempdir().unwrap();
        let session = ClaudePtySession::spawn_program(
            &["sh", "-c", "trap '' HUP; while :; do sleep 1; done"],
            vec![
                "sh".to_string(),
                "-c".to_string(),
                "trap '' HUP; while :; do sleep 1; done".to_string(),
            ],
            dir.path(),
            None,
            Box::new(std::io::sink()),
        )
        .unwrap();
        let pid = session.child.process_id().expect("child 應有 pid");
        assert!(process_alive(pid));
        drop(session); // SIGHUP 被攔 → 有界等待 → SIGKILL
        assert!(!process_alive(pid), "SIGHUP 免疫的行程必須被 SIGKILL 收掉");
    }

    // teardown 必經(Global Constraints 3):respawn 必須收掉舊 child 並產生新的
    #[test]
    fn respawn_kills_old_child_and_starts_new() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = ClaudePtySession::spawn_program(
            &["sleep", "300"],
            vec!["sleep".to_string(), "300".to_string()],
            dir.path(),
            None,
            Box::new(std::io::sink()),
        )
        .unwrap();
        let old = session.child.process_id().expect("舊 child pid");
        session.respawn().unwrap();
        let new = session.child.process_id().expect("新 child pid");
        assert_ne!(old, new, "respawn 必須產生新行程");
        assert!(!process_alive(old), "舊 session 必須被 teardown 收掉(pgrep 無殘留)");
        assert!(process_alive(new), "新 session 必須存活");
    }
}
