//! Disengagement signals: escalation, quit, negative stance.
//!
//! Direct port of `signals/interaction/disengagement.py`.

use std::sync::OnceLock;

use regex::Regex;
use serde_json::json;

use super::constants::{starts_with_prefix, POSITIVE_PREFIXES};
use crate::signals::schemas::{SignalGroup, SignalInstance, SignalType};
use crate::signals::text_processing::{normalize_patterns, NormalizedMessage, NormalizedPattern};

const ESCALATION_PATTERN_TEXTS: &[&str] = &[
    // Human requests
    "speak to a human",
    "talk to a human",
    "connect me to a human",
    "connect me with a human",
    "transfer me to a human",
    "get me a human",
    "chat with a human",
    // Person requests
    "speak to a person",
    "talk to a person",
    "connect me to a person",
    "connect me with a person",
    "transfer me to a person",
    "get me a person",
    "chat with a person",
    // Real person requests
    "speak to a real person",
    "talk to a real person",
    "connect me to a real person",
    "connect me with a real person",
    "transfer me to a real person",
    "get me a real person",
    "chat with a real person",
    // Actual person requests
    "speak to an actual person",
    "talk to an actual person",
    "connect me to an actual person",
    "connect me with an actual person",
    "transfer me to an actual person",
    "get me an actual person",
    "chat with an actual person",
    // Supervisor requests
    "speak to a supervisor",
    "talk to a supervisor",
    "connect me to a supervisor",
    "connect me with a supervisor",
    "transfer me to a supervisor",
    "get me a supervisor",
    "chat with a supervisor",
    // Manager requests
    "speak to a manager",
    "talk to a manager",
    "connect me to a manager",
    "connect me with a manager",
    "transfer me to a manager",
    "get me a manager",
    "chat with a manager",
    // Customer service requests
    "speak to customer service",
    "talk to customer service",
    "connect me to customer service",
    "connect me with customer service",
    "transfer me to customer service",
    "get me customer service",
    "chat with customer service",
    // Customer support requests
    "speak to customer support",
    "talk to customer support",
    "connect me to customer support",
    "connect me with customer support",
    "transfer me to customer support",
    "get me customer support",
    "chat with customer support",
    // Support requests
    "speak to support",
    "talk to support",
    "connect me to support",
    "connect me with support",
    "transfer me to support",
    "get me support",
    "chat with support",
    // Tech support requests
    "speak to tech support",
    "talk to tech support",
    "connect me to tech support",
    "connect me with tech support",
    "transfer me to tech support",
    "get me tech support",
    "chat with tech support",
    // Help desk requests
    "speak to help desk",
    "talk to help desk",
    "connect me to help desk",
    "connect me with help desk",
    "transfer me to help desk",
    "get me help desk",
    "chat with help desk",
    // Explicit escalation
    "escalate this",
];

const QUIT_PATTERN_TEXTS: &[&str] = &[
    "i give up",
    "i'm giving up",
    "im giving up",
    "i'm going to quit",
    "i quit",
    "forget it",
    "forget this",
    "screw it",
    "screw this",
    "don't bother trying",
    "don't bother with this",
    "don't bother with it",
    "don't even bother",
    "why bother",
    "not worth it",
    "this is hopeless",
    "going elsewhere",
    "try somewhere else",
    "look elsewhere",
];

const NEGATIVE_STANCE_PATTERN_TEXTS: &[&str] = &[
    "this is useless",
    "not helpful",
    "doesn't help",
    "not helping",
    "you're not helping",
    "youre not helping",
    "this doesn't work",
    "this doesnt work",
    "this isn't working",
    "this isnt working",
    "still doesn't work",
    "still doesnt work",
    "still not working",
    "still isn't working",
    "still isnt working",
    "waste of time",
    "wasting my time",
    "this is ridiculous",
    "this is absurd",
    "this is insane",
    "this is stupid",
    "this is dumb",
    "this sucks",
    "this is frustrating",
    "not good enough",
    "why can't you",
    "why cant you",
    "same issue",
    "did that already",
    "done that already",
    "tried that already",
    "already tried that",
    "i've done that",
    "ive done that",
    "i've tried that",
    "ive tried that",
    "i'm disappointed",
    "im disappointed",
    "disappointed with you",
    "disappointed in you",
    "useless bot",
    "dumb bot",
    "stupid bot",
];

const AGENT_DIRECTED_PROFANITY_PATTERN_TEXTS: &[&str] = &[
    "this is bullshit",
    "what bullshit",
    "such bullshit",
    "total bullshit",
    "complete bullshit",
    "this is crap",
    "what crap",
    "this is shit",
    "what the hell is wrong with you",
    "what the fuck is wrong with you",
    "you're fucking useless",
    "youre fucking useless",
    "you are fucking useless",
    "fucking useless",
    "this bot is shit",
    "this bot is crap",
    "damn bot",
    "fucking bot",
    "stupid fucking",
    "are you fucking kidding",
    "wtf is wrong with you",
    "wtf is this",
    "ffs just",
    "for fucks sake",
    "for fuck's sake",
    "what the f**k",
    "what the f*ck",
    "what the f***",
    "that's bullsh*t",
    "thats bullsh*t",
    "that's bull***t",
    "thats bull***t",
    "that's bs",
    "thats bs",
    "this is bullsh*t",
    "this is bull***t",
    "this is bs",
];

fn escalation_patterns() -> &'static Vec<NormalizedPattern> {
    static PATS: OnceLock<Vec<NormalizedPattern>> = OnceLock::new();
    PATS.get_or_init(|| normalize_patterns(ESCALATION_PATTERN_TEXTS))
}

fn quit_patterns() -> &'static Vec<NormalizedPattern> {
    static PATS: OnceLock<Vec<NormalizedPattern>> = OnceLock::new();
    PATS.get_or_init(|| normalize_patterns(QUIT_PATTERN_TEXTS))
}

fn negative_stance_patterns() -> &'static Vec<NormalizedPattern> {
    static PATS: OnceLock<Vec<NormalizedPattern>> = OnceLock::new();
    PATS.get_or_init(|| normalize_patterns(NEGATIVE_STANCE_PATTERN_TEXTS))
}

fn profanity_patterns() -> &'static Vec<NormalizedPattern> {
    static PATS: OnceLock<Vec<NormalizedPattern>> = OnceLock::new();
    PATS.get_or_init(|| normalize_patterns(AGENT_DIRECTED_PROFANITY_PATTERN_TEXTS))
}

fn re_consecutive_q() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\?{2,}").unwrap())
}
fn re_consecutive_e() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"!{2,}").unwrap())
}
fn re_mixed_punct() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"[?!]{3,}").unwrap())
}

pub fn analyze_disengagement(
    normalized_messages: &[(usize, &str, NormalizedMessage)],
    char_ngram_threshold: f32,
    token_cosine_threshold: f32,
) -> SignalGroup {
    let mut group = SignalGroup::new("disengagement");

    for (idx, role, norm_msg) in normalized_messages {
        if *role != "human" {
            continue;
        }

        let text = &norm_msg.raw;

        // All-caps shouting check.
        let alpha_chars: String = text.chars().filter(|c| c.is_alphabetic()).collect();
        if alpha_chars.chars().count() >= 10 {
            let upper_count = alpha_chars.chars().filter(|c| c.is_uppercase()).count();
            let upper_ratio = upper_count as f32 / alpha_chars.chars().count() as f32;
            if upper_ratio >= 0.8 {
                let snippet: String = text.chars().take(50).collect();
                group.add_signal(
                    SignalInstance::new(SignalType::DisengagementNegativeStance, *idx, snippet)
                        .with_metadata(json!({
                            "indicator_type": "all_caps",
                            "upper_ratio": upper_ratio,
                        })),
                );
            }
        }

        // Excessive consecutive punctuation.
        let starts_with_positive = starts_with_prefix(text, POSITIVE_PREFIXES);
        let cq = re_consecutive_q().find_iter(text).count();
        let ce = re_consecutive_e().find_iter(text).count();
        let mixed = re_mixed_punct().find_iter(text).count();
        if !starts_with_positive && (cq >= 1 || ce >= 1 || mixed >= 1) {
            let snippet: String = text.chars().take(50).collect();
            group.add_signal(
                SignalInstance::new(SignalType::DisengagementNegativeStance, *idx, snippet)
                    .with_metadata(json!({
                        "indicator_type": "excessive_punctuation",
                        "consecutive_questions": cq,
                        "consecutive_exclamations": ce,
                        "mixed_punctuation": mixed,
                    })),
            );
        }

        // Escalation patterns.
        let mut found_escalation = false;
        for pattern in escalation_patterns() {
            if norm_msg.matches_normalized_pattern(
                pattern,
                char_ngram_threshold,
                token_cosine_threshold,
            ) {
                group.add_signal(
                    SignalInstance::new(
                        SignalType::DisengagementEscalation,
                        *idx,
                        pattern.raw.clone(),
                    )
                    .with_metadata(json!({"pattern_type": "escalation"})),
                );
                found_escalation = true;
                break;
            }
        }

        // Quit patterns (independent of escalation).
        for pattern in quit_patterns() {
            if norm_msg.matches_normalized_pattern(
                pattern,
                char_ngram_threshold,
                token_cosine_threshold,
            ) {
                group.add_signal(
                    SignalInstance::new(SignalType::DisengagementQuit, *idx, pattern.raw.clone())
                        .with_metadata(json!({"pattern_type": "quit"})),
                );
                break;
            }
        }

        // Profanity (more specific) before generic negative stance.
        let mut found_profanity = false;
        for pattern in profanity_patterns() {
            if norm_msg.matches_normalized_pattern(
                pattern,
                char_ngram_threshold,
                token_cosine_threshold,
            ) {
                group.add_signal(
                    SignalInstance::new(
                        SignalType::DisengagementNegativeStance,
                        *idx,
                        pattern.raw.clone(),
                    )
                    .with_metadata(json!({
                        "indicator_type": "profanity",
                        "pattern": pattern.raw,
                    })),
                );
                found_profanity = true;
                break;
            }
        }

        if !found_escalation && !found_profanity {
            for pattern in negative_stance_patterns() {
                if norm_msg.matches_normalized_pattern(
                    pattern,
                    char_ngram_threshold,
                    token_cosine_threshold,
                ) {
                    group.add_signal(
                        SignalInstance::new(
                            SignalType::DisengagementNegativeStance,
                            *idx,
                            pattern.raw.clone(),
                        )
                        .with_metadata(json!({
                            "indicator_type": "complaint",
                            "pattern": pattern.raw,
                        })),
                    );
                    break;
                }
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
    fn detects_human_escalation_request() {
        let msgs = vec![(
            0usize,
            "human",
            nm("This is taking forever, get me a human"),
        )];
        let g = analyze_disengagement(&msgs, 0.65, 0.6);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::DisengagementEscalation)));
    }

    #[test]
    fn detects_quit_intent() {
        let msgs = vec![(0usize, "human", nm("Forget it, I give up"))];
        let g = analyze_disengagement(&msgs, 0.65, 0.6);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::DisengagementQuit)));
    }

    #[test]
    fn detects_negative_stance_complaint() {
        let msgs = vec![(0usize, "human", nm("This is useless"))];
        let g = analyze_disengagement(&msgs, 0.65, 0.6);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::DisengagementNegativeStance)));
    }

    #[test]
    fn detects_excessive_punctuation_as_negative_stance() {
        let msgs = vec![(0usize, "human", nm("WHY isn't this working???"))];
        let g = analyze_disengagement(&msgs, 0.65, 0.6);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::DisengagementNegativeStance)));
    }

    #[test]
    fn positive_excitement_is_not_disengagement() {
        let msgs = vec![(0usize, "human", nm("Yes!! That's perfect!!!"))];
        let g = analyze_disengagement(&msgs, 0.65, 0.6);
        assert!(g
            .signals
            .iter()
            .all(|s| !matches!(s.signal_type, SignalType::DisengagementNegativeStance)));
    }
}
