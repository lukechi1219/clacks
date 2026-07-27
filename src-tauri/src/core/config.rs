//! clacks.config.json 的剖析與驗證(純函式,零 IO——architecture.md 依賴規則 1)。
//! 讀檔、路徑解析與 .claude 注入面防護在 adapters/config_file.rs。
//!
//! ⚠ 本結構只放非機密設定。bot token 只經 CLACKS_BOT_TOKEN 環境變數進入 Rust,
//! 絕不落檔(設計文件既有規則:不用 .env)——不得在此新增任何 credential 欄位

use serde_json::Value;

#[derive(Debug, PartialEq)]
pub struct ClacksConfig {
    /// cyrano 的唯讀專案目錄(未設 = 無額外讀取範圍)。原始字串,尚未解析成路徑
    pub cyrano_readable_dir: Option<String>,
}

#[derive(Debug, PartialEq)]
pub struct ConfigError(pub String);

/// 刻意用 serde_json::Value 逐鍵取值,而非 derive(Deserialize):
/// (1) 要讓未知欄位被忽略而非報錯;(2) 要給出指名道姓的錯誤訊息
/// (serde 的預設訊息對非工程師使用者無用)
pub fn parse_config(json: &str) -> Result<ClacksConfig, ConfigError> {
    let value: Value = serde_json::from_str(json)
        .map_err(|error| ConfigError(format!("JSON 剖析失敗:{error}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| ConfigError("設定檔最外層必須是 JSON 物件".to_string()))?;

    let cyrano_readable_dir = match object.get("cyrano_readable_dir") {
        None | Some(Value::Null) => None,
        Some(Value::String(raw)) => {
            if raw.trim().is_empty() {
                return Err(ConfigError(
                    "cyrano_readable_dir 不得為空字串;不需要唯讀目錄請整個移除此鍵".to_string(),
                ));
            }
            Some(raw.clone())
        }
        Some(other) => {
            return Err(ConfigError(format!(
                "cyrano_readable_dir 必須是字串,實際是 {other}"
            )));
        }
    };

    Ok(ClacksConfig { cyrano_readable_dir })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_cyrano_readable_dir() {
        assert_eq!(
            parse_config(r#"{"cyrano_readable_dir": "../clacks-runtime/cyrano-readable"}"#),
            Ok(ClacksConfig {
                cyrano_readable_dir: Some("../clacks-runtime/cyrano-readable".to_string())
            })
        );
    }

    #[test]
    fn missing_key_means_no_readable_dir() {
        assert_eq!(parse_config("{}"), Ok(ClacksConfig { cyrano_readable_dir: None }));
    }

    #[test]
    fn null_value_means_no_readable_dir() {
        assert_eq!(
            parse_config(r#"{"cyrano_readable_dir": null}"#),
            Ok(ClacksConfig { cyrano_readable_dir: None })
        );
    }

    // 空字串無從區分「不想設」與「設錯」;不想設就整個移除該鍵(fail-closed)
    #[test]
    fn empty_string_is_rejected() {
        assert!(parse_config(r#"{"cyrano_readable_dir": ""}"#).is_err());
    }

    #[test]
    fn whitespace_only_string_is_rejected() {
        assert!(parse_config(r#"{"cyrano_readable_dir": "   "}"#).is_err());
    }

    #[test]
    fn wrong_type_is_rejected() {
        assert!(parse_config(r#"{"cyrano_readable_dir": 42}"#).is_err());
        assert!(parse_config(r#"{"cyrano_readable_dir": ["a"]}"#).is_err());
        assert!(parse_config(r#"{"cyrano_readable_dir": {"a": 1}}"#).is_err());
    }

    #[test]
    fn malformed_json_is_rejected() {
        assert!(parse_config("{not json").is_err());
        assert!(parse_config("").is_err());
    }

    #[test]
    fn non_object_root_is_rejected() {
        assert!(parse_config("[]").is_err());
        assert!(parse_config(r#""just a string""#).is_err());
    }

    // 額外欄位刻意忽略:未來新增設定項時,舊版二進位不會因為看到新鍵就拒絕啟動
    #[test]
    fn unknown_fields_are_ignored() {
        assert_eq!(
            parse_config(r#"{"cyrano_readable_dir": "../x", "future_option": true}"#),
            Ok(ClacksConfig { cyrano_readable_dir: Some("../x".to_string()) })
        );
    }

    // 錯誤訊息必須指出是哪個鍵出問題,否則使用者得靠猜的
    #[test]
    fn error_message_names_the_key() {
        let ConfigError(message) = parse_config(r#"{"cyrano_readable_dir": 42}"#).unwrap_err();
        assert!(message.contains("cyrano_readable_dir"), "訊息未指出鍵名:{message}");
    }
}
