use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Update {
    pub update_id: i64,
    pub message: Option<Message>,
}

#[derive(Deserialize, Debug)]
pub struct Message {
    pub chat: Chat,
    pub text: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct Chat {
    pub id: i64,
}

#[derive(Deserialize)]
struct UpdatesResponse {
    result: Vec<Update>,
}

pub fn next_offset(updates: &[Update], current: i64) -> i64 {
    updates.iter().map(|u| u.update_id + 1).max().unwrap_or(current)
}

pub struct TelegramClient {
    token: String,
    http: reqwest::blocking::Client,
}

impl TelegramClient {
    pub fn from_env() -> Self {
        let token = std::env::var("CLACKS_BOT_TOKEN").expect("CLACKS_BOT_TOKEN not set");
        let http = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(40))
            .build()
            .expect("build http client");
        Self { token, http }
    }

    // 回傳 Result:真機發現本環境長連線常見瞬時 ECONNABORTED(os 53),
    // panic 會殺死 poller 執行緒並靜默終結整條管線;錯誤交由 caller 決定重試
    pub fn get_updates(&self, offset: i64) -> Result<Vec<Update>, reqwest::Error> {
        let url = format!("https://api.telegram.org/bot{}/getUpdates", self.token);
        let resp: UpdatesResponse = self
            .http
            .get(&url)
            .query(&[("offset", offset.to_string()), ("timeout", "30".to_string())])
            .send()
            .map_err(reqwest::Error::without_url)? // 錯誤訊息的 URL 含 token,傳出前必先去除
            .json()
            .map_err(reqwest::Error::without_url)?;
        Ok(resp.result)
    }

    pub fn send_message(&self, chat_id: i64, text: &str) {
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.token);
        self.http
            .post(&url)
            .form(&[("chat_id", chat_id.to_string()), ("text", text.to_string())])
            .send()
            .map_err(reqwest::Error::without_url)
            .expect("sendMessage request");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
