use crate::ports::{GatewayError, IncomingMessage, TelegramGateway, Update};
use serde::Deserialize;

#[derive(Deserialize)]
struct WireUpdate {
    update_id: i64,
    message: Option<WireMessage>,
}

#[derive(Deserialize)]
struct WireMessage {
    chat: WireChat,
    // 已知盲點(findings:nexus 對照):非文字訊息(帶 caption 的照片等)
    // text 為 None,政策留給 Phase 3 taster 管線設計
    text: Option<String>,
}

#[derive(Deserialize)]
struct WireChat {
    id: i64,
}

#[derive(Deserialize)]
struct UpdatesResponse {
    result: Vec<WireUpdate>,
}

#[derive(Deserialize)]
struct WebhookInfoResponse {
    result: WebhookInfo,
}

#[derive(Deserialize)]
struct WebhookInfo {
    #[serde(default)]
    url: String,
}

/// getUpdates 的下一個 offset(純函式;Phase 3 core 成形時搬移)
pub fn next_offset(updates: &[Update], current: i64) -> i64 {
    updates.iter().map(|u| u.update_id + 1).max().unwrap_or(current)
}

pub struct TelegramHttp {
    token: String,
    base_url: String,
    http: reqwest::blocking::Client,
}

impl TelegramHttp {
    pub fn from_env() -> Self {
        let token = std::env::var("CLACKS_BOT_TOKEN").expect("CLACKS_BOT_TOKEN not set");
        Self::new(token, "https://api.telegram.org".to_string())
    }

    /// base_url 可注入:redaction 測試用不可達位址,不打真實 API
    fn new(token: String, base_url: String) -> Self {
        let http = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(40))
            .build()
            .expect("build http client");
        Self { token, base_url, http }
    }

    fn url(&self, method: &str) -> String {
        format!("{}/bot{}/{}", self.base_url, self.token, method)
    }

    /// 一級安全需求(骨架實證:panic 印 URL 洩漏 token,已實際發生並輪替):
    /// 所有 reqwest 錯誤先 without_url 再轉字串,token 不得進任何錯誤訊息
    fn redact(error: reqwest::Error) -> GatewayError {
        GatewayError(error.without_url().to_string())
    }
}

impl TelegramGateway for TelegramHttp {
    fn poll_updates(&self, offset: i64) -> Result<Vec<Update>, GatewayError> {
        let resp: UpdatesResponse = self
            .http
            .get(self.url("getUpdates"))
            .query(&[("offset", offset.to_string()), ("timeout", "30".to_string())])
            .send()
            .map_err(Self::redact)?
            .json()
            .map_err(Self::redact)?;
        Ok(resp
            .result
            .into_iter()
            .map(|u| Update {
                update_id: u.update_id,
                message: u.message.map(|m| IncomingMessage {
                    chat_id: m.chat.id,
                    text: m.text,
                }),
            })
            .collect())
    }

    fn send_reply(&self, chat_id: i64, text: &str) -> Result<(), GatewayError> {
        self.http
            .post(self.url("sendMessage"))
            .form(&[("chat_id", chat_id.to_string()), ("text", text.to_string())])
            .send()
            .map_err(Self::redact)?;
        Ok(())
    }

    fn webhook_url(&self) -> Result<Option<String>, GatewayError> {
        let resp: WebhookInfoResponse = self
            .http
            .get(self.url("getWebhookInfo"))
            .send()
            .map_err(Self::redact)?
            .json()
            .map_err(Self::redact)?;
        Ok(if resp.result.url.is_empty() {
            None
        } else {
            Some(resp.result.url)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::Update;

    fn update(id: i64) -> Update {
        Update { update_id: id, message: None }
    }

    #[test]
    fn advances_past_highest_update_id() {
        assert_eq!(next_offset(&[update(7), update(9), update(8)], 5), 10);
    }

    #[test]
    fn keeps_current_offset_when_no_updates() {
        assert_eq!(next_offset(&[], 5), 5);
    }

    // 一級安全需求(骨架實證:panic 印含 token 的 URL,洩漏實際發生):
    // 三個 API 方法的錯誤都不得含 token。base_url 指向不可達位址,秒級失敗
    #[test]
    fn errors_never_contain_token() {
        let client = TelegramHttp::new(
            "SECRET123TOKEN".to_string(),
            "http://127.0.0.1:9".to_string(),
        );
        let poll_err = client.poll_updates(0).unwrap_err();
        let send_err = client.send_reply(1, "hi").unwrap_err();
        let webhook_err = client.webhook_url().unwrap_err();
        for err in [poll_err, send_err, webhook_err] {
            let shown = format!("{err:?}");
            assert!(!shown.contains("SECRET123TOKEN"), "token leaked: {shown}");
        }
    }

    // 逼出 .json() 解碼錯誤的 redact 分支,connect 階段測不到
    #[test]
    fn decode_errors_never_contain_token() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let addr = listener.local_addr().expect("read local addr");

        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept connection");
                let mut received = Vec::new();
                let mut chunk = [0u8; 512];
                loop {
                    let n = stream.read(&mut chunk).expect("read request");
                    if n == 0 {
                        break;
                    }
                    received.extend_from_slice(&chunk[..n]);
                    if received.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let body = b"not json!";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nconnection: close\r\ncontent-length: {}\r\n\r\n",
                    body.len()
                );
                stream.write_all(response.as_bytes()).expect("write response headers");
                stream.write_all(body).expect("write response body");
                stream.flush().expect("flush response");
            }
        });

        let client = TelegramHttp::new("SECRET123TOKEN".to_string(), format!("http://{addr}"));

        let poll_err = client.poll_updates(0).unwrap_err();
        let webhook_err = client.webhook_url().unwrap_err();

        server.join().expect("stub server thread panicked");

        for err in [poll_err, webhook_err] {
            let shown = format!("{err:?}");
            assert!(!shown.contains("SECRET123TOKEN"), "token leaked: {shown}");
        }
    }
}
