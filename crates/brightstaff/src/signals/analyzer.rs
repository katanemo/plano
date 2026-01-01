//! Agentic Signals - Behavioral quality indicators for agent interactions
//!
//! This module implements various signals that serve as early warning indicators
//! of brilliant successes or failures in agentic interactions. These signals are
//! derived from conversation patterns and can be computed algorithmically from
//! message arrays.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use strsim::jaro_winkler;

use hermesllm::apis::openai::{Message, Role};

// ============================================================================
// Constants
// ============================================================================

/// Flag emoji for marking spans/operations worth investigating
pub const FLAG_MARKER: &str = "\u{1F6A9}";

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
}

impl NormalizedMessage {
    fn from_text(text: &str) -> Self {
        let raw = text.to_string();

        // Normalize unicode punctuation to ASCII equivalents
        let normalized_unicode = text
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

        Self {
            raw,
            tokens,
            token_set,
            bigram_set,
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

    /// Check if phrase exists using fuzzy matching (for typo tolerance)
    fn fuzzy_contains_phrase(&self, phrase: &str, threshold: f64) -> bool {
        let phrase_tokens: Vec<&str> = phrase.split_whitespace().collect();
        if phrase_tokens.is_empty() {
            return false;
        }

        // For single tokens, use higher threshold
        let adjusted_threshold = if phrase_tokens.len() == 1 && phrase.len() < 5 {
            0.95
        } else {
            threshold
        };

        // Check each window of tokens
        if self.tokens.len() >= phrase_tokens.len() {
            for window in self.tokens.windows(phrase_tokens.len()) {
                let window_text = window.join(" ");
                let similarity = jaro_winkler(&window_text, phrase);
                if similarity >= adjusted_threshold {
                    return true;
                }
            }
        }

        false
    }
}

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

/// Main analyzer that computes all signals from a message array
pub struct SignalAnalyzer {
    /// Baseline expected turns for normal interactions
    baseline_turns: usize,
    /// Threshold for fuzzy pattern matching (0.0-1.0)
    fuzzy_threshold: f64,
}

impl SignalAnalyzer {
    /// Extract text content from MessageContent, skipping non-text content
    fn extract_text(content: &hermesllm::apis::openai::MessageContent) -> Option<String> {
        match content {
            hermesllm::apis::openai::MessageContent::Text(text) => Some(text.clone()),
            // Tool calls and other structured content are skipped
            _ => None,
        }
    }

    /// Check if a pattern is long enough to warrant fuzzy matching
    /// Short patterns (< 3 words) should use exact matching only to avoid false positives
    fn should_use_fuzzy_matching(pattern: &str) -> bool {
        pattern.split_whitespace().count() >= 3
    }

    /// Create a new signal analyzer with default settings
    pub fn new() -> Self {
        Self {
            baseline_turns: 5,
            fuzzy_threshold: 0.88,
        }
    }

    /// Create a new signal analyzer with custom baseline
    pub fn with_baseline(baseline_turns: usize) -> Self {
        Self {
            baseline_turns,
            fuzzy_threshold: 0.88,
        }
    }

    /// Create a new signal analyzer with custom settings
    pub fn with_settings(baseline_turns: usize, fuzzy_threshold: f64) -> Self {
        Self {
            baseline_turns,
            fuzzy_threshold,
        }
    }

    /// Analyze a conversation and generate a complete signal report
    pub fn analyze(&self, messages: &[Message]) -> SignalReport {
        // Preprocess all messages once, filtering out non-text content (tool calls, etc.)
        let normalized_messages: Vec<(usize, Role, NormalizedMessage)> = messages
            .iter()
            .enumerate()
            .filter_map(|(i, msg)| {
                Self::extract_text(&msg.content)
                    .map(|text| (i, msg.role.clone(), NormalizedMessage::from_text(&text)))
            })
            .collect();

        let turn_count = self.analyze_turn_count(messages);
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
        let repair_patterns = [
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
        ];

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

            for pattern in &repair_patterns {
                if norm_msg.contains_phrase(pattern) {
                    repair_count += 1;
                    repair_phrases.push(format!("Turn {}: '{}'", i + 1, pattern));
                    found_in_turn = true;
                    break;
                } else if Self::should_use_fuzzy_matching(pattern)
                    && norm_msg.fuzzy_contains_phrase(pattern, self.fuzzy_threshold)
                {
                    repair_count += 1;
                    repair_phrases.push(format!("Turn {}: '{}' (fuzzy)", i + 1, pattern));
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

        // Complaint phrases - removed ultra-generic single words that cause false positives
        let complaint_patterns = [
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
        ];

        // Confusion phrases - removed ultra-generic single words
        let confusion_patterns = [
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
        ];

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

            // Check for complaint patterns (phrase-based, not substring)
            for pattern in &complaint_patterns {
                if norm_msg.contains_phrase(pattern) {
                    indicators.push(FrustrationIndicator {
                        indicator_type: FrustrationType::DirectComplaint,
                        message_index: *i,
                        snippet: pattern.to_string(),
                    });
                    break;
                } else if Self::should_use_fuzzy_matching(pattern)
                    && norm_msg.fuzzy_contains_phrase(pattern, self.fuzzy_threshold)
                {
                    indicators.push(FrustrationIndicator {
                        indicator_type: FrustrationType::DirectComplaint,
                        message_index: *i,
                        snippet: format!("{} (fuzzy)", pattern),
                    });
                    break;
                }
            }

            // Check for confusion patterns (phrase-based)
            for pattern in &confusion_patterns {
                if norm_msg.contains_phrase(pattern) {
                    indicators.push(FrustrationIndicator {
                        indicator_type: FrustrationType::Confusion,
                        message_index: *i,
                        snippet: pattern.to_string(),
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

        // Check for exact or near-duplicate responses using bigram similarity
        for i in 0..assistant_messages.len() {
            for j in (i + 1)..assistant_messages.len() {
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

        let gratitude_patterns = [
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
        ];

        let satisfaction_patterns = [
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
        ];

        let success_patterns = [
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
        ];

        for (i, role, norm_msg) in normalized_messages {
            if *role != Role::User {
                continue;
            }

            // Use per-turn boolean to prevent double-counting
            let mut found_in_turn = false;

            // Check gratitude
            for pattern in &gratitude_patterns {
                if norm_msg.contains_phrase(pattern) {
                    indicators.push(PositiveIndicator {
                        indicator_type: PositiveType::Gratitude,
                        message_index: *i,
                        snippet: pattern.to_string(),
                    });
                    found_in_turn = true;
                    break;
                } else if Self::should_use_fuzzy_matching(pattern)
                    && norm_msg.fuzzy_contains_phrase(pattern, self.fuzzy_threshold)
                {
                    indicators.push(PositiveIndicator {
                        indicator_type: PositiveType::Gratitude,
                        message_index: *i,
                        snippet: format!("{} (fuzzy)", pattern),
                    });
                    found_in_turn = true;
                    break;
                }
            }

            if found_in_turn {
                continue;
            }

            // Check satisfaction
            for pattern in &satisfaction_patterns {
                if norm_msg.contains_phrase(pattern) {
                    indicators.push(PositiveIndicator {
                        indicator_type: PositiveType::Satisfaction,
                        message_index: *i,
                        snippet: pattern.to_string(),
                    });
                    found_in_turn = true;
                    break;
                }
            }

            if found_in_turn {
                continue;
            }

            // Check success confirmation
            for pattern in &success_patterns {
                if norm_msg.contains_phrase(pattern) {
                    indicators.push(PositiveIndicator {
                        indicator_type: PositiveType::Success,
                        message_index: *i,
                        snippet: pattern.to_string(),
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

        let human_agent_patterns = [
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
        ];

        let support_patterns = [
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
        ];

        let quit_patterns = [
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
            "find another",
            "use something else",
        ];

        for (i, role, norm_msg) in normalized_messages {
            if *role != Role::User {
                continue;
            }

            // Check for human agent request
            for pattern in &human_agent_patterns {
                if norm_msg.contains_phrase(pattern) {
                    requests.push(EscalationRequest {
                        message_index: *i,
                        snippet: pattern.to_string(),
                        escalation_type: EscalationType::HumanAgent,
                    });
                    break;
                } else if Self::should_use_fuzzy_matching(pattern)
                    && norm_msg.fuzzy_contains_phrase(pattern, self.fuzzy_threshold)
                {
                    requests.push(EscalationRequest {
                        message_index: *i,
                        snippet: format!("{} (fuzzy)", pattern),
                        escalation_type: EscalationType::HumanAgent,
                    });
                    break;
                }
            }

            // Check for support request
            for pattern in &support_patterns {
                if norm_msg.contains_phrase(pattern) {
                    requests.push(EscalationRequest {
                        message_index: *i,
                        snippet: pattern.to_string(),
                        escalation_type: EscalationType::Support,
                    });
                    break;
                }
            }

            // Check for quit threats
            for pattern in &quit_patterns {
                if norm_msg.contains_phrase(pattern) {
                    requests.push(EscalationRequest {
                        message_index: *i,
                        snippet: pattern.to_string(),
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

impl Default for SignalAnalyzer {
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
    use std::time::Instant;

    fn create_message(role: Role, content: &str) -> Message {
        Message {
            role,
            content: MessageContent::Text(content.to_string()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn preprocess_messages(messages: &[Message]) -> Vec<(usize, Role, NormalizedMessage)> {
        messages
            .iter()
            .enumerate()
            .map(|(i, msg)| {
                let text = msg.content.to_string();
                (i, msg.role.clone(), NormalizedMessage::from_text(&text))
            })
            .collect()
    }

    #[test]
    fn test_turn_count_efficient() {
        let start = Instant::now();
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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

        // Debug output to see what was detected
        println!("Detected {} repetitions:", signal.repetition_count);
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
        let messages = vec![
            create_message(Role::User, "this dosnt work at all"),
            create_message(Role::Assistant, "I apologize"),
        ];

        let normalized_messages = preprocess_messages(&messages);
        let signal = analyzer.analyze_frustration(&normalized_messages);
        assert!(signal.has_frustration);
        assert!(signal.frustration_count >= 1);
        println!("test_fuzzy_matching_complaint took: {:?}", start.elapsed());
    }

    #[test]
    fn test_fuzzy_threshold_configuration() {
        let start = Instant::now();
        let analyzer = SignalAnalyzer::with_settings(5, 0.95);
        assert_eq!(analyzer.fuzzy_threshold, 0.95);

        // Very strict threshold should not match heavily garbled text
        let messages = vec![
            create_message(Role::User, "xyz abc"), // Completely unrelated to any gratitude pattern
        ];
        let normalized_messages = preprocess_messages(&messages);
        let signal = analyzer.analyze_positive_feedback(&normalized_messages);
        assert_eq!(signal.positive_count, 0);
        println!(
            "test_fuzzy_threshold_configuration took: {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn test_exact_match_priority() {
        let start = Instant::now();
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();

        use hermesllm::apis::openai::{FunctionCall, ToolCall};

        // Helper to create a message with tool calls
        let create_assistant_with_tools =
            |content: &str, tool_id: &str, tool_name: &str, args: &str| -> Message {
                Message {
                    role: Role::Assistant,
                    content: MessageContent::Text(content.to_string()),
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
                content: MessageContent::Text(content.to_string()),
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
        let analyzer = SignalAnalyzer::new();

        use hermesllm::apis::openai::{FunctionCall, ToolCall};

        // Helper to create a message with tool calls
        let create_assistant_with_tools =
            |content: &str, tool_id: &str, tool_name: &str, args: &str| -> Message {
                Message {
                    role: Role::Assistant,
                    content: MessageContent::Text(content.to_string()),
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
                content: MessageContent::Text(content.to_string()),
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
        println!("Full signal analysis completed in {:?}", start.elapsed());
    }

    // false negative tests
    #[test]
    fn test_dissatisfaction_polite_not_working_for_me() {
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
        let analyzer = SignalAnalyzer::new();
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
}
