//! Stagnation signals: dragging (turn-count efficiency) and repetition.
//!
//! Direct port of `signals/interaction/stagnation.py`.

use serde_json::json;

use super::constants::{starts_with_prefix, POSITIVE_PREFIXES};
use crate::signals::schemas::{SignalGroup, SignalInstance, SignalType, TurnMetrics};
use crate::signals::text_processing::NormalizedMessage;

/// Adapter row used by stagnation::dragging detector. Mirrors the ShareGPT
/// `{"from": role, "value": text}` shape used in the Python reference.
pub struct ShareGptMsg<'a> {
    pub from: &'a str,
}

pub fn analyze_dragging(
    messages: &[ShareGptMsg<'_>],
    baseline_turns: usize,
    efficiency_threshold: f32,
) -> (SignalGroup, TurnMetrics) {
    let mut group = SignalGroup::new("stagnation");

    let mut user_turns: usize = 0;
    let mut assistant_turns: usize = 0;
    for m in messages {
        match m.from {
            "human" => user_turns += 1,
            "gpt" => assistant_turns += 1,
            _ => {}
        }
    }

    let total_turns = user_turns;
    let efficiency_score: f32 = if total_turns == 0 || total_turns <= baseline_turns {
        1.0
    } else {
        let excess = (total_turns - baseline_turns) as f32;
        1.0 / (1.0 + excess * 0.25)
    };

    let is_dragging = efficiency_score < efficiency_threshold;
    let metrics = TurnMetrics {
        total_turns,
        user_turns,
        assistant_turns,
        is_dragging,
        efficiency_score,
    };

    if is_dragging {
        let last_idx = messages.len().saturating_sub(1);
        group.add_signal(
            SignalInstance::new(
                SignalType::StagnationDragging,
                last_idx,
                format!(
                    "Conversation dragging: {} turns (efficiency: {:.2})",
                    total_turns, efficiency_score
                ),
            )
            .with_confidence(1.0 - efficiency_score)
            .with_metadata(json!({
                "total_turns": total_turns,
                "efficiency_score": efficiency_score,
                "baseline_turns": baseline_turns,
            })),
        );
    }

    (group, metrics)
}

pub fn analyze_repetition(
    normalized_messages: &[(usize, &str, NormalizedMessage)],
    lookback: usize,
    exact_threshold: f32,
    near_duplicate_threshold: f32,
) -> SignalGroup {
    let mut group = SignalGroup::new("stagnation");

    // We keep references into `normalized_messages`. Since `normalized_messages`
    // is borrowed for the whole function, this avoids cloning.
    let mut prev_human: Vec<(usize, &NormalizedMessage)> = Vec::new();
    let mut prev_gpt: Vec<(usize, &NormalizedMessage)> = Vec::new();

    for (idx, role, norm_msg) in normalized_messages {
        if *role != "human" && *role != "gpt" {
            continue;
        }

        // Skip human positive-prefix messages; they're naturally repetitive.
        if *role == "human" && starts_with_prefix(&norm_msg.raw, POSITIVE_PREFIXES) {
            prev_human.push((*idx, norm_msg));
            continue;
        }

        if norm_msg.tokens.len() < 5 {
            if *role == "human" {
                prev_human.push((*idx, norm_msg));
            } else {
                prev_gpt.push((*idx, norm_msg));
            }
            continue;
        }

        let prev = if *role == "human" {
            &prev_human
        } else {
            &prev_gpt
        };
        let start = prev.len().saturating_sub(lookback);
        let mut matched = false;
        for (prev_idx, prev_msg) in &prev[start..] {
            if prev_msg.tokens.len() < 5 {
                continue;
            }
            let similarity = norm_msg.ngram_similarity_with_message(prev_msg);
            if similarity >= exact_threshold {
                group.add_signal(
                    SignalInstance::new(
                        SignalType::StagnationRepetition,
                        *idx,
                        format!("Exact repetition with message {}", prev_idx),
                    )
                    .with_confidence(similarity)
                    .with_metadata(json!({
                        "repetition_type": "exact",
                        "compared_to": prev_idx,
                        "similarity": similarity,
                        "role": role,
                    })),
                );
                matched = true;
                break;
            } else if similarity >= near_duplicate_threshold {
                group.add_signal(
                    SignalInstance::new(
                        SignalType::StagnationRepetition,
                        *idx,
                        format!("Near-duplicate with message {}", prev_idx),
                    )
                    .with_confidence(similarity)
                    .with_metadata(json!({
                        "repetition_type": "near_duplicate",
                        "compared_to": prev_idx,
                        "similarity": similarity,
                        "role": role,
                    })),
                );
                matched = true;
                break;
            }
        }
        let _ = matched;

        if *role == "human" {
            prev_human.push((*idx, norm_msg));
        } else {
            prev_gpt.push((*idx, norm_msg));
        }
    }

    group
}

/// Combined stagnation analyzer: dragging + repetition.
pub fn analyze_stagnation(
    messages: &[ShareGptMsg<'_>],
    normalized_messages: &[(usize, &str, NormalizedMessage)],
    baseline_turns: usize,
) -> (SignalGroup, TurnMetrics) {
    let (dragging_group, metrics) = analyze_dragging(messages, baseline_turns, 0.5);
    let repetition_group = analyze_repetition(normalized_messages, 2, 0.95, 0.85);

    let mut combined = SignalGroup::new("stagnation");
    for s in dragging_group.signals.iter().cloned() {
        combined.add_signal(s);
    }
    for s in repetition_group.signals.iter().cloned() {
        combined.add_signal(s);
    }
    (combined, metrics)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nm(s: &str) -> NormalizedMessage {
        NormalizedMessage::from_text(s, 2000)
    }

    #[test]
    fn dragging_after_many_user_turns() {
        let msgs: Vec<_> = (0..15)
            .flat_map(|_| [ShareGptMsg { from: "human" }, ShareGptMsg { from: "gpt" }])
            .collect();
        let (g, m) = analyze_dragging(&msgs, 5, 0.5);
        assert!(m.is_dragging);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::StagnationDragging)));
    }

    #[test]
    fn no_dragging_below_baseline() {
        let msgs = vec![
            ShareGptMsg { from: "human" },
            ShareGptMsg { from: "gpt" },
            ShareGptMsg { from: "human" },
            ShareGptMsg { from: "gpt" },
        ];
        let (g, m) = analyze_dragging(&msgs, 5, 0.5);
        assert!(!m.is_dragging);
        assert!(g.signals.is_empty());
    }

    #[test]
    fn detects_exact_repetition_in_user_messages() {
        let n = vec![
            (
                0usize,
                "human",
                nm("This widget is broken and needs repair right now"),
            ),
            (1, "gpt", nm("Sorry to hear that. Let me look into it.")),
            (
                2,
                "human",
                nm("This widget is broken and needs repair right now"),
            ),
        ];
        let g = analyze_repetition(&n, 2, 0.95, 0.85);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::StagnationRepetition)));
    }
}
