//! Word extraction for the Top Words box, using Gonzo's rules: identifier-like
//! tokens, lowercased, at least three characters, minus common stop words.

const MIN_WORD_LEN: usize = 3;

const STOP_WORDS: &[&str] = &[
    "the", "and", "for", "are", "but", "not", "you", "all", "can", "had", "her", "was", "one",
    "our", "out", "day", "get", "has", "him", "his", "how", "man", "new", "now", "old", "see",
    "two", "way", "who", "boy", "end", "did", "its", "let", "put", "say", "she", "too", "use",
    "from", "this", "that", "there", "they", "with", "what", "when", "where", "which", "while",
    "why", "will", "would", "could", "should", "might", "must", "if", "then", "than", "so", "just",
    "like", "more", "some", "such", "very", "also", "back", "down", "over", "up", "after",
    "before", "between", "during", "around", "through", "across", "against", "without",
];

/// Call `f` with each word in `message` that counts toward Top Words.
/// One lowercase copy per message; no allocation per word.
pub fn for_each_word(message: &str, mut f: impl FnMut(&str)) {
    let lower = message.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let starts_word = bytes[i].is_ascii_alphabetic() || bytes[i] == b'_';
        if !starts_word {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        let word = &lower[start..i];
        if word.len() >= MIN_WORD_LEN && !STOP_WORDS.contains(&word) {
            f(word);
        }
    }
}

/// The words of `message`, collected (tests).
#[cfg(test)]
pub fn extract_words(message: &str) -> Vec<String> {
    let mut words = Vec::new();
    for_each_word(message, |w| words.push(w.to_string()));
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_identifier_tokens_lowercased() {
        assert_eq!(
            extract_words("Request finished status_code=200 path=/api/v2/Dags"),
            vec!["request", "finished", "status_code", "path", "api", "dags"]
        );
    }

    #[test]
    fn drops_short_words_numbers_and_stop_words() {
        assert_eq!(
            extract_words("the db is up at 10.0.0.1 with 42 rows"),
            vec!["rows"]
        );
    }

    #[test]
    fn tokens_must_start_with_a_letter_or_underscore() {
        assert_eq!(
            extract_words("9lives _private x1y"),
            vec!["lives", "_private", "x1y"]
        );
    }
}
