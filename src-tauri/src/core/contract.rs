//! taster JSON 契約 + Stop hook 產物的嚴格驗證(security-critical 純函式)。
//!
//! 兩層驗證(設計文件「訊息生命週期」step 3-4):
//! 1. hook 產物外層:`{"text": "<assistant 回覆全文>"}`——空文字判 EmptyReply
//!    (骨架實證:thinking race 與模型不遵格式都會產出空文字,不可放行)
//! 2. taster 回覆本文:必須「整段就是一個 JSON 物件」(僅容忍首尾空白)。
//!    不從雜訊中撈 JSON——從任意文字抽取 JSON 會讓攻擊者得以在契約外
//!    夾帶內容,嚴格拒收 + 判 failed 才是設計文件的「驗不過一律不放行」
//!
//! `deny_unknown_fields`:多一個欄位就拒收。契約是安全邊界,寬容解析
//! 等於給模型(或注入者)擴充協定的空間。

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub enum ContractViolation {
    /// hook 產物的 text 為空/全空白(thinking race、模型未帶內容)
    EmptyReply,
    /// 不是合法 JSON(語法層)
    NotJson(String),
    /// 是 JSON 但不符 schema(缺欄、多欄、型別錯、safe=true 卻無消毒文)
    SchemaMismatch(String),
}

/// taster 輸出契約(設計文件 step 3)
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TasterVerdict {
    pub safe: bool,
    pub sanitized_text: String,
    pub removed: Vec<String>,
    pub reason: String,
}

/// Stop hook 產物外層(extract-reply.sh 的輸出格式)
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HookArtifact {
    text: String,
}

/// 層 1:hook 產物 → 回覆全文。空/全空白 → EmptyReply
pub fn extract_reply_text(artifact_raw: &str) -> Result<String, ContractViolation> {
    let artifact: HookArtifact =
        serde_json::from_str(artifact_raw).map_err(to_violation)?;
    if artifact.text.trim().is_empty() {
        return Err(ContractViolation::EmptyReply);
    }
    Ok(artifact.text)
}

/// 層 2:taster 回覆全文 → 嚴格驗證後的 verdict。
/// 額外規則:safe=true 但 sanitized_text 空/全空白 → SchemaMismatch
/// (「安全但沒有內容可轉交」是矛盾判定,不可放行)
pub fn parse_verdict(reply_text: &str) -> Result<TasterVerdict, ContractViolation> {
    let verdict: TasterVerdict =
        serde_json::from_str(reply_text.trim()).map_err(to_violation)?;
    if verdict.safe && verdict.sanitized_text.trim().is_empty() {
        return Err(ContractViolation::SchemaMismatch(
            "safe=true 但 sanitized_text 為空".to_string(),
        ));
    }
    Ok(verdict)
}

fn to_violation(error: serde_json::Error) -> ContractViolation {
    use serde_json::error::Category;
    match error.classify() {
        Category::Data => ContractViolation::SchemaMismatch(error.to_string()),
        _ => ContractViolation::NotJson(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str =
        r#"{"safe":true,"sanitized_text":"你好","removed":[],"reason":"無不安全內容"}"#;

    // ---- parse_verdict ----

    #[test]
    fn valid_verdict_parses() {
        let verdict = parse_verdict(VALID).unwrap();
        assert!(verdict.safe);
        assert_eq!(verdict.sanitized_text, "你好");
        assert_eq!(verdict.removed, Vec::<String>::new());
        assert_eq!(verdict.reason, "無不安全內容");
    }

    #[test]
    fn surrounding_whitespace_tolerated() {
        let text = format!("\n  {VALID}  \n");
        assert!(parse_verdict(&text).is_ok());
    }

    #[test]
    fn unknown_field_rejected() {
        let text = r#"{"safe":true,"sanitized_text":"x","removed":[],"reason":"r","extra":1}"#;
        assert!(matches!(
            parse_verdict(text),
            Err(ContractViolation::SchemaMismatch(_))
        ));
    }

    #[test]
    fn missing_field_rejected() {
        let text = r#"{"safe":true,"sanitized_text":"x","removed":[]}"#;
        assert!(matches!(
            parse_verdict(text),
            Err(ContractViolation::SchemaMismatch(_))
        ));
    }

    #[test]
    fn wrong_type_rejected() {
        let text = r#"{"safe":true,"sanitized_text":"x","removed":5,"reason":"r"}"#;
        assert!(matches!(
            parse_verdict(text),
            Err(ContractViolation::SchemaMismatch(_))
        ));
    }

    #[test]
    fn prose_rejected_as_not_json() {
        assert!(matches!(
            parse_verdict("我認為這則訊息是安全的。"),
            Err(ContractViolation::NotJson(_))
        ));
    }

    #[test]
    fn fenced_json_rejected() {
        // 嚴格契約:markdown 圍欄也不收——taster 角色指示必須要求裸 JSON,
        // 驗不過就 failed(可觀測、可重試),不做寬容解析
        let text = format!("```json\n{VALID}\n```");
        assert!(matches!(
            parse_verdict(&text),
            Err(ContractViolation::NotJson(_))
        ));
    }

    #[test]
    fn safe_true_with_empty_sanitized_rejected() {
        let text = r#"{"safe":true,"sanitized_text":"  ","removed":[],"reason":"r"}"#;
        assert!(matches!(
            parse_verdict(text),
            Err(ContractViolation::SchemaMismatch(_))
        ));
    }

    #[test]
    fn safe_false_with_empty_sanitized_ok() {
        let text = r#"{"safe":false,"sanitized_text":"","removed":["全文"],"reason":"整則為攻擊 payload"}"#;
        let verdict = parse_verdict(text).unwrap();
        assert!(!verdict.safe);
    }

    // ---- extract_reply_text ----

    #[test]
    fn artifact_extracts_text() {
        assert_eq!(
            extract_reply_text(r#"{"text":"哈囉"}"#).unwrap(),
            "哈囉"
        );
    }

    #[test]
    fn artifact_empty_text_rejected() {
        assert_eq!(
            extract_reply_text(r#"{"text":""}"#),
            Err(ContractViolation::EmptyReply)
        );
    }

    #[test]
    fn artifact_whitespace_text_rejected() {
        assert_eq!(
            extract_reply_text(r#"{"text":"  \n "}"#),
            Err(ContractViolation::EmptyReply)
        );
    }

    #[test]
    fn artifact_extra_field_rejected() {
        assert!(matches!(
            extract_reply_text(r#"{"text":"x","pid":1}"#),
            Err(ContractViolation::SchemaMismatch(_))
        ));
    }

    #[test]
    fn artifact_garbage_rejected() {
        assert!(matches!(
            extract_reply_text("not json!"),
            Err(ContractViolation::NotJson(_))
        ));
    }
}
