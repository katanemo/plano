//! Agentic Signals - Behavioral quality indicators for agent interactions
//!
//! This module implements various signals that serve as early warning indicators
//! of brilliant successes or failures in agentic interactions. These signals are
//! derived from conversation patterns and can be computed algorithmically from
//! message arrays.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use hermesllm::apis::openai::{Message, Role};

// ============================================================================
// Constants
// ============================================================================

/// Flag emoji for marking spans/operations worth investigating
pub const FLAG_MARKER: &str = "\u{1F6A9}";

/// Size of character n-grams for similarity matching (3 = trigrams)
const NGRAM_SIZE: usize = 3;

// ============================================================================
// Normalized Message Processing
// ============================================================================

/// Pre-processed message with normalized text and tokens for efficient matching
#[derive(Debug, Clone)]
struct NormalizedMessage {
    /// Original raw text
    raw: String,
    /// Tokens (words) extracted from the message
    tokens: Vec<String>,
    /// Token set for fast lookup
    token_set: HashSet<String>,
    /// Bigram set for fast similarity computation
    bigram_set: HashSet<String>,
    /// Character ngram set for robust similarity matching
    char_ngram_set: HashSet<String>,
    /// Token frequency map for multiset cosine similarity
    token_frequency: HashMap<String, usize>,
}

impl NormalizedMessage {
    #[allow(dead_code)] // Used in tests for algorithm validation
    fn from_text(text: &str) -> Self {
        Self::from_text_with_limit(text, usize::MAX)
    }

    fn from_text_with_limit(text: &str, max_length: usize) -> Self {
        // Truncate to max_length characters to prevent unbounded computation
        // Keep head (20%) + tail (80%) to preserve both context and intent

        let char_count = text.chars().count();

        let raw = if char_count <= max_length {
            text.to_string()
        } else {
            // Split: 20% head, 79% tail, 1 char space delimiter
            let head_len = max_length / 5;
            let tail_len = max_length - head_len - 1;

            let head: String = text.chars().take(head_len).collect();
            let tail: String = text.chars().skip(char_count - tail_len).collect();

            format!("{} {}", head, tail)
        };

        // Normalize unicode punctuation to ASCII equivalents
        let normalized_unicode = raw
            .replace(['\u{2019}', '\u{2018}'], "'") // U+2019/U+2018 SINGLE QUOTATION MARKs
            .replace(['\u{201C}', '\u{201D}'], "\"") // U+201C/U+201D DOUBLE QUOTATION MARKs
            .replace(['\u{2013}', '\u{2014}'], "-"); // U+2013/U+2014 EN/EM DASHes

        // Normalize: lowercase, collapse whitespace
        let normalized = normalized_unicode
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        // Tokenize: split on whitespace and strip punctuation from boundaries
        let tokens: Vec<String> = normalized
            .split_whitespace()
            .map(|word| {
                // Strip leading/trailing punctuation but keep internal punctuation
                word.trim_matches(|c: char| c.is_ascii_punctuation())
                    .to_string()
            })
            .filter(|w| !w.is_empty())
            .collect();

        let token_set: HashSet<String> = tokens.iter().cloned().collect();

        // Generate bigram set directly for similarity matching
        let bigram_set: HashSet<String> = tokens
            .windows(2)
            .map(|w| format!("{} {}", w[0], w[1]))
            .collect();

        // Generate character ngram set for robust similarity matching
        // Uses tokens (with punctuation stripped) for consistency with pattern matching
        let tokens_text = tokens.join(" ");
        let char_ngram_set: HashSet<String> = tokens_text
            .chars()
            .collect::<Vec<_>>()
            .windows(NGRAM_SIZE)
            .map(|w| w.iter().collect::<String>())
            .collect();

        // Compute token frequency map for cosine similarity
        let mut token_frequency: HashMap<String, usize> = HashMap::new();
        for token in &tokens {
            *token_frequency.entry(token.clone()).or_insert(0) += 1;
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

    /// Check if a single token exists in the message (word boundary aware)
    fn contains_token(&self, token: &str) -> bool {
        self.token_set.contains(token)
    }

    /// Check if a phrase (sequence of tokens) exists in the message
    fn contains_phrase(&self, phrase: &str) -> bool {
        let phrase_tokens: Vec<&str> = phrase.split_whitespace().collect();
        if phrase_tokens.is_empty() {
            return false;
        }

        if phrase_tokens.len() == 1 {
            return self.contains_token(phrase_tokens[0]);
        }

        // Multi-word phrase: check for sequence in tokens
        self.tokens.windows(phrase_tokens.len()).any(|window| {
            window
                .iter()
                .zip(phrase_tokens.iter())
                .all(|(token, phrase_token)| token == phrase_token)
        })
    }

    /// Calculate character ngram similarity between this message and a pattern
    /// Returns a similarity score between 0.0 and 1.0
    /// This is robust to typos, small edits, and word insertions
    #[allow(dead_code)] // Used in tests for algorithm validation
    fn char_ngram_similarity(&self, pattern: &str) -> f64 {
        // Normalize the pattern: lowercase and remove ALL punctuation
        // This makes "doesn't" → "doesnt" for robust typo matching
        let normalized_pattern = pattern
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        // Generate ngrams for the pattern
        let pattern_ngrams: HashSet<String> = normalized_pattern
            .chars()
            .collect::<Vec<_>>()
            .windows(NGRAM_SIZE)
            .map(|w| w.iter().collect::<String>())
            .collect();

        if self.char_ngram_set.is_empty() && pattern_ngrams.is_empty() {
            return 1.0; // Both empty = identical
        }

        if self.char_ngram_set.is_empty() || pattern_ngrams.is_empty() {
            return 0.0;
        }

        // Compute Jaccard similarity (intersection / union)
        let intersection = self.char_ngram_set.intersection(&pattern_ngrams).count();
        let union = self.char_ngram_set.union(&pattern_ngrams).count();

        if union == 0 {
            return 0.0;
        }

        intersection as f64 / union as f64
    }

    /// Calculate token-based cosine similarity using term frequencies
    /// Returns a similarity score between 0.0 and 1.0
    /// This handles word frequency and is stable for longer messages
    #[allow(dead_code)] // Used in tests for algorithm validation
    fn token_cosine_similarity(&self, pattern: &str) -> f64 {
        // Tokenize and compute frequencies for the pattern
        let pattern_tokens: Vec<String> = pattern
            .to_lowercase()
            .split_whitespace()
            .map(|word| {
                word.trim_matches(|c: char| c.is_ascii_punctuation())
                    .to_string()
            })
            .filter(|w| !w.is_empty())
            .collect();

        let mut pattern_frequency: HashMap<String, usize> = HashMap::new();
        for token in &pattern_tokens {
            *pattern_frequency.entry(token.clone()).or_insert(0) += 1;
        }

        if self.token_frequency.is_empty() && pattern_frequency.is_empty() {
            return 1.0;
        }

        if self.token_frequency.is_empty() || pattern_frequency.is_empty() {
            return 0.0;
        }

        // Compute cosine similarity
        // cosine_sim = dot_product / (norm1 * norm2)

        let mut dot_product = 0.0;
        let mut norm1_squared = 0.0;
        let mut norm2_squared = 0.0;

        // Collect all unique tokens from both sets
        let all_tokens: HashSet<String> = self
            .token_frequency
            .keys()
            .chain(pattern_frequency.keys())
            .cloned()
            .collect();

        for token in all_tokens {
            let freq1 = *self.token_frequency.get(&token).unwrap_or(&0) as f64;
            let freq2 = *pattern_frequency.get(&token).unwrap_or(&0) as f64;

            dot_product += freq1 * freq2;
            norm1_squared += freq1 * freq1;
            norm2_squared += freq2 * freq2;
        }

        let norm1 = norm1_squared.sqrt();
        let norm2 = norm2_squared.sqrt();

        if norm1 == 0.0 || norm2 == 0.0 {
            return 0.0;
        }

        dot_product / (norm1 * norm2)
    }

    /// Layered phrase matching: exact → character ngram → token cosine
    /// Returns true if the pattern matches using any layer
    #[allow(dead_code)] // Kept for reference; production uses matches_normalized_pattern
    fn layered_contains_phrase(
        &self,
        pattern: &str,
        char_ngram_threshold: f64,
        token_cosine_threshold: f64,
    ) -> bool {
        // Layer 0: Exact phrase match (fastest)
        if self.contains_phrase(pattern) {
            return true;
        }

        // Layer 1: Character ngram similarity (typo/edit robustness)
        // Check whole message first (for short messages)
        if self.char_ngram_similarity(pattern) >= char_ngram_threshold {
            return true;
        }

        // ngram containment check for patterns buried in longer messages
        // If ALL of the pattern's ngrams exist in the message, the pattern must be
        // present (possibly with minor variations like missing apostrophes).
        // This is O(pattern_ngrams) lookups vs expensive window sliding.
        if self.char_ngram_containment(pattern) >= 1.0 {
            return true;
        }

        // Layer 2: Token cosine similarity (semantic stability for long messages)
        if self.token_cosine_similarity(pattern) >= token_cosine_threshold {
            return true;
        }

        false
    }

    fn char_ngram_containment(&self, pattern: &str) -> f64 {
        // Normalize the pattern the same way as char_ngram_similarity
        let normalized_pattern = pattern
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        // Generate ngrams for the pattern
        let pattern_ngrams: HashSet<String> = normalized_pattern
            .chars()
            .collect::<Vec<_>>()
            .windows(NGRAM_SIZE)
            .map(|w| w.iter().collect::<String>())
            .collect();

        if pattern_ngrams.is_empty() {
            return 0.0;
        }

        // Count how many pattern ngrams exist in the message
        let contained = pattern_ngrams
            .iter()
            .filter(|t| self.char_ngram_set.contains(*t))
            .count();

        contained as f64 / pattern_ngrams.len() as f64
    }

    /// Fast matching against a pre-normalized pattern
    /// This avoids re-normalizing and re-computing ngrams for each pattern
    fn matches_normalized_pattern(
        &self,
        pattern: &NormalizedPattern,
        char_ngram_threshold: f64,
        token_cosine_threshold: f64,
    ) -> bool {
        // Layer 0: Exact phrase match (fastest)
        if self.contains_phrase(&pattern.raw) {
            return true;
        }

        // Layer 1: Character ngram similarity using pre-computed ngrams
        if !self.char_ngram_set.is_empty() && !pattern.char_ngram_set.is_empty() {
            let intersection = self
                .char_ngram_set
                .intersection(&pattern.char_ngram_set)
                .count();
            let union = self.char_ngram_set.union(&pattern.char_ngram_set).count();
            if union > 0 {
                let similarity = intersection as f64 / union as f64;
                if similarity >= char_ngram_threshold {
                    return true;
                }
            }
        }

        // Ngram containment check using pre-computed ngrams
        if !pattern.char_ngram_set.is_empty() {
            let contained = pattern
                .char_ngram_set
                .iter()
                .filter(|t| self.char_ngram_set.contains(*t))
                .count();
            let containment = contained as f64 / pattern.char_ngram_set.len() as f64;
            if containment >= 1.0 {
                return true;
            }
        }

        // Layer 2: Token cosine similarity using pre-computed frequencies
        if !self.token_frequency.is_empty() && !pattern.token_frequency.is_empty() {
            let mut dot_product = 0.0;
            let mut norm1_squared = 0.0;
            let mut norm2_squared = 0.0;

            // Iterate over pattern tokens (usually smaller set)
            for (token, &freq2) in &pattern.token_frequency {
                let freq1 = *self.token_frequency.get(token).unwrap_or(&0) as f64;
                let freq2 = freq2 as f64;
                dot_product += freq1 * freq2;
                norm2_squared += freq2 * freq2;
            }

            // Add self tokens not in pattern for norm1
            for &freq1 in self.token_frequency.values() {
                norm1_squared += (freq1 as f64) * (freq1 as f64);
            }

            let norm1 = norm1_squared.sqrt();
            let norm2 = norm2_squared.sqrt();

            if norm1 > 0.0 && norm2 > 0.0 {
                let similarity = dot_product / (norm1 * norm2);
                if similarity >= token_cosine_threshold {
                    return true;
                }
            }
        }

        false
    }
}

// ============================================================================
// Normalized Pattern (pre-computed for performance)
// ============================================================================

/// Pre-processed pattern with normalized text and pre-computed ngrams/tokens
/// This avoids redundant computation when matching against many messages
#[derive(Debug, Clone)]
struct NormalizedPattern {
    /// Original raw pattern text
    raw: String,
    /// Character ngram set for similarity matching
    char_ngram_set: HashSet<String>,
    /// Token frequency map for cosine similarity
    token_frequency: HashMap<String, usize>,
}

impl NormalizedPattern {
    fn new(pattern: &str) -> Self {
        // Normalize: lowercase and remove ALL punctuation
        let normalized = pattern
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        // Generate ngrams
        let char_ngram_set: HashSet<String> = normalized
            .chars()
            .collect::<Vec<_>>()
            .windows(NGRAM_SIZE)
            .map(|w| w.iter().collect::<String>())
            .collect();

        // Compute token frequency map
        let tokens: Vec<String> = normalized
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        let mut token_frequency: HashMap<String, usize> = HashMap::new();
        for token in tokens {
            *token_frequency.entry(token).or_insert(0) += 1;
        }

        Self {
            raw: pattern.to_string(),
            char_ngram_set,
            token_frequency,
        }
    }
}

/// Helper to create a static slice of normalized patterns
fn normalize_patterns(patterns: &[&str]) -> Vec<NormalizedPattern> {
    patterns.iter().map(|p| NormalizedPattern::new(p)).collect()
}

// ============================================================================
// Pre-computed Pattern Caches (initialized once at startup)
// ============================================================================

static REPAIR_PATTERNS: LazyLock<Vec<NormalizedPattern>> = LazyLock::new(|| {
    normalize_patterns(&[
        // Explicit corrections
        "i meant",
        "i mean",
        "sorry, i meant",
        "what i meant was",
        "what i actually meant",
        "i was trying to say",
        "let me correct that",
        "correction",
        "i misspoke",
        // Negations and disagreements
        "no, i",
        "no i",
        "nah i",
        "nope i",
        "not what i",
        "that's not",
        "that's not what",
        "that isn't what",
        "not quite",
        "not exactly",
        // Rephrasing indicators
        "let me rephrase",
        "let me try again",
        "let me clarify",
        "to clarify",
        "to be clear",
        "let me explain",
        "what i'm trying to",
        "what i'm saying",
        "in other words",
        // Actual/really emphasis
        "actually i",
        "actually no",
        "what i actually",
        "i actually",
        "i really meant",
        // Mistake acknowledgment
        "i was wrong",
        "my mistake",
        "my bad",
        "i should have said",
        "i should clarify",
        // Wait/hold indicators
        "wait, i",
        "wait no",
        "hold on",
        "hang on",
    ])
});

static COMPLAINT_PATTERNS: LazyLock<Vec<NormalizedPattern>> = LazyLock::new(|| {
    normalize_patterns(&[
        // Useless/unhelpful (multi-word only)
        "this is useless",
        "not helpful",
        "doesn't help",
        "not helping",
        "you're not helping",
        "no help",
        "unhelpful",
        // Not working
        "this doesn't work",
        "doesn't work",
        "not working",
        "isn't working",
        "won't work",
        "still doesn't work",
        "still not working",
        // Not fixing/solving
        "doesn't fix",
        "not fixing",
        "doesn't solve",
        "doesn't seem to work",
        "doesn't seem to fix",
        "not resolving",
        // Waste/pointless
        "waste of time",
        "wasting my time",
        // Ridiculous/absurd
        "this is ridiculous",
        "ridiculous",
        "this is absurd",
        "absurd",
        "this is insane",
        "insane",
        // Stupid/dumb (as adjectives, not as standalone tokens)
        "this is stupid",
        "this is dumb",
        // Quality complaints (multi-word)
        "this sucks",
        "not good enough",
        // Capability questions
        "why can't you",
        "can't you",
        // Frustration
        "this is frustrating",
        "frustrated",
        "incomplete",
        "overwhelm",
        "overwhelmed",
        "overwhelming",
        "exhausted",
        "struggled",
        // same issue
        "same issue",
        // polite dissatisfaction
        "i'm disappointed",
        "thanks, but",
        "appreciate it, but",
        "good, but",
        // Fed up/done
        "i give up",
        "give up",
        "fed up",
        "had enough",
        "can't take",
        // Bot-specific complaints
        "useless bot",
        "dumb bot",
        "stupid bot",
    ])
});

static CONFUSION_PATTERNS: LazyLock<Vec<NormalizedPattern>> = LazyLock::new(|| {
    normalize_patterns(&[
        // Don't understand
        "i don't understand",
        "don't understand",
        "not understanding",
        "can't understand",
        "don't get it",
        "don't follow",
        // Confused state
        "i'm confused",
        "so confused",
        // Makes no sense
        "makes no sense",
        "doesn't make sense",
        "not making sense",
        // What do you mean (keep multi-word)
        "what do you mean",
        "what does that mean",
        "what are you saying",
        // Lost/unclear
        "i'm lost",
        "totally lost",
        "lost me",
        // No clue
        "no clue",
        "no idea",
        // Come again
        "come again",
        "say that again",
        "repeat that",
    ])
});

static GRATITUDE_PATTERNS: LazyLock<Vec<NormalizedPattern>> = LazyLock::new(|| {
    normalize_patterns(&[
        // Standard gratitude
        "thank you",
        "thanks",
        "thank u",
        "thankyou",
        "thx",
        "ty",
        "tyvm",
        "tysm",
        "thnx",
        "thnks",
        // Strong gratitude
        "thanks so much",
        "thank you so much",
        "thanks a lot",
        "thanks a bunch",
        "much appreciated",
        "really appreciate",
        "greatly appreciate",
        "appreciate it",
        "appreciate that",
        "i appreciate",
        "grateful",
        "so grateful",
        // Helpfulness acknowledgment
        "that's helpful",
        "very helpful",
        "super helpful",
        "really helpful",
        "that helps",
        "this helps",
        "helpful",
        // Perfection expressions
        "perfect",
        "that's perfect",
        "just perfect",
        "exactly what i needed",
        "exactly right",
        "just what i needed",
        "that's exactly",
        // Informal positive
        "you're the best",
        "you rock",
        "you're awesome",
        "awesome sauce",
        "legend",
    ])
});

static SATISFACTION_PATTERNS: LazyLock<Vec<NormalizedPattern>> = LazyLock::new(|| {
    normalize_patterns(&[
        // Works/functions
        "that works",
        "this works",
        "works great",
        "works perfectly",
        "works for me",
        // Great variations
        "that's great",
        "that's amazing",
        "this is great",
        "sounds great",
        "looks great",
        "great job",
        // Excellent/perfect
        "excellent",
        "outstanding",
        "superb",
        "spectacular",
        // Awesome/amazing
        "awesome",
        "that's awesome",
        "amazing",
        "incredible",
        // Love expressions
        "love it",
        "love this",
        "i love",
        "loving it",
        "love that",
        // Brilliant/wonderful
        "brilliant",
        "wonderful",
        "fantastic",
        "fabulous",
        "marvelous",
    ])
});

static SUCCESS_PATTERNS: LazyLock<Vec<NormalizedPattern>> = LazyLock::new(|| {
    normalize_patterns(&[
        // Understanding confirmation
        "got it",
        "i got it",
        "understand",
        "understood",
        "i understand",
        "makes sense",
        "clear now",
        "i see",
        // Success/completion
        "success",
        "successful",
        "it worked",
        "that worked",
        "this worked",
        "worked",
        // Problem resolution
        "solved",
        "resolved",
        "fixed",
        "fixed it",
        "issue resolved",
        "problem solved",
        // Working state
        "working now",
        "it's working",
        "works now",
        "working fine",
        "working great",
        // Completion
        "all set",
        "all good",
        "we're good",
        "i'm good",
        "all done",
        "done",
        "complete",
        "finished",
        // Perfect fit
        "spot on",
        "nailed it",
        "bingo",
        "exactly",
        "just right",
    ])
});

static HUMAN_AGENT_PATTERNS: LazyLock<Vec<NormalizedPattern>> = LazyLock::new(|| {
    normalize_patterns(&[
        // Speak to human
        "speak to a human",
        "speak to human",
        "speak with a human",
        "speak with human",
        "talk to a human",
        "talk to human",
        "talk to a person",
        "talk to person",
        "talk to someone",
        // Human/real agent
        "human agent",
        "real agent",
        "actual agent",
        "live agent",
        "human support",
        // Real/actual person
        "real person",
        "actual person",
        "real human",
        "actual human",
        "someone real",
        // Need/want human
        "need a human",
        "need human",
        "want a human",
        "want human",
        "get me a human",
        "get me human",
        "get me someone",
        // Transfer/connect
        "transfer me",
        "connect me",
        "escalate this",
        // Representative (removed standalone "rep" - too many false positives)
        "representative",
        "customer service rep",
        "customer service representative",
        // Not a bot
        "not a bot",
        "not talking to a bot",
        "tired of bots",
    ])
});

static SUPPORT_PATTERNS: LazyLock<Vec<NormalizedPattern>> = LazyLock::new(|| {
    normalize_patterns(&[
        // Contact support
        "contact support",
        "call support",
        "reach support",
        "get support",
        // Customer support
        "customer support",
        "customer service",
        "tech support",
        "technical support",
        // Help desk
        "help desk",
        "helpdesk",
        "support desk",
        // Talk to support
        "talk to support",
        "speak to support",
        "speak with support",
        "chat with support",
        // Need help
        "need real help",
        "need actual help",
        "help me now",
    ])
});

static QUIT_PATTERNS: LazyLock<Vec<NormalizedPattern>> = LazyLock::new(|| {
    normalize_patterns(&[
        // Give up
        "i give up",
        "give up",
        "giving up",
        // Quit/leaving
        "i'm going to quit",
        "i quit",
        "quitting",
        "i'm leaving",
        "i'm done",
        "i'm out",
        // Forget it
        "forget it",
        "forget this",
        "screw it",
        "screw this",
        // Never mind
        "never mind",
        "nevermind",
        "don't bother",
        "not worth it",
        // Hopeless
        "this is hopeless",
        // Going elsewhere
        "going elsewhere",
        "try somewhere else",
        "look elsewhere",
        "find another",
    ])
});

// ============================================================================
// Core Signal Types
// ============================================================================

/// Overall quality assessment for an agent interaction session
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum InteractionQuality {
    /// Excellent interaction with strong positive signals
    Excellent,
    /// Good interaction with mostly positive signals
    Good,
    /// Neutral interaction with mixed signals
    Neutral,
    /// Poor interaction with concerning signals
    Poor,
    /// Critical interaction with severe negative signals
    Severe,
}

/// Container for all computed signals for a conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalReport {
    /// Turn count and efficiency metrics
    pub turn_count: TurnCountSignal,
    /// Follow-up and repair frequency
    pub follow_up: FollowUpSignal,
    /// User frustration indicators
    pub frustration: FrustrationSignal,
    /// Repetition and looping behavior
    pub repetition: RepetitionSignal,
    /// Positive feedback indicators
    pub positive_feedback: PositiveFeedbackSignal,
    /// User escalation requests
    pub escalation: EscalationSignal,
    /// Overall quality assessment
    pub overall_quality: InteractionQuality,
    /// Human-readable summary
    pub summary: String,
}

// ============================================================================
// Individual Signal Types
// ============================================================================

/// Turn count and efficiency metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnCountSignal {
    /// Total number of turns (user-agent exchanges)
    pub total_turns: usize,
    /// Number of user messages
    pub user_turns: usize,
    /// Number of assistant messages
    pub assistant_turns: usize,
    /// Whether the turn count is concerning (> 7)
    pub is_concerning: bool,
    /// Whether the turn count is excessive (> 12)
    pub is_excessive: bool,
    /// Efficiency score (0.0-1.0, lower turns = higher score)
    pub efficiency_score: f64,
}

/// Follow-up and repair frequency signal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowUpSignal {
    /// Number of detected repair attempts
    pub repair_count: usize,
    /// Ratio of repairs to total user turns
    pub repair_ratio: f64,
    /// Whether repair ratio is concerning (> 0.3)
    pub is_concerning: bool,
    /// List of detected repair phrases
    pub repair_phrases: Vec<String>,
}

/// User frustration indicators
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrustrationSignal {
    /// Number of frustration indicators detected
    pub frustration_count: usize,
    /// Whether frustration is detected
    pub has_frustration: bool,
    /// Severity level (0-3: none, mild, moderate, severe)
    pub severity: u8,
    /// List of detected frustration indicators
    pub indicators: Vec<FrustrationIndicator>,
}

/// Individual frustration indicator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrustrationIndicator {
    /// Type of frustration detected
    pub indicator_type: FrustrationType,
    /// Message index where detected
    pub message_index: usize,
    /// Relevant text snippet
    pub snippet: String,
}

/// Types of frustration indicators
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FrustrationType {
    /// Negative sentiment detected
    NegativeSentiment,
    /// All caps typing
    AllCaps,
    /// Excessive punctuation
    ExcessivePunctuation,
    /// Profanity detected
    Profanity,
    /// Direct complaint
    DirectComplaint,
    /// Expression of confusion
    Confusion,
}

/// Repetition and looping behavior signal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepetitionSignal {
    /// Number of repetitions detected
    pub repetition_count: usize,
    /// Whether significant looping detected (> 2 repetitions)
    pub has_looping: bool,
    /// Severity level (0-3: none, mild, moderate, severe)
    pub severity: u8,
    /// List of detected repetitions
    pub repetitions: Vec<RepetitionInstance>,
}

/// Individual repetition instance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepetitionInstance {
    /// Message indices involved in repetition
    pub message_indices: Vec<usize>,
    /// Similarity score (0.0-1.0)
    pub similarity: f64,
    /// Type of repetition
    pub repetition_type: RepetitionType,
}

/// Types of repetition
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RepetitionType {
    /// Exact repetition
    Exact,
    /// Near-duplicate (high similarity)
    NearDuplicate,
    /// Semantic repetition (similar meaning)
    Semantic,
}

/// Positive feedback indicators
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositiveFeedbackSignal {
    /// Number of positive indicators detected
    pub positive_count: usize,
    /// Whether positive feedback is present
    pub has_positive_feedback: bool,
    /// Confidence score (0.0-1.0)
    pub confidence: f64,
    /// List of detected positive indicators
    pub indicators: Vec<PositiveIndicator>,
}

/// Individual positive indicator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositiveIndicator {
    /// Type of positive feedback
    pub indicator_type: PositiveType,
    /// Message index where detected
    pub message_index: usize,
    /// Relevant text snippet
    pub snippet: String,
}

/// Types of positive indicators
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PositiveType {
    /// Expression of gratitude
    Gratitude,
    /// Explicit satisfaction
    Satisfaction,
    /// Confirmation of success
    Success,
    /// Positive sentiment
    PositiveSentiment,
    /// Natural topic transition
    TopicTransition,
}

/// User escalation signal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationSignal {
    /// Whether escalation was requested
    pub escalation_requested: bool,
    /// Number of escalation requests
    pub escalation_count: usize,
    /// List of detected escalation requests
    pub requests: Vec<EscalationRequest>,
}

/// Individual escalation request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationRequest {
    /// Message index where detected
    pub message_index: usize,
    /// Relevant text snippet
    pub snippet: String,
    /// Type of escalation
    pub escalation_type: EscalationType,
}

/// Types of escalation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EscalationType {
    /// Request for human agent
    HumanAgent,
    /// Request for support
    Support,
    /// Threat to quit/leave
    ThreatToQuit,
    /// General help request
    HelpRequest,
}

// ============================================================================
// Signal Analyzer
// ============================================================================

/// Trait for analyzing conversation signals
pub trait SignalAnalyzer {
    /// Analyze a conversation and generate a complete signal report
    fn analyze(&self, messages: &[Message]) -> SignalReport;
}

/// Text-based implementation of signal analyzer that computes all signals from a message array
pub struct TextBasedSignalAnalyzer {
    /// Baseline expected turns for normal interactions
    baseline_turns: usize,
    /// Threshold for character ngram similarity (0.0-1.0)
    char_ngram_threshold: f64,
    /// Threshold for token cosine similarity (0.0-1.0)
    token_cosine_threshold: f64,
    /// Maximum message length in characters (prevents unbounded computation)
    max_message_length: usize,
    /// Maximum number of messages to process (prevents unbounded computation)
    max_messages: usize,
    /// Maximum window size for repetition detection (prevents O(n²) explosion)
    max_repetition_window: usize,
}

impl TextBasedSignalAnalyzer {
    /// Extract text content from MessageContent, skipping non-text content
    fn extract_text(content: &Option<hermesllm::apis::openai::MessageContent>) -> Option<String> {
        match content {
            Some(hermesllm::apis::openai::MessageContent::Text(text)) => Some(text.clone()),
            // Tool calls and other structured content are skipped
            _ => None,
        }
    }

    /// Create a new signal analyzer with default settings
    pub fn new() -> Self {
        Self {
            baseline_turns: 5,
            char_ngram_threshold: 0.50, // Lowered to handle typos and small edits realistically
            token_cosine_threshold: 0.60, // Lowered for better semantic match in varied contexts
            max_message_length: 2000,   // Prevent unbounded ngram generation
            max_messages: 100,          // Prevent unbounded message processing
            max_repetition_window: 20,  // Prevent O(n²) explosion in repetition detection
        }
    }

    /// Create a new signal analyzer with custom baseline
    pub fn with_baseline(baseline_turns: usize) -> Self {
        Self {
            baseline_turns,
            char_ngram_threshold: 0.50,
            token_cosine_threshold: 0.60,
            max_message_length: 2000,
            max_messages: 100,
            max_repetition_window: 20,
        }
    }

    /// Create a new signal analyzer with custom settings
    ///
    /// # Arguments
    /// * `baseline_turns` - Expected baseline turns for normal interactions
    /// * `char_ngram_threshold` - Threshold for character ngram similarity (0.0-1.0)
    /// * `token_cosine_threshold` - Threshold for token cosine similarity (0.0-1.0)
    pub fn with_settings(
        baseline_turns: usize,
        char_ngram_threshold: f64,
        token_cosine_threshold: f64,
    ) -> Self {
        Self {
            baseline_turns,
            char_ngram_threshold,
            token_cosine_threshold,
            max_message_length: 2000,
            max_messages: 100,
            max_repetition_window: 20,
        }
    }

    /// Create a new signal analyzer with full custom settings including computation limits
    ///
    /// # Arguments
    /// * `baseline_turns` - Expected baseline turns for normal interactions
    /// * `char_ngram_threshold` - Threshold for character ngram similarity (0.0-1.0)
    /// * `token_cosine_threshold` - Threshold for token cosine similarity (0.0-1.0)
    /// * `max_message_length` - Maximum characters per message to process
    /// * `max_messages` - Maximum number of messages to process
    /// * `max_repetition_window` - Maximum messages to compare for repetition detection
    pub fn with_full_settings(
        baseline_turns: usize,
        char_ngram_threshold: f64,
        token_cosine_threshold: f64,
        max_message_length: usize,
        max_messages: usize,
        max_repetition_window: usize,
    ) -> Self {
        Self {
            baseline_turns,
            char_ngram_threshold,
            token_cosine_threshold,
            max_message_length,
            max_messages,
            max_repetition_window,
        }
    }

    // ========================================================================
    // Individual Signal Analyzers
    // ========================================================================

    /// Analyze turn count and efficiency
    fn analyze_turn_count(&self, messages: &[Message]) -> TurnCountSignal {
        let mut user_turns = 0;
        let mut assistant_turns = 0;

        for message in messages {
            match message.role {
                Role::User => user_turns += 1,
                Role::Assistant => assistant_turns += 1,
                _ => {}
            }
        }

        let total_turns = user_turns + assistant_turns;
        let is_concerning = total_turns > 7;
        let is_excessive = total_turns > 12;

        // Calculate efficiency score (exponential decay after baseline)
        let efficiency_score = if total_turns == 0 || total_turns <= self.baseline_turns {
            1.0
        } else {
            let excess = total_turns - self.baseline_turns;
            1.0 / (1.0 + (excess as f64 * 0.3))
        };

        TurnCountSignal {
            total_turns,
            user_turns,
            assistant_turns,
            is_concerning,
            is_excessive,
            efficiency_score,
        }
    }

    /// Analyze follow-up and repair frequency
    fn analyze_follow_up(
        &self,
        normalized_messages: &[(usize, Role, NormalizedMessage)],
    ) -> FollowUpSignal {
        let mut repair_count = 0;
        let mut repair_phrases = Vec::new();
        let mut user_turn_count = 0;

        for (i, role, norm_msg) in normalized_messages {
            if *role != Role::User {
                continue;
            }

            user_turn_count += 1;

            // Use per-turn boolean to prevent double-counting
            let mut found_in_turn = false;

            // Use pre-computed patterns for fast matching
            for pattern in REPAIR_PATTERNS.iter() {
                if norm_msg.matches_normalized_pattern(
                    pattern,
                    self.char_ngram_threshold,
                    self.token_cosine_threshold,
                ) {
                    repair_count += 1;
                    repair_phrases.push(format!("Turn {}: '{}'", i + 1, pattern.raw));
                    found_in_turn = true;
                    break;
                }
            }

            // Only check for semantic similarity if no pattern matched
            if !found_in_turn && *i >= 2 {
                // Find previous user message
                for j in (0..*i).rev() {
                    let (_, prev_role, prev_norm_msg) = &normalized_messages[j];
                    if *prev_role == Role::User {
                        if self.is_similar_rephrase(norm_msg, prev_norm_msg) {
                            repair_count += 1;
                            repair_phrases
                                .push(format!("Turn {}: Similar rephrase detected", i + 1));
                        }
                        break;
                    }
                }
            }
        }

        let repair_ratio = if user_turn_count == 0 {
            0.0
        } else {
            repair_count as f64 / user_turn_count as f64
        };

        let is_concerning = repair_ratio > 0.3;

        FollowUpSignal {
            repair_count,
            repair_ratio,
            is_concerning,
            repair_phrases,
        }
    }

    /// Analyze user frustration indicators
    fn analyze_frustration(
        &self,
        normalized_messages: &[(usize, Role, NormalizedMessage)],
    ) -> FrustrationSignal {
        let mut indicators = Vec::new();

        // Profanity list - only as standalone tokens, not substrings
        let profanity_tokens = [
            "damn", "damnit", "crap", "wtf", "ffs", "bullshit", "shit", "fuck", "fucking",
        ];

        for (i, role, norm_msg) in normalized_messages {
            if *role != Role::User {
                continue;
            }

            let text = &norm_msg.raw;

            // Check for all caps (at least 10 chars and 80% uppercase)
            let alpha_chars: String = text.chars().filter(|c| c.is_alphabetic()).collect();
            if alpha_chars.len() >= 10 {
                let upper_count = alpha_chars.chars().filter(|c| c.is_uppercase()).count();
                let upper_ratio = upper_count as f64 / alpha_chars.len() as f64;
                if upper_ratio >= 0.8 {
                    indicators.push(FrustrationIndicator {
                        indicator_type: FrustrationType::AllCaps,
                        message_index: *i,
                        snippet: text.chars().take(50).collect(),
                    });
                }
            }

            // Check for excessive punctuation
            let question_marks = text.matches('?').count();
            let exclamation_marks = text.matches('!').count();
            if question_marks >= 3 || exclamation_marks >= 3 {
                indicators.push(FrustrationIndicator {
                    indicator_type: FrustrationType::ExcessivePunctuation,
                    message_index: *i,
                    snippet: text.chars().take(50).collect(),
                });
            }

            // Check for complaint patterns using pre-computed patterns
            for pattern in COMPLAINT_PATTERNS.iter() {
                if norm_msg.matches_normalized_pattern(
                    pattern,
                    self.char_ngram_threshold,
                    self.token_cosine_threshold,
                ) {
                    indicators.push(FrustrationIndicator {
                        indicator_type: FrustrationType::DirectComplaint,
                        message_index: *i,
                        snippet: pattern.raw.clone(),
                    });
                    break;
                }
            }

            // Check for confusion patterns using pre-computed patterns
            for pattern in CONFUSION_PATTERNS.iter() {
                if norm_msg.matches_normalized_pattern(
                    pattern,
                    self.char_ngram_threshold,
                    self.token_cosine_threshold,
                ) {
                    indicators.push(FrustrationIndicator {
                        indicator_type: FrustrationType::Confusion,
                        message_index: *i,
                        snippet: pattern.raw.clone(),
                    });
                    break;
                }
            }

            // Check for profanity (token-based, not substring)
            for token in &profanity_tokens {
                if norm_msg.contains_token(token) {
                    indicators.push(FrustrationIndicator {
                        indicator_type: FrustrationType::Profanity,
                        message_index: *i,
                        snippet: token.to_string(),
                    });
                    break;
                }
            }
        }

        let frustration_count = indicators.len();
        let has_frustration = frustration_count > 0;

        // Calculate severity
        let severity = if frustration_count == 0 {
            0
        } else if frustration_count <= 2 {
            1
        } else if frustration_count <= 4 {
            2
        } else {
            3
        };

        FrustrationSignal {
            frustration_count,
            has_frustration,
            severity,
            indicators,
        }
    }

    /// Analyze repetition and looping behavior
    fn analyze_repetition(
        &self,
        normalized_messages: &[(usize, Role, NormalizedMessage)],
    ) -> RepetitionSignal {
        let mut repetitions = Vec::new();

        // Collect assistant messages with normalized content
        let assistant_messages: Vec<(usize, &NormalizedMessage)> = normalized_messages
            .iter()
            .filter(|(_, role, _)| *role == Role::Assistant)
            .map(|(i, _, norm_msg)| (*i, norm_msg))
            .collect();

        // Limit the window size to prevent O(n²) explosion
        // Only compare messages within the max_repetition_window
        let window_size = self.max_repetition_window.min(assistant_messages.len());

        // Check for exact or near-duplicate responses using bigram similarity
        // Only compare within the sliding window
        for i in 0..assistant_messages.len() {
            let window_start = i + 1;
            let window_end = (i + 1 + window_size).min(assistant_messages.len());

            for j in window_start..window_end {
                let (idx_i, norm_msg_i) = &assistant_messages[i];
                let (idx_j, norm_msg_j) = &assistant_messages[j];

                // Skip if messages are too short
                if norm_msg_i.tokens.len() < 5 || norm_msg_j.tokens.len() < 5 {
                    continue;
                }

                // Calculate bigram-based similarity (more accurate for near-duplicates)
                let similarity = self.calculate_bigram_similarity(norm_msg_i, norm_msg_j);

                // Exact match - lowered from 0.95 to 0.85 for bigram similarity
                if similarity >= 0.85 {
                    repetitions.push(RepetitionInstance {
                        message_indices: vec![*idx_i, *idx_j],
                        similarity,
                        repetition_type: RepetitionType::Exact,
                    });
                }
                // Near duplicate - lowered from 0.75 to 0.50 to catch subtle repetitions
                else if similarity >= 0.50 {
                    repetitions.push(RepetitionInstance {
                        message_indices: vec![*idx_i, *idx_j],
                        similarity,
                        repetition_type: RepetitionType::NearDuplicate,
                    });
                }
            }
        }

        let repetition_count = repetitions.len();
        let has_looping = repetition_count > 2;

        let severity = if repetition_count == 0 {
            0
        } else if repetition_count <= 2 {
            1
        } else if repetition_count <= 4 {
            2
        } else {
            3
        };

        RepetitionSignal {
            repetition_count,
            has_looping,
            severity,
            repetitions,
        }
    }

    /// Calculate bigram similarity using cached bigram sets
    fn calculate_bigram_similarity(
        &self,
        norm_msg1: &NormalizedMessage,
        norm_msg2: &NormalizedMessage,
    ) -> f64 {
        // Use pre-cached bigram sets for O(1) lookups
        let set1 = &norm_msg1.bigram_set;
        let set2 = &norm_msg2.bigram_set;

        if set1.is_empty() && set2.is_empty() {
            return 1.0; // Both empty = identical
        }

        if set1.is_empty() || set2.is_empty() {
            return 0.0;
        }

        let intersection = set1.intersection(set2).count();
        let union = set1.union(set2).count();

        if union == 0 {
            return 0.0;
        }

        intersection as f64 / union as f64
    }

    /// Analyze positive feedback indicators
    fn analyze_positive_feedback(
        &self,
        normalized_messages: &[(usize, Role, NormalizedMessage)],
    ) -> PositiveFeedbackSignal {
        let mut indicators = Vec::new();

        for (i, role, norm_msg) in normalized_messages {
            if *role != Role::User {
                continue;
            }

            // Use per-turn boolean to prevent double-counting
            let mut found_in_turn = false;

            // Check gratitude using pre-computed patterns
            for pattern in GRATITUDE_PATTERNS.iter() {
                if norm_msg.matches_normalized_pattern(
                    pattern,
                    self.char_ngram_threshold,
                    self.token_cosine_threshold,
                ) {
                    indicators.push(PositiveIndicator {
                        indicator_type: PositiveType::Gratitude,
                        message_index: *i,
                        snippet: pattern.raw.clone(),
                    });
                    found_in_turn = true;
                    break;
                }
            }

            if found_in_turn {
                continue;
            }

            // Check satisfaction using pre-computed patterns
            for pattern in SATISFACTION_PATTERNS.iter() {
                if norm_msg.matches_normalized_pattern(
                    pattern,
                    self.char_ngram_threshold,
                    self.token_cosine_threshold,
                ) {
                    indicators.push(PositiveIndicator {
                        indicator_type: PositiveType::Satisfaction,
                        message_index: *i,
                        snippet: pattern.raw.clone(),
                    });
                    found_in_turn = true;
                    break;
                }
            }

            if found_in_turn {
                continue;
            }

            // Check success confirmation using pre-computed patterns
            for pattern in SUCCESS_PATTERNS.iter() {
                if norm_msg.matches_normalized_pattern(
                    pattern,
                    self.char_ngram_threshold,
                    self.token_cosine_threshold,
                ) {
                    indicators.push(PositiveIndicator {
                        indicator_type: PositiveType::Success,
                        message_index: *i,
                        snippet: pattern.raw.clone(),
                    });
                    break;
                }
            }
        }

        let positive_count = indicators.len();
        let has_positive_feedback = positive_count > 0;

        // Calculate confidence based on number and diversity of indicators
        let confidence = if positive_count == 0 {
            0.0
        } else if positive_count == 1 {
            0.6
        } else if positive_count == 2 {
            0.8
        } else {
            0.95
        };

        PositiveFeedbackSignal {
            positive_count,
            has_positive_feedback,
            confidence,
            indicators,
        }
    }

    /// Analyze user escalation requests
    fn analyze_escalation(
        &self,
        normalized_messages: &[(usize, Role, NormalizedMessage)],
    ) -> EscalationSignal {
        let mut requests = Vec::new();

        for (i, role, norm_msg) in normalized_messages {
            if *role != Role::User {
                continue;
            }

            let mut found_human_agent = false;

            // Check for human agent request using pre-computed patterns
            for pattern in HUMAN_AGENT_PATTERNS.iter() {
                if norm_msg.matches_normalized_pattern(
                    pattern,
                    self.char_ngram_threshold,
                    self.token_cosine_threshold,
                ) {
                    requests.push(EscalationRequest {
                        message_index: *i,
                        snippet: pattern.raw.clone(),
                        escalation_type: EscalationType::HumanAgent,
                    });
                    found_human_agent = true;
                    break;
                }
            }

            // Check for support request (only if no human agent request found)
            // HumanAgent and Support are too similar and often match the same phrase
            if !found_human_agent {
                for pattern in SUPPORT_PATTERNS.iter() {
                    if norm_msg.matches_normalized_pattern(
                        pattern,
                        self.char_ngram_threshold,
                        self.token_cosine_threshold,
                    ) {
                        requests.push(EscalationRequest {
                            message_index: *i,
                            snippet: pattern.raw.clone(),
                            escalation_type: EscalationType::Support,
                        });
                        break;
                    }
                }
            }

            // Check for quit threats (independent of HumanAgent/Support)
            // A message can contain both "give up" (quit) and "speak to human" (escalation)
            for pattern in QUIT_PATTERNS.iter() {
                if norm_msg.matches_normalized_pattern(
                    pattern,
                    self.char_ngram_threshold,
                    self.token_cosine_threshold,
                ) {
                    requests.push(EscalationRequest {
                        message_index: *i,
                        snippet: pattern.raw.clone(),
                        escalation_type: EscalationType::ThreatToQuit,
                    });
                    break;
                }
            }
        }

        let escalation_count = requests.len();
        let escalation_requested = escalation_count > 0;

        EscalationSignal {
            escalation_requested,
            escalation_count,
            requests,
        }
    }

    // ========================================================================
    // Helper Methods
    // ========================================================================

    /// Check if two messages are similar rephrases
    fn is_similar_rephrase(
        &self,
        norm_msg1: &NormalizedMessage,
        norm_msg2: &NormalizedMessage,
    ) -> bool {
        // Skip if too short
        if norm_msg1.tokens.len() < 3 || norm_msg2.tokens.len() < 3 {
            return false;
        }

        // Common stopwords to downweight
        let stopwords: HashSet<&str> = [
            "i", "me", "my", "you", "the", "a", "an", "is", "are", "was", "were", "to", "with",
            "for", "of", "at", "by", "in", "on", "it", "this", "that", "can", "could", "do",
            "does", "did", "will", "would", "should", "be",
        ]
        .iter()
        .cloned()
        .collect();

        // Filter out stopwords for meaningful overlap
        let tokens1: HashSet<_> = norm_msg1
            .tokens
            .iter()
            .filter(|t| !stopwords.contains(t.as_str()))
            .collect();
        let tokens2: HashSet<_> = norm_msg2
            .tokens
            .iter()
            .filter(|t| !stopwords.contains(t.as_str()))
            .collect();

        // Need at least 2 non-stopword tokens
        if tokens1.len() < 2 || tokens2.len() < 2 {
            return false;
        }

        let intersection = tokens1.intersection(&tokens2).count();
        let min_size = tokens1.len().min(tokens2.len());

        // High overlap suggests rephrase
        let overlap_ratio = intersection as f64 / min_size as f64;
        overlap_ratio >= 0.6
    }

    /// Assess overall interaction quality based on all signals
    fn assess_overall_quality(
        &self,
        turn_count: &TurnCountSignal,
        follow_up: &FollowUpSignal,
        frustration: &FrustrationSignal,
        repetition: &RepetitionSignal,
        positive: &PositiveFeedbackSignal,
        escalation: &EscalationSignal,
    ) -> InteractionQuality {
        // Critical conditions - immediate fail
        if escalation.escalation_requested
            || frustration.severity >= 3
            || repetition.severity >= 3
            || turn_count.is_excessive
        {
            return InteractionQuality::Severe;
        }

        // Calculate quality score
        let mut score = 50.0; // Start at neutral

        // Positive factors
        if positive.has_positive_feedback {
            score += 20.0 * positive.confidence;
        }
        score += turn_count.efficiency_score * 10.0;

        // Negative factors
        if frustration.has_frustration {
            score -= frustration.severity as f64 * 10.00;
        }
        if follow_up.is_concerning {
            score -= 15.0;
        }
        if repetition.has_looping {
            score -= repetition.severity as f64 * 8.0;
        }
        if turn_count.is_concerning {
            score -= 10.0;
        }

        // Map score to quality level
        if score >= 75.0 {
            InteractionQuality::Excellent
        } else if score >= 60.0 {
            InteractionQuality::Good
        } else if score >= 40.0 {
            InteractionQuality::Neutral
        } else if score >= 25.0 {
            InteractionQuality::Poor
        } else {
            InteractionQuality::Severe
        }
    }

    /// Generate human-readable summary
    #[allow(clippy::too_many_arguments)]
    fn generate_summary(
        &self,
        turn_count: &TurnCountSignal,
        follow_up: &FollowUpSignal,
        frustration: &FrustrationSignal,
        repetition: &RepetitionSignal,
        positive: &PositiveFeedbackSignal,
        escalation: &EscalationSignal,
        quality: &InteractionQuality,
    ) -> String {
        let mut summary_parts = Vec::new();

        summary_parts.push(format!("Overall Quality: {:?}", quality));

        summary_parts.push(format!(
            "Turn Count: {} turns (efficiency: {:.1}%)",
            turn_count.total_turns,
            turn_count.efficiency_score * 100.0
        ));

        if follow_up.is_concerning {
            summary_parts.push(format!(
                "⚠️ High repair rate: {:.1}% of user turns",
                follow_up.repair_ratio * 100.0
            ));
        }

        if frustration.has_frustration {
            summary_parts.push(format!(
                "⚠️ Frustration detected: {} indicators (severity: {})",
                frustration.frustration_count, frustration.severity
            ));
        }

        if repetition.has_looping {
            summary_parts.push(format!(
                "⚠️ Looping detected: {} repetitions",
                repetition.repetition_count
            ));
        }

        if positive.has_positive_feedback {
            summary_parts.push(format!(
                "✓ Positive feedback: {} indicators",
                positive.positive_count
            ));
        }

        if escalation.escalation_requested {
            summary_parts.push(format!(
                "⚠️ Escalation requested: {} requests",
                escalation.escalation_count
            ));
        }

        summary_parts.join(" | ")
    }
}

impl SignalAnalyzer for TextBasedSignalAnalyzer {
    fn analyze(&self, messages: &[Message]) -> SignalReport {
        // Limit the number of messages to process (take most recent messages)
        let messages_to_process = if messages.len() > self.max_messages {
            &messages[messages.len() - self.max_messages..]
        } else {
            messages
        };

        // Preprocess all messages once, filtering out non-text content (tool calls, etc.)
        // and truncating long messages
        let normalized_messages: Vec<(usize, Role, NormalizedMessage)> = messages_to_process
            .iter()
            .enumerate()
            .filter_map(|(i, msg)| {
                Self::extract_text(&msg.content).map(|text| {
                    (
                        i,
                        msg.role.clone(),
                        NormalizedMessage::from_text_with_limit(&text, self.max_message_length),
                    )
                })
            })
            .collect();

        let turn_count = self.analyze_turn_count(messages_to_process);
        let follow_up = self.analyze_follow_up(&normalized_messages);
        let frustration = self.analyze_frustration(&normalized_messages);
        let repetition = self.analyze_repetition(&normalized_messages);
        let positive_feedback = self.analyze_positive_feedback(&normalized_messages);
        let escalation = self.analyze_escalation(&normalized_messages);

        let overall_quality = self.assess_overall_quality(
            &turn_count,
            &follow_up,
            &frustration,
            &repetition,
            &positive_feedback,
            &escalation,
        );

        let summary = self.generate_summary(
            &turn_count,
            &follow_up,
            &frustration,
            &repetition,
            &positive_feedback,
            &escalation,
            &overall_quality,
        );

        SignalReport {
            turn_count,
            follow_up,
            frustration,
            repetition,
            positive_feedback,
            escalation,
            overall_quality,
            summary,
        }
    }
}

impl Default for TextBasedSignalAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use hermesllm::apis::openai::MessageContent;
    use hermesllm::transforms::lib::ExtractText;
    use std::time::Instant;

    fn create_message(role: Role, content: &str) -> Message {
        Message {
            role,
            content: Some(MessageContent::Text(content.to_string())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    // ========================================================================
    // Tests for New Similarity Methods
    // ========================================================================

    #[test]
    fn test_char_ngram_similarity_exact_match() {
        let msg = NormalizedMessage::from_text("thank you very much");
        let similarity = msg.char_ngram_similarity("thank you very much");
        assert!(
            similarity > 0.95,
            "Exact match should have very high similarity"
        );
    }

    #[test]
    fn test_char_ngram_similarity_typo() {
        let msg = NormalizedMessage::from_text("thank you very much");
        // Common typo: "thnks" instead of "thanks"
        let similarity = msg.char_ngram_similarity("thnks you very much");
        assert!(
            similarity > 0.50,
            "Should handle single-character typo with decent similarity: {}",
            similarity
        );
    }

    #[test]
    fn test_char_ngram_similarity_small_edit() {
        let msg = NormalizedMessage::from_text("this doesn't work");
        let similarity = msg.char_ngram_similarity("this doesnt work");
        assert!(
            similarity > 0.70,
            "Should handle punctuation removal gracefully: {}",
            similarity
        );
    }

    #[test]
    fn test_char_ngram_similarity_word_insertion() {
        let msg = NormalizedMessage::from_text("i don't understand");
        let similarity = msg.char_ngram_similarity("i really don't understand");
        assert!(
            similarity > 0.40,
            "Should be robust to word insertions: {}",
            similarity
        );
    }

    #[test]
    fn test_token_cosine_similarity_exact_match() {
        let msg = NormalizedMessage::from_text("this is not helpful");
        let similarity = msg.token_cosine_similarity("this is not helpful");
        assert!(
            (similarity - 1.0).abs() < 0.01,
            "Exact match should have cosine similarity of 1.0"
        );
    }

    #[test]
    fn test_token_cosine_similarity_word_order() {
        let msg = NormalizedMessage::from_text("not helpful at all");
        let similarity = msg.token_cosine_similarity("helpful not at all");
        assert!(
            similarity > 0.95,
            "Should be robust to word order changes: {}",
            similarity
        );
    }

    #[test]
    fn test_token_cosine_similarity_frequency() {
        let msg = NormalizedMessage::from_text("help help help please");
        let similarity = msg.token_cosine_similarity("help please");
        assert!(
            similarity > 0.7 && similarity < 1.0,
            "Should account for frequency differences: {}",
            similarity
        );
    }

    #[test]
    fn test_token_cosine_similarity_long_message_with_context() {
        let msg = NormalizedMessage::from_text(
            "I've been trying to set up my account for the past hour \
             and the verification email never arrived. I checked my spam folder \
             and still nothing. This is really frustrating and not helpful at all.",
        );
        let similarity = msg.token_cosine_similarity("not helpful");
        assert!(
            similarity > 0.15 && similarity < 0.7,
            "Should detect pattern in long message with lower but non-zero similarity: {}",
            similarity
        );
    }

    #[test]
    fn test_layered_matching_exact_hit() {
        let msg = NormalizedMessage::from_text("thank you so much");
        assert!(
            msg.layered_contains_phrase("thank you", 0.50, 0.60),
            "Should match exact phrase in Layer 0"
        );
    }

    #[test]
    fn test_layered_matching_typo_hit() {
        // Test that shows layered matching is more robust than exact matching alone
        let msg = NormalizedMessage::from_text("it doesnt work for me");

        // "doesnt work" should match "doesn't work" via character ngrams (high overlap)
        assert!(
            msg.layered_contains_phrase("doesn't work", 0.50, 0.60),
            "Should match 'doesnt work' to 'doesn't work' via character ngrams"
        );
    }

    #[test]
    fn test_layered_matching_word_order_hit() {
        let msg = NormalizedMessage::from_text("helpful not very");
        assert!(
            msg.layered_contains_phrase("not helpful", 0.50, 0.60),
            "Should match reordered words via token cosine in Layer 2"
        );
    }

    #[test]
    fn test_layered_matching_long_message_with_pattern() {
        let msg = NormalizedMessage::from_text(
            "I've tried everything and followed all the instructions \
             but this is not helpful at all and I'm getting frustrated",
        );
        assert!(
            msg.layered_contains_phrase("not helpful", 0.50, 0.60),
            "Should detect pattern buried in long message"
        );
    }

    #[test]
    fn test_layered_matching_no_match() {
        let msg = NormalizedMessage::from_text("everything is working perfectly");
        assert!(
            !msg.layered_contains_phrase("not helpful", 0.50, 0.60),
            "Should not match completely different content"
        );
    }

    #[test]
    fn test_char_ngram_vs_token_cosine_tradeoffs() {
        // Character ngrams handle character-level changes well
        let msg1 = NormalizedMessage::from_text("this doesnt work");
        let char_sim1 = msg1.char_ngram_similarity("this doesn't work");
        assert!(
            char_sim1 > 0.70,
            "Character ngrams should handle punctuation: {}",
            char_sim1
        );

        // Token cosine is better for word order and long messages with semantic overlap
        let msg2 =
            NormalizedMessage::from_text("I really appreciate all your help with this issue today");
        let token_sim2 = msg2.token_cosine_similarity("thank you for help");
        assert!(
            token_sim2 > 0.15,
            "Token cosine should detect semantic overlap: {}",
            token_sim2
        );
    }

    // ========================================================================
    // Existing Tests
    // ========================================================================

    fn preprocess_messages(messages: &[Message]) -> Vec<(usize, Role, NormalizedMessage)> {
        messages
            .iter()
            .enumerate()
            .map(|(i, msg)| {
                let text = msg.content.extract_text();
                (i, msg.role.clone(), NormalizedMessage::from_text(&text))
            })
            .collect()
    }

    #[test]
    fn test_turn_count_efficient() {
        let start = Instant::now();
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![
            create_message(Role::User, "Hello"),
            create_message(Role::Assistant, "Hi! How can I help?"),
            create_message(Role::User, "Thanks!"),
        ];

        let signal = analyzer.analyze_turn_count(&messages);
        assert_eq!(signal.total_turns, 3);
        assert_eq!(signal.user_turns, 2);
        assert_eq!(signal.assistant_turns, 1);
        assert!(!signal.is_concerning);
        assert!(!signal.is_excessive);
        assert!(signal.efficiency_score > 0.9);
        println!("test_turn_count_efficient took: {:?}", start.elapsed());
    }

    #[test]
    fn test_turn_count_excessive() {
        let start = Instant::now();
        let analyzer = TextBasedSignalAnalyzer::new();
        let mut messages = Vec::new();
        for i in 0..15 {
            messages.push(create_message(
                if i % 2 == 0 {
                    Role::User
                } else {
                    Role::Assistant
                },
                &format!("Message {}", i),
            ));
        }

        let signal = analyzer.analyze_turn_count(&messages);
        assert_eq!(signal.total_turns, 15);
        assert!(signal.is_concerning);
        assert!(signal.is_excessive);
        assert!(signal.efficiency_score < 0.5);
        println!("test_turn_count_excessive took: {:?}", start.elapsed());
    }

    #[test]
    fn test_follow_up_detection() {
        let start = Instant::now();
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![
            create_message(Role::User, "Show me restaurants"),
            create_message(Role::Assistant, "Here are some options"),
            create_message(Role::User, "No, I meant Italian restaurants"),
            create_message(Role::Assistant, "Here are Italian restaurants"),
        ];

        let normalized_messages = preprocess_messages(&messages);
        let signal = analyzer.analyze_follow_up(&normalized_messages);
        assert_eq!(signal.repair_count, 1);
        assert!(signal.repair_ratio > 0.0);
        println!("test_follow_up_detection took: {:?}", start.elapsed());
    }

    #[test]
    fn test_frustration_detection() {
        let start = Instant::now();
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![
            create_message(Role::User, "THIS IS RIDICULOUS!!!"),
            create_message(Role::Assistant, "I apologize for the frustration"),
            create_message(Role::User, "This doesn't work at all"),
        ];

        let normalized_messages = preprocess_messages(&messages);
        let signal = analyzer.analyze_frustration(&normalized_messages);
        assert!(signal.has_frustration);
        assert!(signal.frustration_count >= 2);
        assert!(signal.severity > 0);
        println!("test_frustration_detection took: {:?}", start.elapsed());
    }

    #[test]
    fn test_positive_feedback_detection() {
        let start = Instant::now();
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![
            create_message(Role::User, "Can you help me?"),
            create_message(Role::Assistant, "Sure!"),
            create_message(Role::User, "Thank you! That's exactly what I needed."),
        ];

        let normalized_messages = preprocess_messages(&messages);
        let signal = analyzer.analyze_positive_feedback(&normalized_messages);
        assert!(signal.has_positive_feedback);
        assert!(signal.positive_count >= 1);
        assert!(signal.confidence > 0.5);
        println!(
            "test_positive_feedback_detection took: {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn test_escalation_detection() {
        let start = Instant::now();
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![
            create_message(Role::User, "This isn't working"),
            create_message(Role::Assistant, "Let me help"),
            create_message(Role::User, "I need to speak to a human agent"),
        ];

        let normalized_messages = preprocess_messages(&messages);
        let signal = analyzer.analyze_escalation(&normalized_messages);
        assert!(signal.escalation_requested);
        assert_eq!(signal.escalation_count, 1);
        println!("test_escalation_detection took: {:?}", start.elapsed());
    }

    #[test]
    fn test_repetition_detection() {
        let start = Instant::now();
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![
            create_message(Role::User, "What's the weather?"),
            create_message(
                Role::Assistant,
                "I can help you with the weather information",
            ),
            create_message(Role::User, "Show me the forecast"),
            create_message(Role::Assistant, "Sure, I can help you with the forecast"),
            create_message(Role::User, "Stop repeating yourself"),
        ];

        let normalized_messages = preprocess_messages(&messages);
        let signal = analyzer.analyze_repetition(&normalized_messages);

        for rep in &signal.repetitions {
            println!(
                "  - Messages {:?}, similarity: {:.3}, type: {:?}",
                rep.message_indices, rep.similarity, rep.repetition_type
            );
        }

        assert!(signal.repetition_count > 0,
                "Should detect the subtle repetition between 'I can help you with the weather information' \
                 and 'Sure, I can help you with the forecast'");
        println!("test_repetition_detection took: {:?}", start.elapsed());
    }

    #[test]
    fn test_full_analysis_excellent() {
        let start = Instant::now();
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![
            create_message(Role::User, "I need to book a flight"),
            create_message(Role::Assistant, "Sure! Where would you like to go?"),
            create_message(Role::User, "New York"),
            create_message(Role::Assistant, "Great! I found several options."),
            create_message(Role::User, "Perfect!"),
        ];

        let report = analyzer.analyze(&messages);
        assert!(matches!(
            report.overall_quality,
            InteractionQuality::Excellent | InteractionQuality::Good
        ));
        assert!(report.positive_feedback.has_positive_feedback);
        assert!(!report.frustration.has_frustration);
        println!("test_full_analysis_excellent took: {:?}", start.elapsed());
    }

    #[test]
    fn test_full_analysis_poor() {
        let start = Instant::now();
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![
            create_message(Role::User, "Help me"),
            create_message(Role::Assistant, "How can I assist?"),
            create_message(Role::User, "No, I meant something else"),
            create_message(Role::Assistant, "What do you need?"),
            create_message(Role::User, "THIS DOESN'T WORK!!!"),
            create_message(Role::Assistant, "I apologize"),
            create_message(Role::User, "Let me speak to a human"),
        ];

        let report = analyzer.analyze(&messages);
        assert!(matches!(
            report.overall_quality,
            InteractionQuality::Poor | InteractionQuality::Severe
        ));
        assert!(report.frustration.has_frustration);
        assert!(report.escalation.escalation_requested);
        println!("test_full_analysis_poor took: {:?}", start.elapsed());
    }

    #[test]
    fn test_fuzzy_matching_gratitude() {
        let start = Instant::now();
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![
            create_message(Role::User, "Can you help me?"),
            create_message(Role::Assistant, "Sure!"),
            create_message(Role::User, "thnaks! that's exactly what i needed."),
        ];

        let normalized_messages = preprocess_messages(&messages);
        let signal = analyzer.analyze_positive_feedback(&normalized_messages);
        assert!(signal.has_positive_feedback);
        assert!(signal.positive_count >= 1);
        println!("test_fuzzy_matching_gratitude took: {:?}", start.elapsed());
    }

    #[test]
    fn test_fuzzy_matching_escalation() {
        let start = Instant::now();
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![
            create_message(Role::User, "This isn't working"),
            create_message(Role::Assistant, "Let me help"),
            create_message(Role::User, "i need to speek to a human agnet"),
        ];

        let normalized_messages = preprocess_messages(&messages);
        let signal = analyzer.analyze_escalation(&normalized_messages);
        assert!(signal.escalation_requested);
        assert_eq!(signal.escalation_count, 1);
        println!("test_fuzzy_matching_escalation took: {:?}", start.elapsed());
    }

    #[test]
    fn test_fuzzy_matching_repair() {
        let start = Instant::now();
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![
            create_message(Role::User, "Show me restaurants"),
            create_message(Role::Assistant, "Here are some options"),
            create_message(Role::User, "no i ment Italian restaurants"),
            create_message(Role::Assistant, "Here are Italian restaurants"),
        ];

        let normalized_messages = preprocess_messages(&messages);
        let signal = analyzer.analyze_follow_up(&normalized_messages);
        assert!(signal.repair_count >= 1);
        println!("test_fuzzy_matching_repair took: {:?}", start.elapsed());
    }

    #[test]
    fn test_fuzzy_matching_complaint() {
        let start = Instant::now();
        let analyzer = TextBasedSignalAnalyzer::new();
        // Use a complaint that should match - "doesnt work" is close enough to "doesn't work"
        let messages = vec![
            create_message(Role::User, "this doesnt work at all"), // Common typo: missing apostrophe
            create_message(Role::Assistant, "I apologize"),
        ];

        let normalized_messages = preprocess_messages(&messages);
        let signal = analyzer.analyze_frustration(&normalized_messages);

        // The layered matching should catch this via character ngrams or token cosine
        // "doesnt work" has high character-level similarity to "doesn't work"
        assert!(
            signal.has_frustration,
            "Should detect frustration from complaint pattern"
        );
        assert!(signal.frustration_count >= 1);
        println!("test_fuzzy_matching_complaint took: {:?}", start.elapsed());
    }

    #[test]
    fn test_exact_match_priority() {
        let start = Instant::now();
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![create_message(Role::User, "thank you so much")];

        let normalized_messages = preprocess_messages(&messages);
        let signal = analyzer.analyze_positive_feedback(&normalized_messages);
        assert!(signal.has_positive_feedback);
        // Should detect exact match, not fuzzy
        assert!(signal.indicators[0].snippet.contains("thank you"));
        assert!(!signal.indicators[0].snippet.contains("fuzzy"));
        println!("test_exact_match_priority took: {:?}", start.elapsed());
    }

    // ========================================================================
    // Anti-Tests: Verify fixes stay fixed
    // ========================================================================

    #[test]
    fn test_hello_not_profanity() {
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![create_message(Role::User, "hello there")];

        let normalized_messages = preprocess_messages(&messages);
        let signal = analyzer.analyze_frustration(&normalized_messages);
        assert!(
            !signal.has_frustration,
            "\"hello\" should not trigger profanity detection"
        );
    }

    #[test]
    fn test_prepare_not_escalation() {
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![create_message(
            Role::User,
            "Can you help me prepare for the meeting?",
        )];

        let normalized_messages = preprocess_messages(&messages);
        let signal = analyzer.analyze_escalation(&normalized_messages);
        assert!(
            !signal.escalation_requested,
            "\"prepare\" should not trigger escalation (rep pattern removed)"
        );
    }

    #[test]
    fn test_unicode_apostrophe_confusion() {
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![
            create_message(Role::User, "I'm confused"), // Unicode apostrophe
        ];

        let normalized_messages = preprocess_messages(&messages);
        let signal = analyzer.analyze_frustration(&normalized_messages);
        assert!(
            signal.has_frustration,
            "Unicode apostrophe 'I'm confused' should trigger confusion"
        );
    }

    #[test]
    fn test_unicode_quotes_work() {
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![create_message(
            Role::User,
            "\u{201C}doesn\u{2019}t work\u{201D} with unicode quotes",
        )];

        let normalized_messages = preprocess_messages(&messages);
        let signal = analyzer.analyze_frustration(&normalized_messages);
        assert!(
            signal.has_frustration,
            "Unicode quotes should be normalized and match patterns"
        );
    }

    #[test]
    fn test_absolute_not_profanity() {
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![create_message(Role::User, "That's absolute nonsense")];

        let normalized_messages = preprocess_messages(&messages);
        let signal = analyzer.analyze_frustration(&normalized_messages);
        // Should match on "nonsense" logic, not on "bs" substring
        let has_bs_match = signal
            .indicators
            .iter()
            .any(|ind| ind.snippet.contains("bs"));
        assert!(
            !has_bs_match,
            "\"absolute\" should not trigger 'bs' profanity match"
        );
    }

    #[test]
    fn test_stopwords_not_rephrase() {
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![
            create_message(Role::User, "Help me with X"),
            create_message(Role::Assistant, "Sure"),
            create_message(Role::User, "Help me with Y"),
        ];

        let normalized_messages = preprocess_messages(&messages);
        let signal = analyzer.analyze_follow_up(&normalized_messages);
        // Should not detect as rephrase since only stopwords overlap
        assert_eq!(
            signal.repair_count, 0,
            "Messages with only stopword overlap should not be rephrases"
        );
    }

    #[test]
    fn test_frustrated_user_with_legitimate_repair() {
        let start = Instant::now();
        let analyzer = TextBasedSignalAnalyzer::new();

        use hermesllm::apis::openai::{FunctionCall, ToolCall};

        // Helper to create a message with tool calls
        let create_assistant_with_tools =
            |content: &str, tool_id: &str, tool_name: &str, args: &str| -> Message {
                Message {
                    role: Role::Assistant,
                    content: Some(MessageContent::Text(content.to_string())),
                    name: None,
                    tool_calls: Some(vec![ToolCall {
                        id: tool_id.to_string(),
                        call_type: "function".to_string(),
                        function: FunctionCall {
                            name: tool_name.to_string(),
                            arguments: args.to_string(),
                        },
                    }]),
                    tool_call_id: None,
                }
            };

        // Helper to create a tool response message
        let create_tool_message = |tool_call_id: &str, content: &str| -> Message {
            Message {
                role: Role::Tool,
                content: Some(MessageContent::Text(content.to_string())),
                name: None,
                tool_calls: None,
                tool_call_id: Some(tool_call_id.to_string()),
            }
        };

        // Scenario: User DOES mention New York in first message, making "I already told you" legitimate
        let messages = vec![
            create_message(
                Role::User,
                "I need to book a flight from New York to Paris for December 20th",
            ),
            create_assistant_with_tools(
                "I'll help you search for flights to Paris.",
                "call_123",
                "search_flights",
                r#"{"origin": "NYC", "destination": "Paris", "date": "2025-12-20"}"#,
            ),
            create_tool_message("call_123", r#"{"flights": []}"#),
            create_message(
                Role::Assistant,
                "I couldn't find any flights. Could you provide your departure city?",
            ),
            create_message(Role::User, "I already told you, from New York!"),
            create_assistant_with_tools(
                "Let me try again.",
                "call_456",
                "search_flights",
                r#"{"origin": "New York", "destination": "Paris", "date": "2025-12-20"}"#,
            ),
            create_tool_message("call_456", r#"{"flights": []}"#),
            create_message(
                Role::Assistant,
                "I'm still not finding results. Let me check the system.",
            ),
            create_message(
                Role::User,
                "THIS IS RIDICULOUS!!! The tool doesn't work at all. Why do you keep calling it?",
            ),
            create_message(
                Role::Assistant,
                "I sincerely apologize for the frustration with the search tool.",
            ),
            create_message(
                Role::User,
                "Forget it. I need to speak to a human agent. This is a waste of time.",
            ),
        ];

        let report = analyzer.analyze(&messages);

        // Tool messages should be filtered out, so we should only analyze text messages
        // That's 4 user messages + 5 assistant text messages = 9 turns
        assert_eq!(
            report.turn_count.total_turns, 9,
            "Should count 9 text messages (tool messages filtered out)"
        );
        assert!(
            report.turn_count.is_concerning,
            "Should flag concerning turn count"
        );

        // Should detect frustration (all caps, complaints)
        assert!(
            report.frustration.has_frustration,
            "Should detect frustration"
        );
        assert!(
            report.frustration.frustration_count >= 2,
            "Should detect multiple frustration indicators"
        );
        assert!(
            report.frustration.severity >= 2,
            "Should have moderate or higher frustration severity"
        );

        // Should detect escalation request
        assert!(
            report.escalation.escalation_requested,
            "Should detect escalation to human agent"
        );
        assert!(
            report.escalation.escalation_count >= 1,
            "Should detect at least one escalation"
        );

        // Overall quality should be Poor or Severe
        assert!(
            matches!(
                report.overall_quality,
                InteractionQuality::Poor | InteractionQuality::Severe
            ),
            "Quality should be Poor or Severe, got {:?}",
            report.overall_quality
        );

        println!(
            "test_frustrated_user_with_legitimate_repair took: {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn test_frustrated_user_false_claim() {
        let start = Instant::now();
        let analyzer = TextBasedSignalAnalyzer::new();

        use hermesllm::apis::openai::{FunctionCall, ToolCall};

        // Helper to create a message with tool calls
        let create_assistant_with_tools =
            |content: &str, tool_id: &str, tool_name: &str, args: &str| -> Message {
                Message {
                    role: Role::Assistant,
                    content: Some(MessageContent::Text(content.to_string())),
                    name: None,
                    tool_calls: Some(vec![ToolCall {
                        id: tool_id.to_string(),
                        call_type: "function".to_string(),
                        function: FunctionCall {
                            name: tool_name.to_string(),
                            arguments: args.to_string(),
                        },
                    }]),
                    tool_call_id: None,
                }
            };

        // Helper to create a tool response message
        let create_tool_message = |tool_call_id: &str, content: &str| -> Message {
            Message {
                role: Role::Tool,
                content: Some(MessageContent::Text(content.to_string())),
                name: None,
                tool_calls: None,
                tool_call_id: Some(tool_call_id.to_string()),
            }
        };

        // Scenario: User NEVER mentions New York in first message but claims "I already told you"
        // This represents realistic frustrated user behavior - exaggeration/misremembering
        let messages = vec![
            create_message(
                Role::User,
                "I need to book a flight to Paris for December 20th",
            ),
            create_assistant_with_tools(
                "I'll help you search for flights to Paris.",
                "call_123",
                "search_flights",
                r#"{"destination": "Paris", "date": "2025-12-20"}"#,
            ),
            create_tool_message("call_123", r#"{"error": "origin required"}"#),
            create_message(
                Role::Assistant,
                "I couldn't find any flights. Could you provide your departure city?",
            ),
            create_message(Role::User, "I already told you, from New York!"), // False claim - never mentioned it
            create_assistant_with_tools(
                "Let me try again.",
                "call_456",
                "search_flights",
                r#"{"origin": "New York", "destination": "Paris", "date": "2025-12-20"}"#,
            ),
            create_tool_message("call_456", r#"{"flights": []}"#),
            create_message(
                Role::Assistant,
                "I'm still not finding results. Let me check the system.",
            ),
            create_message(
                Role::User,
                "THIS IS RIDICULOUS!!! The tool doesn't work at all. Why do you keep calling it?",
            ),
            create_message(
                Role::Assistant,
                "I sincerely apologize for the frustration with the search tool.",
            ),
            create_message(
                Role::User,
                "Forget it. I need to speak to a human agent. This is a waste of time.",
            ),
        ];

        let report = analyzer.analyze(&messages);

        // Tool messages should be filtered out, so we should only analyze text messages
        // That's 4 user messages + 5 assistant text messages = 9 turns
        assert_eq!(
            report.turn_count.total_turns, 9,
            "Should count 9 text messages (tool messages filtered out)"
        );
        assert!(
            report.turn_count.is_concerning,
            "Should flag concerning turn count"
        );

        // Should detect frustration (all caps, complaints, false claims)
        assert!(
            report.frustration.has_frustration,
            "Should detect frustration"
        );
        assert!(
            report.frustration.frustration_count >= 2,
            "Should detect multiple frustration indicators"
        );
        assert!(
            report.frustration.severity >= 2,
            "Should have moderate or higher frustration severity"
        );

        // Should detect escalation request
        assert!(
            report.escalation.escalation_requested,
            "Should detect escalation to human agent"
        );
        assert!(
            report.escalation.escalation_count >= 1,
            "Should detect at least one escalation"
        );

        // Note: May detect false positive "positive feedback" due to fuzzy matching
        // e.g., "I already told YOU" matches "you rock", "THIS is RIDICULOUS" matches "this helps"
        // However, the overall quality should still be Poor/Severe due to frustration+escalation

        // Overall quality should be Poor or Severe (frustration + escalation indicates poor interaction)
        assert!(
            matches!(
                report.overall_quality,
                InteractionQuality::Poor | InteractionQuality::Severe
            ),
            "Quality should be Poor or Severe for frustrated user with false claims, got {:?}",
            report.overall_quality
        );

        println!(
            "test_frustrated_user_false_claim took: {:?}",
            start.elapsed()
        );
    }

    // false negative tests
    #[test]
    fn test_dissatisfaction_polite_not_working_for_me() {
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![
            create_message(Role::User, "Thanks, but this still isn't working for me."), // Polite dissatisfaction, e.g., I appreciate it, but this isn't what I was looking for.
            create_message(Role::Assistant, "Sorry—what error do you see?"),
        ];
        let normalized = preprocess_messages(&messages);
        let signal = analyzer.analyze_frustration(&normalized);
        assert!(
            signal.has_frustration,
            "Polite dissatisfaction should be detected"
        );
    }

    #[test]
    fn test_dissatisfaction_giving_up_without_escalation() {
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![create_message(
            Role::User,
            "Never mind, I'll figure it out myself.",
        )];
        let normalized = preprocess_messages(&messages);
        let signal = analyzer.analyze_escalation(&normalized);
        assert!(
            signal.escalation_requested,
            "Giving up should count as escalation/quit intent"
        );
    }

    #[test]
    fn test_dissatisfaction_same_problem_again() {
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![create_message(
            Role::User,
            "I'm running into the same issue again.",
        )];
        let normalized = preprocess_messages(&messages);
        let signal = analyzer.analyze_frustration(&normalized);
        assert!(
            signal.has_frustration,
            "'same issue again' should be detected"
        );
    }

    #[test]
    fn test_unsatisfied_incomplete() {
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![create_message(Role::User, "This feels incomplete.")];
        let normalized = preprocess_messages(&messages);
        let signal = analyzer.analyze_frustration(&normalized);
        assert!(
            signal.has_frustration,
            "Should detect 'incomplete' dissatisfaction"
        );
    }

    #[test]
    fn test_low_mood_overwhelming() {
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![create_message(
            Role::User,
            "This is overwhelming and I'm not sure what to do.",
        )];
        let normalized = preprocess_messages(&messages);
        let signal = analyzer.analyze_frustration(&normalized);
        assert!(signal.has_frustration, "Should detect overwhelmed language");
    }

    #[test]
    fn test_low_mood_exhausted_trying() {
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![create_message(
            Role::User,
            "I'm exhausted trying to get this working.",
        )];
        let normalized = preprocess_messages(&messages);
        let signal = analyzer.analyze_frustration(&normalized);
        assert!(
            signal.has_frustration,
            "Should detect exhaustion/struggle language"
        );
    }

    #[test]
    fn test_common_polite_unresolved_dissatisfaction() {
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![
            create_message(Role::User, "I'm trying to set up SSH keys for GitHub."),
            create_message(
                Role::Assistant,
                "Sure. First generate a key using ssh-keygen.",
            ),
            create_message(Role::User, "I did that already."),
            create_message(
                Role::Assistant,
                "Then add the key to your GitHub account settings.",
            ),
            create_message(Role::User, "I've done that too."),
            create_message(
                Role::Assistant,
                "After that, make sure your SSH agent is running.",
            ),
            create_message(
                Role::User,
                "Okay, but this still doesn't seem to fix the issue.",
            ),
            create_message(Role::Assistant, "What error message are you seeing?"),
            create_message(Role::User, "It's just not connecting the way I expected."),
        ];

        let report = analyzer.analyze(&messages);

        // This is a common false negative if you only look for caps/profanity.
        // Desired: detect dissatisfaction/frustration (or at least not rate as Excellent).
        assert!(
            report.frustration.has_frustration || report.follow_up.repair_count >= 1,
            "Should detect polite unresolved dissatisfaction via frustration or follow-up indicators"
        );

        assert!(
            !matches!(report.overall_quality, InteractionQuality::Excellent),
            "Should not classify unresolved dissatisfaction as Excellent"
        );
    }

    #[test]
    fn test_common_resigned_giving_up_quietly() {
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![
            create_message(
                Role::User,
                "Can you explain how to deploy this with Docker?",
            ),
            create_message(
                Role::Assistant,
                "You need to write a Dockerfile and build an image.",
            ),
            create_message(Role::User, "I tried that."),
            create_message(Role::Assistant, "Then you can run docker-compose up."),
            create_message(Role::User, "I did, but it didn’t really help."),
            create_message(Role::Assistant, "What error are you getting?"),
            create_message(
                Role::User,
                "Honestly, never mind. I’ll just try something else.",
            ),
        ];

        let report = analyzer.analyze(&messages);

        // Many systems miss "never mind / I'll try something else" if they only look for "human agent".
        assert!(
            report.escalation.escalation_requested || report.frustration.has_frustration,
            "Resigned quitting language should trigger escalation or frustration"
        );

        assert!(
            matches!(
                report.overall_quality,
                InteractionQuality::Poor | InteractionQuality::Severe
            ) || report.escalation.escalation_requested
                || report.frustration.has_frustration,
            "Giving up should not be classified as a high-quality interaction"
        );
    }

    #[test]
    fn test_common_discouraged_overwhelmed_low_mood() {
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![
            create_message(Role::User, "I'm trying to understand backpropagation."),
            create_message(
                Role::Assistant,
                "It's a way to compute gradients efficiently.",
            ),
            create_message(Role::User, "I’ve read that explanation already."),
            create_message(Role::Assistant, "Would you like a mathematical derivation?"),
            create_message(Role::User, "Maybe, but I’m still having trouble following."),
            create_message(Role::Assistant, "I can walk through a simple example."),
            create_message(
                Role::User,
                "That might help, but honestly this is pretty overwhelming.",
            ),
            create_message(Role::Assistant, "Let’s slow it down step by step."),
            create_message(
                Role::User,
                "Yeah… I’m just feeling kind of discouraged right now.",
            ),
        ];

        let report = analyzer.analyze(&messages);

        // This is negative affect without caps/profanity. Should still count as frustration/negative signal.
        assert!(
            report.frustration.has_frustration,
            "Overwhelmed/discouraged language should be detected as negative sentiment/frustration"
        );

        assert!(
            !matches!(report.overall_quality, InteractionQuality::Excellent),
            "Low-mood discouragement should not be classified as Excellent"
        );
    }

    #[test]
    fn test_common_misalignment_not_what_i_asked() {
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![
            create_message(Role::User, "How do I optimize this SQL query?"),
            create_message(
                Role::Assistant,
                "You can add indexes to improve performance.",
            ),
            create_message(Role::User, "I already have indexes."),
            create_message(Role::Assistant, "Then you could consider query caching."),
            create_message(Role::User, "That’s not really what I was asking about."),
            create_message(
                Role::Assistant,
                "What specifically are you trying to optimize?",
            ),
            create_message(
                Role::User,
                "The execution plan — this answer doesn’t address that.",
            ),
        ];

        let report = analyzer.analyze(&messages);

        // Misalignment often shows as follow-up repair or frustration.
        assert!(
            report.follow_up.repair_count >= 1 || report.frustration.has_frustration,
            "Misalignment ('not what I asked') should trigger repair or frustration signals"
        );

        assert!(
            !matches!(report.overall_quality, InteractionQuality::Excellent),
            "Misalignment should not be rated as Excellent"
        );
    }

    #[test]
    fn test_common_false_negative_polite_disappointment_complexity() {
        let analyzer = TextBasedSignalAnalyzer::new();
        let messages = vec![
            create_message(Role::User, "Can you help me write a regex for this?"),
            create_message(Role::Assistant, "Sure, try this pattern: ^[a-z]+$"),
            create_message(Role::User, "I tested it."),
            create_message(Role::Assistant, "Did it work?"),
            create_message(Role::User, "Not quite — it matches more than it should."),
            create_message(Role::Assistant, "You can refine it with a lookahead."),
            create_message(
                Role::User,
                "I see… this is more complicated than I expected.",
            ),
        ];

        let report = analyzer.analyze(&messages);

        // Polite disappointment often becomes a false negative.
        assert!(
            report.frustration.has_frustration || report.follow_up.repair_count >= 1,
            "Polite dissatisfaction ('not quite', 'more complicated than expected') should trigger a negative signal"
        );

        assert!(
            !matches!(report.overall_quality, InteractionQuality::Excellent),
            "Polite disappointment should not be classified as Excellent"
        );
    }

    #[test]
    fn test_catastrophic_failure_looping_assistant() {
        let analyzer = TextBasedSignalAnalyzer::new();

        // Catastrophic failure: assistant stuck in loop, user increasingly frustrated
        let messages = vec![
            // Turn 1: Initial request
            create_message(Role::User, "I need to cancel my subscription"),
            create_message(Role::Assistant, "I can help you with account management. What would you like to do today?"),

            // Turn 3: First repair - repetition clarification
            create_message(Role::User, "I just said I want to cancel my subscription"),
            // Turn 4: Exact repetition from assistant
            create_message(Role::Assistant, "I can help you with account management. What would you like to do today?"),

            // Turn 5: Repair (negation) + frustration (ALL CAPS)
            create_message(Role::User, "No, I meant CANCEL. My. Subscription."),
            // Turn 6: Near-duplicate repetition
            create_message(Role::Assistant, "I understand you need help with your account. Let me assist you with that."),

            // Turn 7: Severe frustration (complaint + excessive punctuation + ALL CAPS)
            create_message(Role::User, "This is not helpful!! I want to CANCEL my subscription!!!"),
            // Turn 8: Exact repetition again (looping)
            create_message(Role::Assistant, "I can help you with account management. What would you like to do today?"),

            // Turn 9: Repair (explicit rephrase) + frustration (complaint)
            create_message(Role::User, "This doesnt work. Let me rephrase: I need to terminate my account and stop being charged"),
            // Turn 10: Near-duplicate repetition (looping continues)
            create_message(Role::Assistant, "I understand you need help with your account. Let me assist you with that."),

            // Turn 11: Escalation (multiple types) + frustration
            create_message(Role::User, "I give up. Can I speak to a real person? This is a waste of time"),
        ];

        let report = analyzer.analyze(&messages);

        // Validate turn count
        assert_eq!(
            report.turn_count.total_turns, 11,
            "Should have 11 total turns"
        );
        assert_eq!(report.turn_count.user_turns, 6, "Should have 6 user turns");
        assert_eq!(
            report.turn_count.assistant_turns, 5,
            "Should have 5 assistant turns"
        );
        assert!(
            report.turn_count.is_concerning,
            "11 turns should be concerning (>7)"
        );
        assert!(
            !report.turn_count.is_excessive,
            "11 turns should not be excessive (<=12)"
        );
        assert!(
            report.turn_count.efficiency_score < 0.5,
            "Efficiency should be low"
        );

        // Validate repair detection (USER signals - query reformulation)
        // Detected repairs:
        // 1. "I just said I want to cancel..." - pattern: "I just said"
        // 2. "No, I meant CANCEL..." - pattern: "No, I meant"
        // 3. "Let me rephrase: I need to terminate..." - pattern: "let me rephrase"
        // Note: "This is not helpful!!" is frustration (not repair)
        // Note: "I give up..." is escalation (not repair)
        assert_eq!(
            report.follow_up.repair_count, 3,
            "Should detect exactly 3 repair attempts from user messages"
        );
        assert_eq!(
            report.follow_up.repair_ratio, 0.5,
            "Repair ratio should be 0.5 (3 repairs / 6 user messages)"
        );
        assert!(
            report.follow_up.is_concerning,
            "50% repair ratio should be highly concerning (threshold is 30%)"
        );

        // Validate frustration detection
        assert!(
            report.frustration.has_frustration,
            "Should detect frustration"
        );
        assert!(
            report.frustration.frustration_count >= 4,
            "Should detect multiple frustration indicators: found {}",
            report.frustration.frustration_count
        );
        assert!(
            report.frustration.severity >= 2,
            "Should be at least moderate frustration"
        );

        // Validate repetition/looping detection (ASSISTANT signals - not following instructions)
        // The assistant repeats the same unhelpful responses multiple times:
        // 1. "I can help you with account management..." appears 3 times (exact repetition)
        // 2. "I understand you need help with your account..." appears 2 times (near-duplicate)
        assert!(
            report.repetition.repetition_count >= 4,
            "Should detect at least 4 assistant repetitions (exact + near-duplicates)"
        );
        assert!(
            report.repetition.has_looping,
            "Should detect looping (>2 repetitions indicates stuck agent)"
        );
        assert!(
            report.repetition.severity >= 2,
            "Should be moderate to severe looping (assistant not adapting)"
        );

        // Validate escalation detection
        assert!(
            report.escalation.escalation_requested,
            "Should detect escalation request"
        );
        assert!(
            report.escalation.escalation_count >= 2,
            "Should detect multiple escalation indicators: 'give up' + 'speak to a real person'"
        );

        // Validate overall quality
        assert_eq!(report.overall_quality, InteractionQuality::Severe, "Should be classified as Severe due to escalation + excessive frustration + looping + high repair ratio");
    }
}
