use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardFormat {
    PlainText,
    HtmlRichText,
    PngImage,
    JpegImage,
    CodeSnippet { language_hint: String },
}

pub fn classify_clipboard_bytes(data: &[u8]) -> ClipboardFormat {
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return ClipboardFormat::PngImage;
    }
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return ClipboardFormat::JpegImage;
    }
    if let Ok(text) = std::str::from_utf8(data) {
        let trimmed = text.trim_start();
        if trimmed.starts_with('<') && (trimmed.contains("</html>") || trimmed.contains("</div>")) {
            return ClipboardFormat::HtmlRichText;
        }
        if trimmed.contains("fn ") && trimmed.contains('{') {
            return ClipboardFormat::CodeSnippet {
                language_hint: "rust".into(),
            };
        }
        if trimmed.contains("func ") && trimmed.contains('{') {
            return ClipboardFormat::CodeSnippet {
                language_hint: "go".into(),
            };
        }
        if trimmed.contains("def ") && trimmed.contains(':') {
            return ClipboardFormat::CodeSnippet {
                language_hint: "python".into(),
            };
        }
    }
    ClipboardFormat::PlainText
}
