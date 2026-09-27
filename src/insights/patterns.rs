//! Log pattern mining with Drain (`drain3` crate), configured like Gonzo:
//! depth 4, 50% similarity, 50 children per node, at most 100 patterns.
//! Variable parts render as `***`.

use std::borrow::Cow;
use std::collections::HashMap;

use drain3::{Config, Matcher, Template};

const MAX_PATTERNS: usize = 100;
/// Lines are cut to this many tokens / bytes before matching; `drain3`
/// rejects longer ones outright.
const MAX_TOKENS: usize = 256;
const MAX_BYTES: usize = 4096;
const TEMPLATE_DISPLAY_CHARS: usize = 100;

#[derive(Debug, Clone, PartialEq)]
pub struct PatternInfo {
    pub template: String,
    pub count: u64,
    pub percent: f64,
}

/// A pattern as last seen: its text, line count, and shape (positions,
/// literal tokens). The text only changes when the shape does.
struct Cluster {
    text: String,
    count: u64,
    shape: (usize, usize),
}

pub struct PatternMiner {
    matcher: Matcher,
    /// Patterns by id. `drain3` only returns the pattern a line joined, so the
    /// running list lives here.
    clusters: HashMap<usize, Cluster>,
    /// Lines turned away once the pattern limit was reached.
    overflow: u64,
}

impl PatternMiner {
    pub fn new() -> Self {
        let cfg = Config {
            depth: 4,
            similarity_threshold: 0.5,
            max_children: 50,
            max_clusters: MAX_PATTERNS,
            max_tokens: MAX_TOKENS,
            max_bytes: MAX_BYTES,
            ..Config::default()
        };
        Self {
            matcher: Matcher::new(cfg),
            clusters: HashMap::new(),
            overflow: 0,
        }
    }

    pub fn add(&mut self, message: &str) {
        let line = clamp(message);
        if line.is_empty() {
            return;
        }
        let Ok(template) = self.matcher.add_log_message(&line) else {
            self.overflow += 1;
            return;
        };
        let count = template.count() as u64;
        let shape = (template.token_count(), template.tokens().len());
        if let Some(cluster) = self.clusters.get_mut(&template.id()) {
            cluster.count = count;
            if cluster.shape != shape {
                cluster.text = render(&template);
                cluster.shape = shape;
            }
            return;
        }
        // `drain3` enforces its cluster limit on only one of its code paths,
        // so the limit is applied here as well.
        if self.clusters.len() >= MAX_PATTERNS {
            self.overflow += 1;
            return;
        }
        let text = render(&template);
        self.clusters
            .insert(template.id(), Cluster { text, count, shape });
    }

    /// Number of distinct patterns.
    pub fn len(&self) -> usize {
        self.clusters.len()
    }

    /// Lines that matched a pattern.
    pub fn total(&self) -> u64 {
        self.clusters.values().map(|c| c.count).sum()
    }

    /// Lines not counted because the pattern limit was reached.
    pub fn overflow(&self) -> u64 {
        self.overflow
    }

    /// Top `n` patterns by count (ties by template); percent of all matched lines.
    pub fn top(&self, n: usize) -> Vec<PatternInfo> {
        let total = self.total().max(1) as f64;
        let mut all: Vec<PatternInfo> = self
            .clusters
            .values()
            .map(|c| PatternInfo {
                template: c.text.clone(),
                count: c.count,
                percent: c.count as f64 * 100.0 / total,
            })
            .collect();
        all.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.template.cmp(&b.template))
        });
        all.truncate(n);
        all
    }
}

impl Default for PatternMiner {
    fn default() -> Self {
        Self::new()
    }
}

/// The message as is when within `drain3`'s limits; otherwise the first
/// `MAX_TOKENS` tokens, at most `MAX_BYTES` bytes (on a char boundary).
fn clamp(message: &str) -> Cow<'_, str> {
    let trimmed = message.trim();
    if trimmed.len() <= MAX_BYTES && trimmed.split_whitespace().count() <= MAX_TOKENS {
        return Cow::Borrowed(trimmed);
    }
    let joined = message
        .split_whitespace()
        .take(MAX_TOKENS)
        .collect::<Vec<_>>()
        .join(" ");
    if joined.len() <= MAX_BYTES {
        return Cow::Owned(joined);
    }
    let mut end = MAX_BYTES;
    while !joined.is_char_boundary(end) {
        end -= 1;
    }
    Cow::Owned(joined[..end].to_string())
}

/// Template text with `***` for variable parts, shortened for display.
fn render(template: &Template) -> String {
    let mut literal = template.tokens().iter();
    let parts: Vec<&str> = (0..template.token_count())
        .map(|idx| {
            if template.is_param(idx) {
                "***"
            } else {
                literal.next().map(|t| t.as_ref()).unwrap_or("***")
            }
        })
        .collect();
    let text = parts.join(" ");
    if text.chars().count() <= TEMPLATE_DISPLAY_CHARS {
        return text;
    }
    let short: String = text.chars().take(TEMPLATE_DISPLAY_CHARS - 3).collect();
    format!("{short}...")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn similar_lines_share_a_pattern() {
        let mut m = PatternMiner::new();
        m.add("user 17 logged in from 10.0.0.1");
        m.add("user 42 logged in from 10.0.0.9");
        m.add("user 99 logged in from 10.0.0.3");
        m.add("disk full on /dev/sda1");
        let top = m.top(10);
        assert_eq!(top.len(), 2, "{top:?}");
        assert_eq!(top[0].count, 3);
        assert!(
            top[0].template.contains("logged in from"),
            "{}",
            top[0].template
        );
        assert!(top[0].template.contains("***"), "{}", top[0].template);
        assert!((top[0].percent - 75.0).abs() < 0.01);
        assert_eq!(m.total(), 4);
    }

    #[test]
    fn counts_stream_in_one_line_at_a_time() {
        let mut m = PatternMiner::new();
        for i in 0..10 {
            m.add(&format!("request {i} finished ok"));
        }
        assert_eq!(m.top(1)[0].count, 10);
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn very_long_lines_are_truncated_not_dropped() {
        let mut m = PatternMiner::new();
        let long = (0..500)
            .map(|i| format!("tok{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        m.add(&long);
        m.add(&"x".repeat(10_000));
        assert_eq!(m.total(), 2);
    }

    #[test]
    fn blank_messages_are_ignored() {
        let mut m = PatternMiner::new();
        m.add("   ");
        assert_eq!(m.total(), 0);
    }

    #[test]
    fn lines_past_the_pattern_limit_are_counted_as_overflow() {
        let mut m = PatternMiner::new();
        for i in 0..150 {
            // Distinct token counts force distinct patterns.
            m.add(&vec!["word"; i + 1].join(" "));
        }
        assert!(m.len() <= 100);
        assert_eq!(m.total() + m.overflow(), 150);
        assert!(m.overflow() > 0);
    }

    #[test]
    fn long_templates_are_shortened_for_display() {
        let mut m = PatternMiner::new();
        m.add(&"abcdefghij ".repeat(20));
        assert!(m.top(1)[0].template.chars().count() <= 100);
    }
}
