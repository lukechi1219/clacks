mod outbox;
mod pty;
mod pty_input;
mod telegram;

use std::io::Write;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

fn main() {
    // 工作目錄在 repo 外:嵌套在 repo 內會被祖先 CLAUDE.md 污染角色(見 skeleton findings)
    std::fs::remove_dir_all("../clacks-runtime/echo/outbox").ok();

    let tg = telegram::TelegramClient::from_env();
    let mut cli = pty::spawn_claude("../clacks-runtime/echo");

    let (outbox_tx, outbox_rx) = mpsc::channel();
    let _watcher = outbox::watch_outbox(Path::new("../clacks-runtime/echo/outbox"), outbox_tx);

    println!("\n[skeleton] waiting 15s for CLI boot");
    std::thread::sleep(Duration::from_secs(15));

    let (msg_tx, msg_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let poller = telegram::TelegramClient::from_env();
        let mut offset = 0i64;
        loop {
            // 最小生存迴圈(護欄豁免,見 findings):本環境長連線常見瞬時 os 53,
            // 不重拉則 poller 一死全管線靜默終結。固定 3s、無退避——政策仍留給 Phase 2
            let updates = match poller.get_updates(offset) {
                Ok(updates) => updates,
                Err(e) => {
                    println!("\n[skeleton] poll error, retry in 3s: {e}");
                    std::thread::sleep(Duration::from_secs(3));
                    continue;
                }
            };
            offset = telegram::next_offset(&updates, offset);
            for u in updates {
                if let Some(m) = u.message {
                    if let Some(text) = m.text {
                        let _ = msg_tx.send((m.chat.id, text));
                    }
                }
            }
        }
    });

    for (chat_id, text) in msg_rx {
        println!("\n[skeleton] chat {chat_id} -> inject");
        cli.writer
            .write_all(&pty_input::bracketed_paste(&text))
            .expect("pty write");
        cli.writer.flush().expect("pty flush");
        // 真機發現:\r 與 paste 信封同寫不觸發送出,需延遲後單獨送
        std::thread::sleep(Duration::from_millis(150));
        cli.writer.write_all(b"\r").expect("pty write cr");
        cli.writer.flush().expect("pty flush cr");

        match outbox_rx.recv_timeout(Duration::from_secs(120)) {
            Ok(path) => {
                std::thread::sleep(Duration::from_millis(200)); // hook 以 > redirect 寫檔,Create 事件可能早於內容寫完(骨架級簡化)
                let raw = std::fs::read_to_string(&path).expect("read outbox file");
                let v: serde_json::Value = serde_json::from_str(&raw).expect("outbox json");
                let reply = v["text"].as_str().unwrap_or("(empty)");
                tg.send_message(chat_id, reply);
            }
            Err(_) => tg.send_message(chat_id, "[skeleton] timeout"),
        }
    }
}
