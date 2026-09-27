//! Log severity as Gonzo reports it: six levels, unknown lines count as INFO.

use crate::ui::logs::{detect_level, LogLevel};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Severity {
    Fatal,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl Severity {
    /// Most to least severe, the order every chart and filter uses.
    pub const ALL: [Severity; 6] = [
        Severity::Fatal,
        Severity::Error,
        Severity::Warn,
        Severity::Info,
        Severity::Debug,
        Severity::Trace,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn label(self) -> &'static str {
        match self {
            Severity::Fatal => "FATAL",
            Severity::Error => "ERROR",
            Severity::Warn => "WARN",
            Severity::Info => "INFO",
            Severity::Debug => "DEBUG",
            Severity::Trace => "TRACE",
        }
    }

    /// Severity of a plain-text line, using dox's own level detection.
    pub fn detect(text: &str) -> Self {
        Self::from_level(detect_level(text))
    }

    /// dox's detected level as a severity; lines without one count as INFO.
    pub fn from_level(level: LogLevel) -> Self {
        match level {
            LogLevel::Fatal => Severity::Fatal,
            LogLevel::Error => Severity::Error,
            LogLevel::Warn => Severity::Warn,
            LogLevel::Debug => Severity::Debug,
            LogLevel::Trace => Severity::Trace,
            LogLevel::Info | LogLevel::Other => Severity::Info,
        }
    }

    /// Map a level field value (`warning`, `ERR`, `crit`, ...) to a severity.
    pub fn normalize(value: &str) -> Self {
        let v = value.trim().to_ascii_uppercase();
        match v.as_str() {
            "TRACE" | "TRAC" | "TRC" => Severity::Trace,
            "DEBUG" | "DEBU" | "DBG" | "DEB" => Severity::Debug,
            "INFO" | "INFORMATION" | "INF" => Severity::Info,
            "WARN" | "WARNING" | "WRNG" | "WRN" => Severity::Warn,
            "ERROR" | "ERR" | "ERRO" => Severity::Error,
            "FATAL" | "FATL" | "FTL" | "CRITICAL" | "CRIT" | "CRT" | "PANIC" | "PNC" => {
                Severity::Fatal
            }
            _ => match v.get(..4) {
                Some("WARN") => Severity::Warn,
                Some("ERRO") => Severity::Error,
                Some("DEBU") => Severity::Debug,
                Some("TRAC") => Severity::Trace,
                Some("FATA") | Some("CRIT") => Severity::Fatal,
                _ => Severity::Info,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_follows_all_order() {
        for (i, s) in Severity::ALL.iter().enumerate() {
            assert_eq!(s.index(), i);
        }
        assert_eq!(Severity::Warn.label(), "WARN");
    }

    #[test]
    fn normalize_maps_common_spellings() {
        assert_eq!(Severity::normalize("warning"), Severity::Warn);
        assert_eq!(Severity::normalize(" ERR "), Severity::Error);
        assert_eq!(Severity::normalize("crit"), Severity::Fatal);
        assert_eq!(Severity::normalize("panic"), Severity::Fatal);
        assert_eq!(Severity::normalize("dbg"), Severity::Debug);
        assert_eq!(Severity::normalize("trc"), Severity::Trace);
        assert_eq!(Severity::normalize("information"), Severity::Info);
        assert_eq!(Severity::normalize("errorish"), Severity::Error);
    }

    #[test]
    fn unknown_values_count_as_info() {
        assert_eq!(Severity::normalize("verbose"), Severity::Info);
        assert_eq!(Severity::normalize(""), Severity::Info);
        assert_eq!(Severity::detect("just some text"), Severity::Info);
    }

    #[test]
    fn detect_uses_the_level_word_in_the_line() {
        assert_eq!(
            Severity::detect("2026-01-01 12:00:00 [error    ] boom"),
            Severity::Error
        );
        assert_eq!(
            Severity::detect("LOG:  FATAL:  terminating"),
            Severity::Fatal
        );
    }
}
