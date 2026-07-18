//! Phase 2 smoke:與骨架同款 echo 管線,但全程走 ports + adapters。
//! 驗證 adapter 忠實搬運骨架行為(尤其 env_clear 最小環境下 claude 仍可啟動)。
//! 從 repo root 執行(../clacks-runtime 相對路徑才會對):
//!   CLACKS_BOT_TOKEN=$(security find-generic-password -s clacks-bot -w) \
//!     cargo run --manifest-path src-tauri/Cargo.toml --bin smoke

use clacks::adapters::pty::ClaudePtySession;
use clacks::adapters::telegram::{next_offset, TelegramHttp};
use clacks::ports::{CliSession, TelegramGateway};
use std::sync::mpsc;
use std::time::Duration;

fn main() {
    // 工作目錄在 repo 外:嵌套在 repo 內會被祖先 CLAUDE.md 污染角色(骨架實證)
    let runtime = std::path::Path::new("../clacks-runtime/echo");
    std::fs::remove_dir_all(runtime.join("outbox")).ok();

    let tg = TelegramHttp::from_env();
    // webhook 互斥檢查(骨架實證:掛著 webhook 時 getUpdates 必 409)
    match tg.webhook_url() {
        Ok(None) => {}
        Ok(Some(url)) => {
            eprintln!("[smoke] webhook active ({url}) — getUpdates 會 409,先解除 webhook");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("[smoke] webhook check failed: {e:?}");
            std::process::exit(1);
        }
    }

    let mut cli = ClaudePtySession::spawn(runtime, Box::new(std::io::stdout()))
        .expect("spawn claude");

    println!("\n[smoke] waiting 15s for CLI boot");
    std::thread::sleep(Duration::from_secs(15));

    let (msg_tx, msg_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let poller = TelegramHttp::from_env();
        let mut offset = 0i64;
        loop {
            // 最小生存迴圈(骨架護欄豁免沿用):os 53 為本環境系統性現象,
            // 固定 3s 重拉、無退避——政策仍留給 Phase 3 orchestrator
            let updates = match poller.poll_updates(offset) {
                Ok(updates) => updates,
                Err(e) => {
                    println!("\n[smoke] poll error, retry in 3s: {e:?}");
                    std::thread::sleep(Duration::from_secs(3));
                    continue;
                }
            };
            offset = next_offset(&updates, offset);
            for update in updates {
                if let Some(message) = update.message {
                    if let Some(text) = message.text {
                        let _ = msg_tx.send((message.chat_id, text));
                    }
                }
            }
        }
    });

    for (chat_id, text) in msg_rx {
        if text.starts_with('/') {
            // 控制指令分流(port 語意驗證點):不等產物——骨架在這裡會空等 120s
            println!("\n[smoke] chat {chat_id} -> control {text}");
            cli.inject_control(&text).expect("inject control");
            let _ = tg.send_reply(chat_id, &format!("[smoke] control injected: {text}"));
            continue;
        }
        println!("\n[smoke] chat {chat_id} -> inject");
        cli.inject_message(&text).expect("inject message");
        match cli.wait_artifact(Duration::from_secs(120)) {
            Ok(artifact) => {
                let value: serde_json::Value =
                    serde_json::from_str(&artifact.raw).expect("artifact json");
                let reply = value["text"].as_str().unwrap_or("(empty)");
                let _ = tg.send_reply(chat_id, reply);
            }
            Err(e) => {
                let _ = tg.send_reply(chat_id, &format!("[smoke] no artifact: {e:?}"));
            }
        }
    }
}
