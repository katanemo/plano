//! Top-level signal analyzer.
//!
//! Direct port of `signals/analyzer.py`. Orchestrates all detectors across
//! the three layers (interaction / execution / environment) and produces a
//! `SignalReport`.

use hermesllm::apis::openai::{Message, Role};
use hermesllm::transforms::ExtractText;

use super::environment::exhaustion::analyze_exhaustion;
use super::execution::failure::analyze_failure;
use super::execution::loops::analyze_loops;
use super::interaction::disengagement::analyze_disengagement;
use super::interaction::misalignment::analyze_misalignment;
use super::interaction::satisfaction::analyze_satisfaction;
use super::interaction::stagnation::{analyze_stagnation, ShareGptMsg};
use super::schemas::{
    EnvironmentSignals, ExecutionSignals, InteractionQuality, InteractionSignals, SignalReport,
    SignalType, TurnMetrics,
};
use super::text_processing::NormalizedMessage;

/// Marker appended to the span operation name when concerning signals are
/// detected. Kept in sync with the previous implementation for backward
/// compatibility with downstream consumers.
pub const FLAG_MARKER: &str = "[!]";

/// ShareGPT-shaped row used as the canonical input to the analyzer's
/// detectors. `from` is one of `"human"`, `"gpt"`, `"function_call"`,
/// `"observation"`. `value` is the raw message body.
#[derive(Debug, Clone, Copy)]
pub struct ShareGptMessage<'a> {
    pub from: &'a str,
    pub value: &'a str,
}

/// Configuration knobs for the analyzer. Defaults match
/// `signals/analyzer.py:SignalAnalyzer.__init__`.
#[derive(Debug, Clone)]
pub struct SignalAnalyzerConfig {
    pub baseline_turns: usize,
    pub char_ngram_threshold: f32,
    pub token_cosine_threshold: f32,
    pub max_message_length: usize,
    pub max_messages: usize,
}

impl Default for SignalAnalyzerConfig {
    fn default() -> Self {
        Self {
            baseline_turns: 5,
            char_ngram_threshold: 0.65,
            token_cosine_threshold: 0.60,
            max_message_length: 2000,
            max_messages: 100,
        }
    }
}

/// Top-level analyzer.
pub struct SignalAnalyzer {
    cfg: SignalAnalyzerConfig,
}

impl Default for SignalAnalyzer {
    fn default() -> Self {
        Self::new(SignalAnalyzerConfig::default())
    }
}

impl SignalAnalyzer {
    pub fn new(cfg: SignalAnalyzerConfig) -> Self {
        Self { cfg }
    }

    /// Run the full multi-layer analysis on a ShareGPT-shaped conversation.
    pub fn analyze_sharegpt(&self, messages: &[ShareGptMessage<'_>]) -> SignalReport {
        // Truncate to the last `max_messages` (last-N is what the Python does).
        let slice: &[ShareGptMessage<'_>] = if messages.len() > self.cfg.max_messages {
            &messages[messages.len() - self.cfg.max_messages..]
        } else {
            messages
        };
        let offset = messages.len().saturating_sub(slice.len());

        // Preprocess to absolute-indexed normalized human/gpt messages.
        let normalized_owned: Vec<(usize, &str, NormalizedMessage)> = slice
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                if (m.from == "human" || m.from == "gpt") && !m.value.is_empty() {
                    Some((
                        offset + i,
                        m.from,
                        NormalizedMessage::from_text(m.value, self.cfg.max_message_length),
                    ))
                } else {
                    None
                }
            })
            .collect();

        let misalignment = analyze_misalignment(
            &normalized_owned,
            self.cfg.char_ngram_threshold,
            self.cfg.token_cosine_threshold,
        );

        let stagnation_input: Vec<ShareGptMsg<'_>> =
            slice.iter().map(|m| ShareGptMsg { from: m.from }).collect();
        let (mut stagnation, turn_metrics) = analyze_stagnation(
            &stagnation_input,
            &normalized_owned,
            self.cfg.baseline_turns,
        );

        let disengagement = analyze_disengagement(
            &normalized_owned,
            self.cfg.char_ngram_threshold,
            self.cfg.token_cosine_threshold,
        );

        let satisfaction = analyze_satisfaction(
            &normalized_owned,
            self.cfg.char_ngram_threshold,
            self.cfg.token_cosine_threshold,
        );

        let failure = analyze_failure(slice);
        let loops = analyze_loops(slice);
        let exhaustion = analyze_exhaustion(slice);

        // Bias the dragging signal's message_index back into absolute coords.
        for s in &mut stagnation.signals {
            s.message_index = offset + s.message_index.min(slice.len().saturating_sub(1));
        }

        let interaction = InteractionSignals {
            misalignment,
            stagnation,
            disengagement,
            satisfaction,
        };
        let execution = ExecutionSignals { failure, loops };
        let environment = EnvironmentSignals { exhaustion };

        let (overall_quality, score) = assess_quality(
            &interaction,
            &execution,
            &environment,
            turn_metrics.user_turns,
        );
        let summary = generate_summary(
            &turn_metrics,
            &interaction,
            &execution,
            &environment,
            overall_quality,
        );

        SignalReport {
            interaction,
            execution,
            environment,
            overall_quality,
            quality_score: score,
            turn_metrics,
            summary,
        }
    }

    /// Convenience entry point: convert OpenAI-shaped chat `Message`s into the
    /// ShareGPT format the detectors operate on, then run analysis.
    pub fn analyze_openai(&self, messages: &[Message]) -> SignalReport {
        let owned = messages_to_sharegpt(messages);
        let view: Vec<ShareGptMessage<'_>> = owned
            .iter()
            .map(|(role, value)| ShareGptMessage {
                from: role.as_str(),
                value: value.as_str(),
            })
            .collect();
        self.analyze_sharegpt(&view)
    }
}

/// Convert OpenAI-shaped messages to a sequence of ShareGPT
/// `(role, value)` pairs.
///
/// Mapping (preserves original message order; tool calls are emitted as a
/// separate `function_call` row immediately after the assistant text):
///
/// - `User` -> `("human", text)`
/// - `Assistant` -> `("gpt", text)`, then one `("function_call", json)` per tool call
/// - `Tool` -> `("observation", text)`
/// - `System` / `Developer` -> dropped (not analyzed)
pub fn messages_to_sharegpt(messages: &[Message]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::with_capacity(messages.len());
    for m in messages {
        match m.role {
            Role::User => {
                let text = m.content.extract_text();
                out.push(("human".to_string(), text));
            }
            Role::Assistant => {
                let text = m.content.extract_text();
                if !text.is_empty() {
                    out.push(("gpt".to_string(), text));
                }
                if let Some(calls) = &m.tool_calls {
                    for call in calls {
                        let payload = serde_json::json!({
                            "name": call.function.name,
                            "arguments": call.function.arguments,
                        });
                        out.push(("function_call".to_string(), payload.to_string()));
                    }
                }
            }
            Role::Tool => {
                let text = m.content.extract_text();
                out.push(("observation".to_string(), text));
            }
            Role::System | Role::Developer => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Quality scoring (mirrors `_assess_quality` in the reference)
// ---------------------------------------------------------------------------

fn assess_quality(
    interaction: &InteractionSignals,
    execution: &ExecutionSignals,
    environment: &EnvironmentSignals,
    user_turns: usize,
) -> (InteractionQuality, f32) {
    // Critical: explicit escalation/quit OR severe disengagement OR severe stagnation.
    let has_escalation_or_quit = interaction.disengagement.signals.iter().any(|s| {
        matches!(
            s.signal_type,
            SignalType::DisengagementEscalation | SignalType::DisengagementQuit
        )
    });
    if (interaction.disengagement.count > 0 && has_escalation_or_quit)
        || interaction.disengagement.severity >= 3
        || interaction.stagnation.severity >= 3
    {
        return (InteractionQuality::Severe, 0.0);
    }

    let mut score: f32 = 50.0;

    if interaction.satisfaction.count > 0 {
        let confidence = match interaction.satisfaction.count {
            1 => 0.6,
            2 => 0.8,
            _ => 0.95,
        };
        score += 20.0 * confidence;
    }

    if interaction.disengagement.count > 0 {
        score -= interaction.disengagement.severity as f32 * 10.0;
    }
    if interaction.misalignment.severity > 0 {
        let denom = user_turns.max(1) as f32;
        if interaction.misalignment.count as f32 / denom > 0.3 {
            score -= 15.0;
        }
    }
    if interaction.stagnation.count > 2 {
        score -= interaction.stagnation.severity as f32 * 8.0;
    }

    if execution.failure.count > 0 {
        score -= execution.failure.count as f32 * 8.0;
    }
    if execution.loops.count > 0 {
        score -= execution.loops.count as f32 * 5.0;
    }
    if environment.exhaustion.count > 0 {
        score -= environment.exhaustion.count as f32 * 3.0;
    }

    score = score.clamp(0.0, 100.0);

    let quality = if score >= 75.0 {
        InteractionQuality::Excellent
    } else if score >= 60.0 {
        InteractionQuality::Good
    } else if score >= 40.0 {
        InteractionQuality::Neutral
    } else if score >= 25.0 {
        InteractionQuality::Poor
    } else {
        InteractionQuality::Severe
    };
    (quality, score)
}

fn generate_summary(
    turn_metrics: &TurnMetrics,
    interaction: &InteractionSignals,
    execution: &ExecutionSignals,
    environment: &EnvironmentSignals,
    quality: InteractionQuality,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("Overall Quality: {}", quality.as_str()));
    parts.push(format!(
        "Turn Count: {} turns (efficiency: {:.1}%)",
        turn_metrics.total_turns,
        turn_metrics.efficiency_score * 100.0
    ));

    if interaction.misalignment.count > 0 {
        let denom = turn_metrics.user_turns.max(1) as f32;
        let repair_ratio = interaction.misalignment.count as f32 / denom;
        if repair_ratio > 0.3 {
            parts.push(format!(
                "High misalignment rate: {:.1}% of user turns",
                repair_ratio * 100.0
            ));
        }
    }

    if interaction.disengagement.count > 0 {
        parts.push(format!(
            "Disengagement detected: {} indicators (severity: {})",
            interaction.disengagement.count, interaction.disengagement.severity
        ));
    }

    if interaction.stagnation.count > 2 {
        parts.push(format!(
            "Looping detected: {} repetitions",
            interaction.stagnation.count
        ));
    }

    if interaction.satisfaction.count > 0 {
        parts.push(format!(
            "Positive feedback: {} indicators",
            interaction.satisfaction.count
        ));
    }

    if execution.failure.count > 0 {
        parts.push(format!(
            "Execution failures: {} (agent-caused)",
            execution.failure.count
        ));
    }

    if environment.exhaustion.count > 0 {
        parts.push(format!(
            "Environment issues: {} (external)",
            environment.exhaustion.count
        ));
    }

    let escalation_count = interaction
        .disengagement
        .signals
        .iter()
        .filter(|s| matches!(s.signal_type, SignalType::DisengagementEscalation))
        .count();
    if escalation_count > 0 {
        parts.push(format!(
            "Escalation requested: {} requests",
            escalation_count
        ));
    }

    parts.join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hermesllm::apis::openai::{Message, MessageContent, Role};
    #[allow(unused_imports)]
    use hermesllm::transforms::ExtractText;

    fn user(t: &str) -> Message {
        Message {
            role: Role::User,
            content: Some(MessageContent::Text(t.to_string())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }
    fn assistant(t: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: Some(MessageContent::Text(t.to_string())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    #[test]
    fn report_quality_neutral_for_short_clean_chat() {
        let msgs = vec![
            user("Hello, can you help me with a question?"),
            assistant("Of course, what's your question?"),
            user("How does X work?"),
            assistant("X works by ..."),
        ];
        let r = SignalAnalyzer::default().analyze_openai(&msgs);
        assert!(matches!(
            r.overall_quality,
            InteractionQuality::Neutral | InteractionQuality::Good | InteractionQuality::Excellent
        ));
        assert!(r.summary.starts_with("Overall Quality:"));
    }

    #[test]
    fn report_severe_when_user_escalates() {
        let msgs = vec![
            user("This isn't helpful at all"),
            assistant("I'm sorry, can you tell me more?"),
            user("Get me a human, this is useless"),
        ];
        let r = SignalAnalyzer::default().analyze_openai(&msgs);
        assert_eq!(r.overall_quality, InteractionQuality::Severe);
        assert!(r
            .interaction
            .disengagement
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::DisengagementEscalation)));
    }

    #[test]
    fn report_excellent_when_user_satisfied() {
        let msgs = vec![
            user("Can you summarize this report?"),
            assistant("Here's a summary: ..."),
            user("That's perfect, exactly what I needed, you're awesome!"),
        ];
        let r = SignalAnalyzer::default().analyze_openai(&msgs);
        assert!(r.interaction.satisfaction.count > 0);
        assert!(matches!(
            r.overall_quality,
            InteractionQuality::Good | InteractionQuality::Excellent
        ));
    }

    #[test]
    fn repro_gratitude_does_not_trigger_misalignment() {
        let msgs = vec![
            user("What is the weather in Istanbul?"),
            assistant("Istanbul is 14C and partly cloudy."),
            user("That worked, exactly what I needed. Thanks, that is perfect!"),
        ];
        let r = SignalAnalyzer::default().analyze_openai(&msgs);
        for s in &r.interaction.misalignment.signals {
            eprintln!(
                "misalignment fired: type={:?} idx={} snippet={:?} meta={:?}",
                s.signal_type, s.message_index, s.snippet, s.metadata
            );
        }
        assert_eq!(
            r.interaction.misalignment.count, 0,
            "a pure gratitude message should not trigger repair/misalignment"
        );
        assert!(r.interaction.satisfaction.count > 0);
    }

    #[test]
    fn execution_failures_lower_quality() {
        let msgs = vec![ShareGptMessage {
            from: "human",
            value: "do the thing",
        }];
        let _ = msgs;
        // Build a synthetic ShareGPT input with multiple tool failures.
        let convo = vec![
            ShareGptMessage {
                from: "human",
                value: "create a user",
            },
            ShareGptMessage {
                from: "function_call",
                value: r#"{"name":"create_user","arguments":{"age":"twelve"}}"#,
            },
            ShareGptMessage {
                from: "observation",
                value: "Error: validation failed - expected integer got string",
            },
            ShareGptMessage {
                from: "function_call",
                value: r#"{"name":"create_user","arguments":{}}"#,
            },
            ShareGptMessage {
                from: "observation",
                value: "missing required field: name",
            },
        ];
        let r = SignalAnalyzer::default().analyze_sharegpt(&convo);
        assert!(r.execution.failure.count >= 1);
        assert!(r.quality_score < 50.0);
    }
}
