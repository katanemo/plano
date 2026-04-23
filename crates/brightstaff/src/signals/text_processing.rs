//! Text normalization and similarity primitives.
//!
//! Direct Rust port of `signals/text_processing.py` from the reference. The
//! shapes (`NormalizedMessage`, `NormalizedPattern`) and similarity formulas
//! match the Python implementation exactly so that pattern matching produces
//! the same results on the same inputs.

use std::collections::{HashMap, HashSet};

/// Size of character n-grams used for fuzzy similarity (3 = trigrams).
pub const NGRAM_SIZE: usize = 3;

const PUNCT_TRIM: &[char] = &[
    '!', '"', '#', '$', '%', '&', '\'', '(', ')', '*', '+', ',', '-', '.', '/', ':', ';', '<', '=',
    '>', '?', '@', '[', '\\', ']', '^', '_', '`', '{', '|', '}', '~',
];

/// Pre-processed message with normalized text and tokens for efficient matching.
#[derive(Debug, Clone, Default)]
pub struct NormalizedMessage {
    pub raw: String,
    pub tokens: Vec<String>,
    pub token_set: HashSet<String>,
    pub bigram_set: HashSet<String>,
    pub char_ngram_set: HashSet<String>,
    pub token_frequency: HashMap<String, usize>,
}

impl NormalizedMessage {
    /// Create a normalized message from raw text. Mirrors
    /// `NormalizedMessage.from_text` in the reference, including the
    /// head-20%/tail-80% truncation strategy when text exceeds `max_length`.
    pub fn from_text(text: &str, max_length: usize) -> Self {
        let char_count = text.chars().count();

        let raw: String = if char_count <= max_length {
            text.to_string()
        } else {
            let head_len = max_length / 5;
            // Reserve one char for the joining space.
            let tail_len = max_length.saturating_sub(head_len + 1);
            let head: String = text.chars().take(head_len).collect();
            let tail: String = text
                .chars()
                .skip(char_count.saturating_sub(tail_len))
                .collect();
            format!("{} {}", head, tail)
        };

        // Normalize unicode punctuation to ASCII equivalents.
        let normalized_unicode = raw
            .replace(['\u{2019}', '\u{2018}'], "'")
            .replace(['\u{201c}', '\u{201d}'], "\"")
            .replace(['\u{2013}', '\u{2014}'], "-");

        // Lowercase + collapse whitespace (matches Python's `" ".join(s.split())`).
        let normalized: String = normalized_unicode
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        let mut tokens: Vec<String> = Vec::new();
        for word in normalized.split_whitespace() {
            let stripped: String = word.trim_matches(PUNCT_TRIM).to_string();
            if !stripped.is_empty() {
                tokens.push(stripped);
            }
        }

        let token_set: HashSet<String> = tokens.iter().cloned().collect();

        let mut bigram_set: HashSet<String> = HashSet::new();
        for i in 0..tokens.len().saturating_sub(1) {
            bigram_set.insert(format!("{} {}", tokens[i], tokens[i + 1]));
        }

        let tokens_text = tokens.join(" ");
        let char_ngram_set = char_ngrams(&tokens_text, NGRAM_SIZE);

        let mut token_frequency: HashMap<String, usize> = HashMap::new();
        for t in &tokens {
            *token_frequency.entry(t.clone()).or_insert(0) += 1;
        }

        Self {
            raw,
            tokens,
            token_set,
            bigram_set,
            char_ngram_set,
            token_frequency,
        }
    }

    pub fn contains_token(&self, token: &str) -> bool {
        self.token_set.contains(token)
    }

    pub fn contains_phrase(&self, phrase: &str) -> bool {
        let phrase_tokens: Vec<&str> = phrase.split_whitespace().collect();
        if phrase_tokens.is_empty() {
            return false;
        }
        if phrase_tokens.len() == 1 {
            return self.contains_token(phrase_tokens[0]);
        }
        if phrase_tokens.len() > self.tokens.len() {
            return false;
        }
        let n = phrase_tokens.len();
        for i in 0..=self.tokens.len() - n {
            if self.tokens[i..i + n]
                .iter()
                .zip(phrase_tokens.iter())
                .all(|(a, b)| a == b)
            {
                return true;
            }
        }
        false
    }

    /// Character n-gram (Jaccard) similarity vs another normalized message.
    pub fn ngram_similarity_with_message(&self, other: &NormalizedMessage) -> f32 {
        jaccard(&self.char_ngram_set, &other.char_ngram_set)
    }

    /// Character n-gram (Jaccard) similarity vs a raw pattern string.
    pub fn ngram_similarity_with_pattern(&self, pattern: &str) -> f32 {
        let normalized = strip_non_word_chars(&pattern.to_lowercase());
        let pattern_ngrams = char_ngrams(&normalized, NGRAM_SIZE);
        jaccard(&self.char_ngram_set, &pattern_ngrams)
    }

    /// Fraction of pattern's ngrams contained in this message's ngram set.
    pub fn char_ngram_containment(&self, pattern: &str) -> f32 {
        let normalized = strip_non_word_chars(&pattern.to_lowercase());
        let pattern_ngrams = char_ngrams(&normalized, NGRAM_SIZE);
        if pattern_ngrams.is_empty() {
            return 0.0;
        }
        let contained = pattern_ngrams
            .iter()
            .filter(|ng| self.char_ngram_set.contains(*ng))
            .count();
        contained as f32 / pattern_ngrams.len() as f32
    }

    /// Token-frequency cosine similarity vs a raw pattern string.
    pub fn token_cosine_similarity(&self, pattern: &str) -> f32 {
        let mut pattern_freq: HashMap<String, usize> = HashMap::new();
        for word in pattern.to_lowercase().split_whitespace() {
            let stripped = word.trim_matches(PUNCT_TRIM);
            if !stripped.is_empty() {
                *pattern_freq.entry(stripped.to_string()).or_insert(0) += 1;
            }
        }
        cosine_freq(&self.token_frequency, &pattern_freq)
    }

    /// Layered match against a pre-normalized pattern. Mirrors
    /// `matches_normalized_pattern` from the reference: exact phrase ->
    /// char-ngram Jaccard -> token cosine.
    pub fn matches_normalized_pattern(
        &self,
        pattern: &NormalizedPattern,
        char_ngram_threshold: f32,
        token_cosine_threshold: f32,
    ) -> bool {
        // Layer 0: exact phrase match using pre-tokenized message.
        let plen = pattern.tokens.len();
        let slen = self.tokens.len();
        if plen > 0 && plen <= slen {
            for i in 0..=slen - plen {
                if self.tokens[i..i + plen] == pattern.tokens[..] {
                    return true;
                }
            }
        }

        // Layer 1: character n-gram Jaccard similarity.
        if !self.char_ngram_set.is_empty() && !pattern.char_ngram_set.is_empty() {
            let inter = self
                .char_ngram_set
                .intersection(&pattern.char_ngram_set)
                .count();
            let union = self.char_ngram_set.union(&pattern.char_ngram_set).count();
            if union > 0 {
                let sim = inter as f32 / union as f32;
                if sim >= char_ngram_threshold {
                    return true;
                }
            }
        }

        // Layer 2: token frequency cosine similarity.
        if !self.token_frequency.is_empty() && !pattern.token_frequency.is_empty() {
            let sim = cosine_freq(&self.token_frequency, &pattern.token_frequency);
            if sim >= token_cosine_threshold {
                return true;
            }
        }

        false
    }
}

/// Pre-processed pattern with normalized text and pre-computed n-grams/tokens.
#[derive(Debug, Clone, Default)]
pub struct NormalizedPattern {
    pub raw: String,
    pub tokens: Vec<String>,
    pub char_ngram_set: HashSet<String>,
    pub token_frequency: HashMap<String, usize>,
}

impl NormalizedPattern {
    pub fn from_text(pattern: &str) -> Self {
        let normalized = pattern
            .to_lowercase()
            .replace(['\u{2019}', '\u{2018}'], "'")
            .replace(['\u{201c}', '\u{201d}'], "\"")
            .replace(['\u{2013}', '\u{2014}'], "-");
        let normalized: String = normalized.split_whitespace().collect::<Vec<_>>().join(" ");

        // Tokenize the same way as NormalizedMessage (trim boundary punctuation,
        // keep internal punctuation).
        let mut tokens: Vec<String> = Vec::new();
        for word in normalized.split_whitespace() {
            let stripped = word.trim_matches(PUNCT_TRIM);
            if !stripped.is_empty() {
                tokens.push(stripped.to_string());
            }
        }

        // For ngrams + cosine, strip ALL punctuation (matches Python's
        // `re.sub(r"[^\w\s]", "", normalized)`).
        let normalized_for_ngrams = strip_non_word_chars(&normalized);
        let char_ngram_set = char_ngrams(&normalized_for_ngrams, NGRAM_SIZE);

        let tokens_no_punct: Vec<&str> = normalized_for_ngrams.split_whitespace().collect();
        let mut token_frequency: HashMap<String, usize> = HashMap::new();
        for t in &tokens_no_punct {
            *token_frequency.entry((*t).to_string()).or_insert(0) += 1;
        }

        Self {
            raw: pattern.to_string(),
            tokens,
            char_ngram_set,
            token_frequency,
        }
    }
}

/// Convenience: normalize a list of raw pattern strings into `NormalizedPattern`s.
pub fn normalize_patterns(patterns: &[&str]) -> Vec<NormalizedPattern> {
    patterns
        .iter()
        .map(|p| NormalizedPattern::from_text(p))
        .collect()
}

// ---------------------------------------------------------------------------
// Similarity primitives
// ---------------------------------------------------------------------------

fn char_ngrams(s: &str, n: usize) -> HashSet<String> {
    // Python iterates by character index, not byte; mirror that with .chars().
    let chars: Vec<char> = s.chars().collect();
    let mut out: HashSet<String> = HashSet::new();
    if chars.len() < n {
        return out;
    }
    for i in 0..=chars.len() - n {
        out.insert(chars[i..i + n].iter().collect());
    }
    out
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        0.0
    } else {
        inter as f32 / union as f32
    }
}

fn cosine_freq(a: &HashMap<String, usize>, b: &HashMap<String, usize>) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut dot: f64 = 0.0;
    let mut n1_sq: f64 = 0.0;
    let mut n2_sq: f64 = 0.0;
    for (token, &freq2) in b {
        let freq1 = *a.get(token).unwrap_or(&0);
        dot += (freq1 * freq2) as f64;
        n2_sq += (freq2 * freq2) as f64;
    }
    for &freq1 in a.values() {
        n1_sq += (freq1 * freq1) as f64;
    }
    let n1 = n1_sq.sqrt();
    let n2 = n2_sq.sqrt();
    if n1 == 0.0 || n2 == 0.0 {
        0.0
    } else {
        (dot / (n1 * n2)) as f32
    }
}

/// Python equivalent: `re.sub(r"[^\w\s]", "", text)` followed by whitespace
/// collapse. Python's `\w` is `[A-Za-z0-9_]` plus unicode word characters; we
/// use Rust's `char::is_alphanumeric()` plus `_` for an equivalent definition.
fn strip_non_word_chars(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' || c.is_whitespace() {
            out.push(c);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_lowercases_and_strips_punctuation() {
        let m = NormalizedMessage::from_text("Hello, World!", 2000);
        assert_eq!(m.tokens, vec!["hello".to_string(), "world".to_string()]);
    }

    #[test]
    fn normalizes_smart_quotes() {
        let m = NormalizedMessage::from_text("don\u{2019}t", 2000);
        assert!(m.tokens.contains(&"don't".to_string()));
    }

    #[test]
    fn truncates_long_text_with_head_tail() {
        let long = "a".repeat(3000);
        let m = NormalizedMessage::from_text(&long, 2000);
        // raw should be ~ 2000 chars (head + space + tail)
        assert!(m.raw.chars().count() <= 2001);
        assert!(m.raw.starts_with("aa"));
        assert!(m.raw.ends_with("aa"));
    }

    #[test]
    fn contains_phrase_matches_consecutive_tokens() {
        let m = NormalizedMessage::from_text("I think this is great work", 2000);
        assert!(m.contains_phrase("this is great"));
        assert!(!m.contains_phrase("great this"));
    }

    #[test]
    fn matches_pattern_via_exact_phrase() {
        let m = NormalizedMessage::from_text("No, I meant the second one", 2000);
        let p = NormalizedPattern::from_text("no i meant");
        assert!(m.matches_normalized_pattern(&p, 0.65, 0.6));
    }

    #[test]
    fn matches_pattern_via_char_ngram_fuzziness() {
        // Typo in "meant" -> "ment" so layer 0 (exact phrase) cannot match,
        // forcing the matcher to fall back to layer 1 (char n-gram Jaccard).
        let m = NormalizedMessage::from_text("No I ment", 2000);
        let p = NormalizedPattern::from_text("no i meant");
        assert!(m.matches_normalized_pattern(&p, 0.4, 0.6));
    }

    #[test]
    fn jaccard_identical_sets_is_one() {
        let a: HashSet<String> = ["abc", "bcd"].iter().map(|s| s.to_string()).collect();
        assert!((jaccard(&a, &a) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_freq_orthogonal_is_zero() {
        let mut a: HashMap<String, usize> = HashMap::new();
        a.insert("hello".to_string(), 1);
        let mut b: HashMap<String, usize> = HashMap::new();
        b.insert("world".to_string(), 1);
        assert_eq!(cosine_freq(&a, &b), 0.0);
    }
}
