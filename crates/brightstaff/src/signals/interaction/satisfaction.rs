//! Satisfaction signals: gratitude, confirmation, success.
//!
//! Direct port of `signals/interaction/satisfaction.py`.

use std::sync::OnceLock;

use serde_json::json;

use crate::signals::schemas::{SignalGroup, SignalInstance, SignalType};
use crate::signals::text_processing::{normalize_patterns, NormalizedMessage, NormalizedPattern};

const GRATITUDE_PATTERN_TEXTS: &[&str] = &[
    "that's helpful",
    "that helps",
    "this helps",
    "appreciate it",
    "appreciate that",
    "that's perfect",
    "exactly what i needed",
    "just what i needed",
    "you're the best",
    "you rock",
    "you're awesome",
    "you're amazing",
    "you're great",
];

const CONFIRMATION_PATTERN_TEXTS: &[&str] = &[
    "that works",
    "this works",
    "that's great",
    "that's amazing",
    "this is great",
    "that's awesome",
    "love it",
    "love this",
    "love that",
];

const SUCCESS_PATTERN_TEXTS: &[&str] = &[
    "it worked",
    "that worked",
    "this worked",
    "it's working",
    "that's working",
    "this is working",
];

fn gratitude_patterns() -> &'static Vec<NormalizedPattern> {
    static PATS: OnceLock<Vec<NormalizedPattern>> = OnceLock::new();
    PATS.get_or_init(|| normalize_patterns(GRATITUDE_PATTERN_TEXTS))
}

fn confirmation_patterns() -> &'static Vec<NormalizedPattern> {
    static PATS: OnceLock<Vec<NormalizedPattern>> = OnceLock::new();
    PATS.get_or_init(|| normalize_patterns(CONFIRMATION_PATTERN_TEXTS))
}

fn success_patterns() -> &'static Vec<NormalizedPattern> {
    static PATS: OnceLock<Vec<NormalizedPattern>> = OnceLock::new();
    PATS.get_or_init(|| normalize_patterns(SUCCESS_PATTERN_TEXTS))
}

pub fn analyze_satisfaction(
    normalized_messages: &[(usize, &str, NormalizedMessage)],
    char_ngram_threshold: f32,
    token_cosine_threshold: f32,
) -> SignalGroup {
    let mut group = SignalGroup::new("satisfaction");

    for (idx, role, norm_msg) in normalized_messages {
        if *role != "human" {
            continue;
        }

        let mut found = false;

        for pattern in gratitude_patterns() {
            if norm_msg.matches_normalized_pattern(
                pattern,
                char_ngram_threshold,
                token_cosine_threshold,
            ) {
                group.add_signal(
                    SignalInstance::new(
                        SignalType::SatisfactionGratitude,
                        *idx,
                        pattern.raw.clone(),
                    )
                    .with_metadata(json!({"pattern_type": "gratitude"})),
                );
                found = true;
                break;
            }
        }
        if found {
            continue;
        }

        for pattern in confirmation_patterns() {
            if norm_msg.matches_normalized_pattern(
                pattern,
                char_ngram_threshold,
                token_cosine_threshold,
            ) {
                group.add_signal(
                    SignalInstance::new(
                        SignalType::SatisfactionConfirmation,
                        *idx,
                        pattern.raw.clone(),
                    )
                    .with_metadata(json!({"pattern_type": "confirmation"})),
                );
                found = true;
                break;
            }
        }
        if found {
            continue;
        }

        for pattern in success_patterns() {
            if norm_msg.matches_normalized_pattern(
                pattern,
                char_ngram_threshold,
                token_cosine_threshold,
            ) {
                group.add_signal(
                    SignalInstance::new(SignalType::SatisfactionSuccess, *idx, pattern.raw.clone())
                        .with_metadata(json!({"pattern_type": "success"})),
                );
                break;
            }
        }
    }

    group
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nm(s: &str) -> NormalizedMessage {
        NormalizedMessage::from_text(s, 2000)
    }

    #[test]
    fn detects_gratitude() {
        let msgs = vec![(0usize, "human", nm("That's perfect, appreciate it!"))];
        let g = analyze_satisfaction(&msgs, 0.65, 0.6);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::SatisfactionGratitude)));
    }

    #[test]
    fn detects_confirmation() {
        let msgs = vec![(0usize, "human", nm("That works for me, thanks"))];
        let g = analyze_satisfaction(&msgs, 0.65, 0.6);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::SatisfactionConfirmation)));
    }

    #[test]
    fn detects_success() {
        let msgs = vec![(0usize, "human", nm("Great, it worked!"))];
        let g = analyze_satisfaction(&msgs, 0.65, 0.6);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::SatisfactionSuccess)));
    }
}
