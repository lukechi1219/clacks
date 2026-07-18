use portable_pty::{native_pty_system, Child, CommandBuilder, PtyPair, PtySize};
use std::io::{Read, Write};

pub struct CliPty {
    pub writer: Box<dyn Write + Send>,
    _pair: PtyPair,
    _child: Box<dyn Child + Send + Sync>,
}

pub fn spawn_claude(workdir: &str) -> CliPty {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize { rows: 40, cols: 120, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");

    let mut cmd = CommandBuilder::new("claude");
    // 安全約束:portable-pty 預設繼承整個父環境,bot token 絕不能進 CLI 子行程
    cmd.env_remove("CLACKS_BOT_TOKEN");
    cmd.cwd(workdir);
    // 不用 --settings:CLI 自動載入 workdir 的 .claude/settings.json,並用會雙重註冊 hook
    let child = pair.slave.spawn_command(cmd).expect("spawn claude");
    // 骨架簡化:pair.slave 未 drop,drain thread 在 child 退出後收不到 EOF(Ctrl-C 結束無害);
    // Phase 2 收割成 adapter 時必須 spawn 後 drop slave

    let mut reader = pair.master.try_clone_reader().expect("clone pty reader");
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut out = std::io::stdout().lock();
                    let _ = out.write_all(&buf[..n]);
                    let _ = out.flush();
                }
            }
        }
    });

    let writer = pair.master.take_writer().expect("take pty writer");
    CliPty { writer, _pair: pair, _child: child }
}
