pub fn bracketed_paste(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() + 12);
    bytes.extend_from_slice(b"\x1b[200~");
    bytes.extend_from_slice(text.as_bytes());
    bytes.extend_from_slice(b"\x1b[201~");
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    // 真機發現(2026-07-17 E2E):\r 緊跟 201~ 同一次寫入不會觸發 TUI 送出,
    // 故本函式只產生 paste 信封;\r 由 caller 延遲後單獨寫入
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
