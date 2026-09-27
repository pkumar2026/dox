//! Contents of the log detail popup (Enter on a log line) and the text its
//! copy keys put on the clipboard.

use crate::insights::parse::parse_line;
use crate::insights::severity::Severity;
use crate::ui::logs::timestamp_prefix_len;

#[derive(Debug, Clone, PartialEq)]
pub struct Detail {
    pub time: String,
    pub severity: Severity,
    pub service: Option<String>,
    pub host: Option<String>,
    pub message: String,
    pub attributes: Vec<(String, String)>,
    /// The line body pretty-printed, when it is JSON.
    pub pretty_json: Option<String>,
    pub raw: String,
}

pub fn detail(raw: &str) -> Detail {
    let ts = timestamp_prefix_len(raw);
    let body = raw[ts..].trim();
    let parsed = parse_line(raw);
    let pretty_json = body
        .starts_with('{')
        .then(|| serde_json::from_str::<serde_json::Value>(body).ok())
        .flatten()
        .and_then(|v| serde_json::to_string_pretty(&v).ok());
    Detail {
        time: raw[..ts].trim().to_string(),
        severity: parsed.severity,
        service: parsed.service,
        host: parsed.host,
        message: parsed.message,
        attributes: parsed.attributes,
        pretty_json,
        raw: raw.to_string(),
    }
}

/// Ctrl+y text: every field of the entry, one per line.
pub fn clipboard_text(d: &Detail) -> String {
    let mut lines = vec![
        format!("Time: {}", d.time),
        format!("Severity: {}", d.severity.label()),
    ];
    if let Some(service) = &d.service {
        lines.push(format!("Service: {service}"));
    }
    if let Some(host) = &d.host {
        lines.push(format!("Host: {host}"));
    }
    lines.push(format!("Message: {}", d.message));
    if !d.attributes.is_empty() {
        lines.push("Attributes:".to_string());
        lines.extend(d.attributes.iter().map(|(k, v)| format!("  {k} = {v}")));
    }
    lines.push(format!("Raw: {}", d.raw));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_line_detail() {
        let d =
            detail("2026-09-24T15:12:57.1Z [error    ] request failed method=POST status_code=500");
        assert_eq!(d.time, "2026-09-24T15:12:57.1Z");
        assert_eq!(d.severity, Severity::Error);
        assert!(d.message.starts_with("request failed"));
        assert_eq!(d.attributes.len(), 2);
        assert!(d.pretty_json.is_none());
    }

    #[test]
    fn json_line_is_pretty_printed() {
        let d = detail(r#"2026-09-24T15:12:57.1Z {"level":"info","msg":"hi","service":"api"}"#);
        assert_eq!(d.service.as_deref(), Some("api"));
        let pretty = d.pretty_json.expect("json");
        assert!(pretty.contains("\n  \"msg\": \"hi\""), "{pretty}");
    }

    #[test]
    fn clipboard_text_lists_every_field() {
        let d = detail("2026-09-24T15:12:57.1Z ERROR boom code=7");
        let text = clipboard_text(&d);
        for part in [
            "Time: 2026-09-24T15:12:57.1Z",
            "Severity: ERROR",
            "Message: boom code=7",
            "code = 7",
            "Raw: ",
        ] {
            assert!(text.contains(part), "missing {part:?} in {text}");
        }
    }
}
