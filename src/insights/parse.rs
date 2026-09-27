//! Turn one raw Docker log line into the parts the insights count: message,
//! severity, `key=value` / JSON fields, and the service and host if present.

use serde_json::{Map, Value};

use super::severity::Severity;
use crate::ui::logs::{detect_level, timestamp_prefix_len, LogLevel};

const LEVEL_KEYS: &[&str] = &[
    "level",
    "severity",
    "lvl",
    "levelname",
    "loglevel",
    "log.level",
    "severity_text",
];
const MESSAGE_KEYS: &[&str] = &["msg", "message", "log", "event"];
const SERVICE_KEYS: &[&str] = &["service.name", "service", "app", "application", "component"];
const HOST_KEYS: &[&str] = &["host.name", "host", "hostname"];
/// Words that mark a severity prefix to strip from a text message.
const LEVEL_WORDS: &[&str] = &[
    "TRACE", "TRC", "DEBUG", "DBG", "INFO", "INF", "NOTICE", "WARN", "WARNING", "WRN", "ERROR",
    "ERR", "FATAL", "FTL", "CRITICAL", "CRIT", "PANIC",
];

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedLine {
    pub message: String,
    pub severity: Severity,
    pub attributes: Vec<(String, String)>,
    pub service: Option<String>,
    pub host: Option<String>,
}

/// Parse a line as dox stores it (Docker's timestamp prefix included),
/// detecting its level and timestamp. Streaming code uses `parse_entry`.
pub fn parse_line(raw: &str) -> ParsedLine {
    parse_entry(raw, timestamp_prefix_len(raw), detect_level(raw))
}

/// Parse a line whose timestamp length and level the log buffer already
/// worked out, so neither is scanned for again. A JSON level field wins.
pub fn parse_entry(raw: &str, ts_len: usize, level: LogLevel) -> ParsedLine {
    let body = raw.get(ts_len..).unwrap_or(raw).trim();
    let severity = Severity::from_level(level);
    parse_json(body, severity).unwrap_or_else(|| parse_text(body, severity))
}

fn parse_json(body: &str, detected: Severity) -> Option<ParsedLine> {
    if !body.starts_with('{') {
        return None;
    }
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(body) else {
        return None;
    };
    let fields = flatten(&map);
    let level_key = find_key(&fields, LEVEL_KEYS);
    let message_key = find_key(&fields, MESSAGE_KEYS);
    let message = message_key
        .and_then(|k| lookup(&fields, k))
        .unwrap_or(body)
        .to_string();
    let severity = level_key
        .and_then(|k| lookup(&fields, k))
        .map_or(detected, Severity::normalize);
    let attributes: Vec<(String, String)> = fields
        .iter()
        .filter(|(k, _)| Some(k.as_str()) != level_key && Some(k.as_str()) != message_key)
        .cloned()
        .collect();
    Some(with_origin(message, severity, attributes))
}

fn parse_text(body: &str, severity: Severity) -> ParsedLine {
    with_origin(
        strip_prefix_noise(body).to_string(),
        severity,
        extract_fields(body),
    )
}

fn with_origin(
    message: String,
    severity: Severity,
    attributes: Vec<(String, String)>,
) -> ParsedLine {
    let service = first_value(&attributes, SERVICE_KEYS);
    let host = first_value(&attributes, HOST_KEYS);
    ParsedLine {
        message,
        severity,
        attributes,
        service,
        host,
    }
}

/// Top-level JSON scalars, plus one nested level as `parent.child`.
fn flatten(map: &Map<String, Value>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (key, value) in map {
        match value {
            Value::Object(inner) => {
                for (child, v) in inner {
                    if let Some(s) = scalar(v) {
                        out.push((format!("{key}.{child}"), s));
                    }
                }
            }
            other => {
                if let Some(s) = scalar(other) {
                    out.push((key.clone(), s));
                }
            }
        }
    }
    out
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// The first of `names` present among `fields` (case-insensitive), as stored.
fn find_key<'a>(fields: &'a [(String, String)], names: &[&str]) -> Option<&'a str> {
    names.iter().find_map(|name| {
        fields
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(k, _)| k.as_str())
    })
}

fn lookup<'a>(fields: &'a [(String, String)], key: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn first_value(fields: &[(String, String)], names: &[&str]) -> Option<String> {
    find_key(fields, names)
        .and_then(|k| lookup(fields, k))
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// `key=value` pairs that start a whitespace-separated token. Values may be
/// quoted with `"` or `'`.
fn extract_fields(text: &str) -> Vec<(String, String)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let at_token_start = i == 0 || bytes[i - 1].is_ascii_whitespace();
        if at_token_start && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
            let key_start = i;
            while i < bytes.len() && is_key_char(bytes[i]) {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'=' {
                let key = &text[key_start..i];
                let (value, next) = read_value(text, i + 1);
                out.push((key.to_string(), value.to_string()));
                i = next;
                continue;
            }
        }
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
    }
    out
}

fn is_key_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-')
}

/// Value starting at `start`, and the index just past it.
fn read_value(text: &str, start: usize) -> (&str, usize) {
    let bytes = text.as_bytes();
    if let Some(&quote) = bytes.get(start).filter(|b| matches!(b, b'"' | b'\'')) {
        let mut i = start + 1;
        while i < bytes.len() && bytes[i] != quote {
            if bytes[i] == b'\\' {
                i += 1;
            }
            i += 1;
        }
        let end = i.min(bytes.len());
        return (&text[start + 1..end], (end + 1).min(bytes.len()));
    }
    let mut i = start;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    (&text[start..i], i)
}

/// Drop leading timestamps and a severity token (`[info ]`, `ERROR:`).
fn strip_prefix_noise(body: &str) -> &str {
    let mut s = body.trim_start();
    loop {
        let ts = timestamp_prefix_len(s);
        if ts > 0 {
            s = s[ts..].trim_start();
            continue;
        }
        if let Some(rest) = s.strip_prefix('[') {
            if let Some(end) = rest.find(']') {
                if is_level_word(&rest[..end]) {
                    s = rest[end + 1..].trim_start();
                    continue;
                }
            }
        }
        let first = s.split_whitespace().next().unwrap_or("");
        if !first.is_empty() && is_level_word(first) {
            s = s[first.len()..].trim_start();
            continue;
        }
        return s;
    }
}

fn is_level_word(token: &str) -> bool {
    let t = token.trim().trim_end_matches(':').to_ascii_uppercase();
    LEVEL_WORDS.contains(&t.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::logs::LogLevel;

    fn attr<'a>(p: &'a ParsedLine, key: &str) -> Option<&'a str> {
        p.attributes
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn parse_entry_trusts_the_known_level() {
        // The log buffer already detected the level; it is not detected again.
        let p = parse_entry(
            "2026-09-26T10:00:00Z INFO but really fine",
            20,
            LogLevel::Error,
        );
        assert_eq!(p.severity, Severity::Error);
        assert_eq!(p.message, "but really fine");
    }

    #[test]
    fn a_json_level_field_still_wins() {
        let raw = r#"2026-09-26T10:00:00Z {"level":"warn","msg":"m"}"#;
        assert_eq!(
            parse_entry(raw, 20, LogLevel::Info).severity,
            Severity::Warn
        );
    }

    #[test]
    fn unknown_level_counts_as_info() {
        assert_eq!(
            parse_entry("hello", 0, LogLevel::Other).severity,
            Severity::Info
        );
    }

    #[test]
    fn airflow_structlog_line() {
        let raw = "2026-09-24T15:12:57.689526Z 2026-09-24T15:12:57.689526Z [info     ] request finished               [http.access] client_addr=172.19.0.1:42078 duration_us=17733 method=GET path=/ui/auth/menus query='limit=10&offset=0' status_code=200";
        let p = parse_line(raw);
        assert_eq!(p.severity, Severity::Info);
        assert!(p.message.starts_with("request finished"), "{}", p.message);
        assert_eq!(attr(&p, "status_code"), Some("200"));
        assert_eq!(attr(&p, "method"), Some("GET"));
        assert_eq!(attr(&p, "query"), Some("limit=10&offset=0"));
        assert_eq!(attr(&p, "client_addr"), Some("172.19.0.1:42078"));
    }

    #[test]
    fn postgres_line_without_fields() {
        let raw = "2026-08-23T02:30:12.359572635Z 2026-08-23 02:30:12.359 UTC [119996] FATAL:  terminating connection due to administrator command";
        let p = parse_line(raw);
        assert_eq!(p.severity, Severity::Fatal);
        assert!(p.attributes.is_empty());
        assert!(
            p.message.contains("terminating connection"),
            "{}",
            p.message
        );
    }

    #[test]
    fn json_line_uses_level_message_and_fields() {
        let raw = r#"2026-09-26T10:00:00.000000000Z {"level":"warning","msg":"slow query","service":"brand-boost","db":{"ms":812},"host":"api-1"}"#;
        let p = parse_line(raw);
        assert_eq!(p.severity, Severity::Warn);
        assert_eq!(p.message, "slow query");
        assert_eq!(p.service.as_deref(), Some("brand-boost"));
        assert_eq!(p.host.as_deref(), Some("api-1"));
        assert_eq!(attr(&p, "db.ms"), Some("812"));
        assert_eq!(attr(&p, "level"), None);
        assert_eq!(attr(&p, "msg"), None);
    }

    #[test]
    fn service_comes_from_common_field_names() {
        let p = parse_line("app=worker component=x hello");
        assert_eq!(p.service.as_deref(), Some("worker"));
    }

    #[test]
    fn plain_line_without_timestamp() {
        let p = parse_line("ERROR: disk full");
        assert_eq!(p.severity, Severity::Error);
        assert!(p.message.contains("disk full"));
    }

    #[test]
    fn urls_and_equations_in_text_are_not_fields() {
        let p = parse_line("see http://x/y?a=b and 1+1=2");
        assert!(p.attributes.is_empty(), "{:?}", p.attributes);
    }
}
