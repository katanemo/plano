//! Misalignment signals: corrections, rephrases, clarifications.
//!
//! Direct port of `signals/interaction/misalignment.py`.

use std::sync::OnceLock;

use serde_json::json;

use super::constants::{stopwords, CONFIRMATION_PREFIXES};
use crate::signals::schemas::{SignalGroup, SignalInstance, SignalType};
use crate::signals::text_processing::{normalize_patterns, NormalizedMessage, NormalizedPattern};

const CORRECTION_PATTERN_TEXTS: &[&str] = &[
    "no, i meant",
    "no i meant",
    "no, i said",
    "no i said",
    "no, i asked",
    "no i asked",
    "nah, i meant",
    "nope, i meant",
    "not what i said",
    "not what i asked",
    "that's not what i said",
    "that's not what i asked",
    "that's not what i meant",
    "thats not what i said",
    "thats not what i asked",
    "thats not what i meant",
    "that's not what you",
    "no that's not what i",
    "no, that's not what i",
    "you're not quite right",
    "youre not quite right",
    "you're not exactly right",
    "youre not exactly right",
    "you're wrong about",
    "youre wrong about",
    "i just said",
    "i already said",
    "i already told you",
];

const REPHRASE_PATTERN_TEXTS: &[&str] = &[
    "let me rephrase",
    "let me explain again",
    "what i'm trying to say",
    "what i'm saying is",
    "in other words",
];

const CLARIFICATION_PATTERN_TEXTS: &[&str] = &[
    "i don't understand",
    "don't understand",
    "not understanding",
    "can't understand",
    "don't get it",
    "don't follow",
    "i'm confused",
    "so confused",
    "makes no sense",
    "doesn't make sense",
    "not making sense",
    "what do you mean",
    "what does that mean",
    "what are you saying",
    "i'm lost",
    "totally lost",
    "lost me",
    "no clue what you",
    "no idea what you",
    "no clue what that",
    "no idea what that",
    "come again",
    "say that again",
    "repeat that",
    "trouble following",
    "hard to follow",
    "can't follow",
];

fn correction_patterns() -> &'static Vec<NormalizedPattern> {
    static PATS: OnceLock<Vec<NormalizedPattern>> = OnceLock::new();
    PATS.get_or_init(|| normalize_patterns(CORRECTION_PATTERN_TEXTS))
}

fn rephrase_patterns() -> &'static Vec<NormalizedPattern> {
    static PATS: OnceLock<Vec<NormalizedPattern>> = OnceLock::new();
    PATS.get_or_init(|| normalize_patterns(REPHRASE_PATTERN_TEXTS))
}

fn clarification_patterns() -> &'static Vec<NormalizedPattern> {
    static PATS: OnceLock<Vec<NormalizedPattern>> = OnceLock::new();
    PATS.get_or_init(|| normalize_patterns(CLARIFICATION_PATTERN_TEXTS))
}

fn is_confirmation_message(text: &str) -> bool {
    let lowered = text.to_lowercase();
    let trimmed = lowered.trim();
    CONFIRMATION_PREFIXES.iter().any(|p| trimmed.starts_with(p))
}

/// Detect whether two user messages appear to be rephrases of each other.
pub fn is_similar_rephrase(
    norm_msg1: &NormalizedMessage,
    norm_msg2: &NormalizedMessage,
    overlap_threshold: f32,
    min_meaningful_tokens: usize,
    max_new_content_ratio: f32,
) -> bool {
    if norm_msg1.tokens.len() < 3 || norm_msg2.tokens.len() < 3 {
        return false;
    }
    if is_confirmation_message(&norm_msg1.raw) {
        return false;
    }

    let stops = stopwords();
    let tokens1: std::collections::HashSet<&str> = norm_msg1
        .tokens
        .iter()
        .filter(|t| !stops.contains(t.as_str()))
        .map(|s| s.as_str())
        .collect();
    let tokens2: std::collections::HashSet<&str> = norm_msg2
        .tokens
        .iter()
        .filter(|t| !stops.contains(t.as_str()))
        .map(|s| s.as_str())
        .collect();

    if tokens1.len() < min_meaningful_tokens || tokens2.len() < min_meaningful_tokens {
        return false;
    }

    let new_tokens: std::collections::HashSet<&&str> = tokens1.difference(&tokens2).collect();
    let new_content_ratio = if tokens1.is_empty() {
        0.0
    } else {
        new_tokens.len() as f32 / tokens1.len() as f32
    };
    if new_content_ratio > max_new_content_ratio {
        return false;
    }

    let intersection = tokens1.intersection(&tokens2).count();
    let min_size = tokens1.len().min(tokens2.len());
    if min_size == 0 {
        return false;
    }
    let overlap_ratio = intersection as f32 / min_size as f32;
    overlap_ratio >= overlap_threshold
}

/// Analyze user messages for misalignment signals.
pub fn analyze_misalignment(
    normalized_messages: &[(usize, &str, NormalizedMessage)],
    char_ngram_threshold: f32,
    token_cosine_threshold: f32,
) -> SignalGroup {
    let mut group = SignalGroup::new("misalignment");

    let mut prev_user_idx: Option<usize> = None;
    let mut prev_user_msg: Option<&NormalizedMessage> = None;

    for (idx, role, norm_msg) in normalized_messages {
        if *role != "human" {
            continue;
        }

        let mut found_in_turn = false;

        for pattern in correction_patterns() {
            if norm_msg.matches_normalized_pattern(
                pattern,
                char_ngram_threshold,
                token_cosine_threshold,
            ) {
                group.add_signal(
                    SignalInstance::new(
                        SignalType::MisalignmentCorrection,
                        *idx,
                        pattern.raw.clone(),
                    )
                    .with_metadata(json!({"pattern_type": "correction"})),
                );
                found_in_turn = true;
                break;
            }
        }

        if found_in_turn {
            prev_user_idx = Some(*idx);
            prev_user_msg = Some(norm_msg);
            continue;
        }

        for pattern in rephrase_patterns() {
            if norm_msg.matches_normalized_pattern(
                pattern,
                char_ngram_threshold,
                token_cosine_threshold,
            ) {
                group.add_signal(
                    SignalInstance::new(
                        SignalType::MisalignmentRephrase,
                        *idx,
                        pattern.raw.clone(),
                    )
                    .with_metadata(json!({"pattern_type": "rephrase"})),
                );
                found_in_turn = true;
                break;
            }
        }

        if found_in_turn {
            prev_user_idx = Some(*idx);
            prev_user_msg = Some(norm_msg);
            continue;
        }

        for pattern in clarification_patterns() {
            if norm_msg.matches_normalized_pattern(
                pattern,
                char_ngram_threshold,
                token_cosine_threshold,
            ) {
                group.add_signal(
                    SignalInstance::new(
                        SignalType::MisalignmentClarification,
                        *idx,
                        pattern.raw.clone(),
                    )
                    .with_metadata(json!({"pattern_type": "clarification"})),
                );
                found_in_turn = true;
                break;
            }
        }

        if found_in_turn {
            prev_user_idx = Some(*idx);
            prev_user_msg = Some(norm_msg);
            continue;
        }

        // Semantic rephrase vs the previous user message (recent only).
        if let (Some(prev_idx), Some(prev_msg)) = (prev_user_idx, prev_user_msg) {
            let turns_between = idx.saturating_sub(prev_idx);
            if turns_between <= 3 && is_similar_rephrase(norm_msg, prev_msg, 0.75, 4, 0.5) {
                group.add_signal(
                    SignalInstance::new(
                        SignalType::MisalignmentRephrase,
                        *idx,
                        "[similar rephrase detected]",
                    )
                    .with_confidence(0.8)
                    .with_metadata(json!({
                        "pattern_type": "semantic_rephrase",
                        "compared_to": prev_idx,
                    })),
                );
            }
        }

        prev_user_idx = Some(*idx);
        prev_user_msg = Some(norm_msg);
    }

    group
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nm(s: &str) -> NormalizedMessage {
        NormalizedMessage::from_text(s, 2000)
    }

    fn make(items: &[(&'static str, &str)]) -> Vec<(usize, &'static str, NormalizedMessage)> {
        items
            .iter()
            .enumerate()
            .map(|(i, (role, text))| (i, *role, nm(text)))
            .collect()
    }

    #[test]
    fn detects_explicit_correction() {
        let msgs = make(&[
            ("human", "Show me my orders"),
            ("gpt", "Sure, here are your invoices"),
            ("human", "No, I meant my recent orders"),
        ]);
        let g = analyze_misalignment(&msgs, 0.65, 0.6);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::MisalignmentCorrection)));
    }

    #[test]
    fn detects_rephrase_marker() {
        let msgs = make(&[
            ("human", "Show me X"),
            ("gpt", "Sure"),
            ("human", "Let me rephrase: I want X grouped by date"),
        ]);
        let g = analyze_misalignment(&msgs, 0.65, 0.6);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::MisalignmentRephrase)));
    }

    #[test]
    fn detects_clarification_request() {
        let msgs = make(&[
            ("human", "Run the report"),
            ("gpt", "Foobar quux baz."),
            ("human", "I don't understand what you mean"),
        ]);
        let g = analyze_misalignment(&msgs, 0.65, 0.6);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::MisalignmentClarification)));
    }

    #[test]
    fn confirmation_is_not_a_rephrase() {
        let m1 = nm("Yes, that's correct, please proceed with the order");
        let m2 = nm("please proceed with the order for the same product");
        assert!(!is_similar_rephrase(&m1, &m2, 0.75, 4, 0.5));
    }
}
