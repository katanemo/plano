//! Top-level signal analyzer.
//!
//! Direct port of `signals/analyzer.py`. Orchestrates all detectors across
//! the three layers (interaction / execution / environment) and produces a
//! `SignalReport`.
//!
//! Two entry points:
//!
//! - [`SignalAnalyzer::analyze_sharegpt`] / [`SignalAnalyzer::analyze_openai`]:
//!   grade a whole conversation in one pass.
//! - [`SignalAnalyzer::analyze_step`]: grade the newest message using the
//!   report returned for the previous message. Every detector only looks a
//!   bounded distance backwards, so this does constant work per text message
//!   regardless of conversation length. [`SignalAnalyzer::get_message_reports`]
//!   is built on it. Mirrors the `analyze_step` / `get_message_reports`
//!   optimization in `signals/analyzer.py` (window + previous report).

use std::collections::HashMap;

use hermesllm::apis::openai::{Message, Role};
use hermesllm::transforms::ExtractText;
use serde_json::json;

use super::environment::exhaustion::analyze_exhaustion;
use super::execution::failure::analyze_failure;
use super::execution::loops::analyze_loops;
use super::interaction::disengagement::analyze_disengagement;
use super::interaction::misalignment::analyze_misalignment;
use super::interaction::satisfaction::analyze_satisfaction;
use super::interaction::stagnation::{
    analyze_repetition, analyze_stagnation, compute_turn_metrics, dragging_signal, ShareGptMsg,
};
use super::schemas::{
    EnvironmentSignals, ExecutionSignals, InteractionQuality, InteractionSignals, MessageReport,
    SignalGroup, SignalInstance, SignalReport, SignalType, TurnMetrics,
};
use super::text_processing::NormalizedMessage;

/// Marker appended to the span operation name when concerning signals are
/// detected. The 🚩 emoji (U+1F6A9) matches the pre-port implementation so
/// downstream consumers that search for flagged traces by span-name emoji
/// keep working.
pub const FLAG_MARKER: &str = "\u{1F6A9}";

/// ShareGPT-shaped row used as the canonical input to the analyzer's
/// detectors. `from` is one of `"human"`, `"gpt"`, `"function_call"`,
/// `"observation"`. `value` is the raw message body.
#[derive(Debug, Clone, Copy)]
pub struct ShareGptMessage<'a> {
    pub from: &'a str,
    pub value: &'a str,
}

/// Normalized message tagged with its absolute index and role, as consumed
/// by the interaction detectors.
pub type TaggedMessage<'a> = (usize, &'a str, NormalizedMessage);

/// Group priority used by [`SignalAnalyzer::step`] to pick the single signal
/// reported for a message.
const MESSAGE_REPORT_PRIORITY: [&str; 7] = [
    "disengagement", // user frustration / quit (most severe)
    "satisfaction",  // user thanks / success (positive feedback)
    "misalignment",  // user corrections
    "failure",       // execution failures (agent-caused)
    "loops",         // execution loops
    "exhaustion",    // external errors
    "stagnation",    // general dragging (lowest priority)
];

/// Configuration knobs for the analyzer. Defaults match
/// `signals/analyzer.py:SignalAnalyzer.__init__`.
#[derive(Debug, Clone)]
pub struct SignalAnalyzerConfig {
    pub baseline_turns: usize,
    pub char_ngram_threshold: f32,
    pub token_cosine_threshold: f32,
    pub max_message_length: usize,
    /// Maximum number of messages `analyze_sharegpt`/`analyze_openai` process
    /// (tail kept).
    pub max_messages: usize,
    /// Maximum number of messages `analyze_step` scans backwards for the
    /// previous same-role messages or function call.
    pub context_lookback: usize,
}

impl Default for SignalAnalyzerConfig {
    fn default() -> Self {
        Self {
            baseline_turns: 5,
            char_ngram_threshold: 0.65,
            token_cosine_threshold: 0.60,
            max_message_length: 2000,
            max_messages: 100,
            context_lookback: 200,
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

    // ------------------------------------------------------------------
    // Incremental analysis
    // ------------------------------------------------------------------

    /// Analyze the newest message of a conversation incrementally.
    ///
    /// Given the conversation so far and the report returned for the
    /// previous message, detects the signals contributed by the last
    /// message, merges them into the previous report, and re-assesses
    /// quality. Only the newest message is normalized; earlier messages are
    /// touched only for the bounded context each detector needs. Does
    /// constant work per call regardless of conversation length, unlike
    /// re-running [`SignalAnalyzer::analyze_sharegpt`] on the whole
    /// conversation on every new message.
    ///
    /// `messages` is the conversation so far (newest message last);
    /// `prev_report` is the report returned by the previous call, or `None`
    /// for the first message.
    pub fn analyze_step(
        &self,
        messages: &[ShareGptMessage<'_>],
        prev_report: Option<&SignalReport>,
    ) -> (MessageReport, SignalReport) {
        self.step(messages, messages.len() - 1, prev_report)
    }

    /// Analyze a conversation and return message-level reports, one per
    /// message, each with the cumulative quality assessment from the start
    /// of the conversation up to that message. Built on `analyze_step`.
    pub fn get_message_reports(&self, messages: &[ShareGptMessage<'_>]) -> Vec<MessageReport> {
        let mut reports = Vec::with_capacity(messages.len());
        let mut prev: Option<SignalReport> = None;
        for i in 0..messages.len() {
            let (entry, report) = self.step(messages, i, prev.as_ref());
            prev = Some(report);
            reports.push(entry);
        }
        reports
    }

    /// Analyze message `i` given the report for messages `0..i`.
    fn step(
        &self,
        messages: &[ShareGptMessage<'_>],
        i: usize,
        prev_report: Option<&SignalReport>,
    ) -> (MessageReport, SignalReport) {
        let msg = messages[i];
        let role = msg.from;
        let value = msg.value;

        let prev = prev_report.cloned().unwrap_or_default();
        let prev_counts = signal_counts(&prev);

        // Carry forward previous signals. Dragging is re-derived each step,
        // so only repetition signals are carried from the stagnation group.
        let mut misalignment_sig = prev.interaction.misalignment.signals.clone();
        let mut disengagement_sig = prev.interaction.disengagement.signals.clone();
        let mut satisfaction_sig = prev.interaction.satisfaction.signals.clone();
        let mut failure_sig = prev.execution.failure.signals.clone();
        let mut exhaustion_sig = prev.environment.exhaustion.signals.clone();
        let mut stagnation_sig: Vec<SignalInstance> = prev
            .interaction
            .stagnation
            .signals
            .iter()
            .filter(|s| !matches!(s.signal_type, SignalType::StagnationDragging))
            .cloned()
            .collect();
        let mut loops_group = prev.execution.loops.clone();

        if (role == "human" || role == "gpt") && !value.is_empty() {
            let norm = NormalizedMessage::from_text(value, self.cfg.max_message_length);
            let same_role = self.previous_same_role(messages, i, role);

            if role == "human" {
                // Misalignment compares against the previous human message;
                // the detector itself applies the distance rule.
                let mut ctx: Vec<TaggedMessage> = Vec::with_capacity(2);
                if let Some(last) = same_role.last() {
                    ctx.push(last.clone());
                }
                ctx.push((i, role, norm.clone()));
                let group = analyze_misalignment(
                    &ctx,
                    self.cfg.char_ngram_threshold,
                    self.cfg.token_cosine_threshold,
                );
                misalignment_sig.extend(at_index(&group, i));

                let single = [(i, role, norm.clone())];
                disengagement_sig.extend(
                    analyze_disengagement(
                        &single,
                        self.cfg.char_ngram_threshold,
                        self.cfg.token_cosine_threshold,
                    )
                    .signals,
                );
                satisfaction_sig.extend(
                    analyze_satisfaction(
                        &single,
                        self.cfg.char_ngram_threshold,
                        self.cfg.token_cosine_threshold,
                    )
                    .signals,
                );
            }

            // Repetition compares against the last two same-role messages.
            let mut rep_ctx = same_role;
            rep_ctx.push((i, role, norm));
            let repetition_group = analyze_repetition(&rep_ctx, 2, 0.95, 0.85);
            stagnation_sig.extend(at_index(&repetition_group, i));
        } else if role == "observation" {
            let lower_bound = i.saturating_sub(self.cfg.context_lookback);
            let mut call_index: Option<usize> = None;
            for j in (lower_bound..i).rev() {
                if messages[j].from == "function_call" {
                    call_index = Some(j);
                    break;
                }
            }

            // Run the detectors on the (call, observation) pair and remap
            // indices back to absolute conversation coordinates.
            let ctx: Vec<ShareGptMessage<'_>> = match call_index {
                Some(ci) => vec![messages[ci], messages[i]],
                None => vec![messages[i]],
            };
            let mut failure_group = analyze_failure(&ctx);
            for s in failure_group.signals.iter_mut() {
                s.message_index = i;
                if let Some(obj) = s.metadata.as_object_mut() {
                    obj.insert(
                        "call_index".to_string(),
                        json!(call_index.unwrap_or(i.saturating_sub(1))),
                    );
                }
            }
            failure_sig.extend(failure_group.signals);

            let mut exhaustion_group = analyze_exhaustion(&[messages[i]]);
            for s in exhaustion_group.signals.iter_mut() {
                s.message_index = i;
            }
            exhaustion_sig.extend(exhaustion_group.signals);
        } else if role == "function_call" {
            // Loop detection needs the tool-call sequence; recompute only
            // when a new call arrives.
            loops_group = analyze_loops(&messages[..=i]);
        }

        // Turn metrics and dragging.
        let user_turns = prev.turn_metrics.user_turns + usize::from(role == "human");
        let assistant_turns = prev.turn_metrics.assistant_turns + usize::from(role == "gpt");
        let turn_metrics =
            compute_turn_metrics(user_turns, assistant_turns, self.cfg.baseline_turns, 0.5);
        if turn_metrics.is_dragging {
            stagnation_sig.insert(
                0,
                dragging_signal(&turn_metrics, i, self.cfg.baseline_turns),
            );
        }

        let interaction = InteractionSignals {
            misalignment: make_group("misalignment", misalignment_sig),
            stagnation: make_group("stagnation", stagnation_sig),
            disengagement: make_group("disengagement", disengagement_sig),
            satisfaction: make_group("satisfaction", satisfaction_sig),
        };
        let execution = ExecutionSignals {
            failure: make_group("failure", failure_sig),
            loops: loops_group,
        };
        let environment = EnvironmentSignals {
            exhaustion: make_group("exhaustion", exhaustion_sig),
        };

        let (quality, score) = assess_quality(
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
            quality,
        );
        let report = SignalReport {
            interaction,
            execution,
            environment,
            overall_quality: quality,
            quality_score: score,
            turn_metrics,
            summary,
        };

        let entry = message_report(msg, i, &report, score, &prev_counts);
        (entry, report)
    }

    /// Last two non-empty messages of `role` before `i`, oldest first,
    /// scanning back at most `context_lookback` messages.
    fn previous_same_role<'a>(
        &self,
        messages: &[ShareGptMessage<'a>],
        i: usize,
        role: &'a str,
    ) -> Vec<TaggedMessage<'a>> {
        let mut found: Vec<TaggedMessage<'a>> = Vec::with_capacity(2);
        let lower_bound = i.saturating_sub(self.cfg.context_lookback);
        for j in (lower_bound..i).rev() {
            let m = messages[j];
            if m.from == role && !m.value.is_empty() {
                found.push((
                    j,
                    role,
                    NormalizedMessage::from_text(m.value, self.cfg.max_message_length),
                ));
                if found.len() == 2 {
                    break;
                }
            }
        }
        found.reverse();
        found
    }
}

/// Signals of a group located at message `i`.
fn at_index(group: &SignalGroup, i: usize) -> Vec<SignalInstance> {
    group
        .signals
        .iter()
        .filter(|s| s.message_index == i)
        .cloned()
        .collect()
}

fn make_group(category: &str, signals: Vec<SignalInstance>) -> SignalGroup {
    let mut group = SignalGroup::new(category);
    for s in signals {
        group.add_signal(s);
    }
    group
}

/// Number of signal instances per signal type in a report.
fn signal_counts(report: &SignalReport) -> HashMap<&'static str, usize> {
    let mut counts = HashMap::new();
    for s in report.iter_signals() {
        *counts.entry(s.signal_type.as_str()).or_insert(0) += 1;
    }
    counts
}

/// Build the message-level report for message `i`.
///
/// Picks the highest-priority signal located at this message whose type
/// count grew since the previous message. The count check stops
/// conversation-level signals (e.g. dragging) from being re-flagged on every
/// message once they have fired.
fn message_report(
    msg: ShareGptMessage<'_>,
    i: usize,
    report: &SignalReport,
    score: f32,
    prev_counts: &HashMap<&'static str, usize>,
) -> MessageReport {
    let counts = signal_counts(report);
    let new_types: std::collections::HashSet<&'static str> = counts
        .iter()
        .filter(|(t, c)| **c > *prev_counts.get(**t).unwrap_or(&0))
        .map(|(t, _)| *t)
        .collect();

    let mut signal_class: Option<String> = None;
    let mut signal_type: Option<String> = None;
    let mut matched: Option<String> = None;

    'outer: for name in MESSAGE_REPORT_PRIORITY {
        let group: &SignalGroup = match name {
            "disengagement" => &report.interaction.disengagement,
            "satisfaction" => &report.interaction.satisfaction,
            "misalignment" => &report.interaction.misalignment,
            "failure" => &report.execution.failure,
            "loops" => &report.execution.loops,
            "exhaustion" => &report.environment.exhaustion,
            "stagnation" => &report.interaction.stagnation,
            _ => unreachable!(),
        };
        for s in &group.signals {
            if s.message_index == i && new_types.contains(s.signal_type.as_str()) {
                // "interaction.misalignment.correction" -> ("interaction", "misalignment.correction")
                let full = s.signal_type.as_str();
                if let Some((cls, ty)) = full.split_once('.') {
                    signal_class = Some(cls.to_string());
                    signal_type = Some(ty.to_string());
                }
                matched = Some(s.snippet.clone());
                break 'outer;
            }
        }
    }

    let label = match (&signal_class, signal_type.as_deref()) {
        (None, _) => "neutral",
        (Some(_), Some(ty)) if ty.contains("satisfaction") => "positive",
        // Clarifications are too generic/mild to flag as negative.
        (Some(_), Some(ty)) if ty.contains("clarification") => "neutral",
        _ => "negative",
    };

    MessageReport {
        role: msg.from.to_string(),
        content: msg.value.to_string(),
        quality: report.overall_quality.as_str().to_string(),
        score,
        label: label.to_string(),
        signal_class,
        signal_type,
        matched,
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
    if interaction.misalignment.severity > 0 && interaction.misalignment_ratio(user_turns) > 0.3 {
        score -= 15.0;
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

/// Render the per-conversation summary string.
///
/// Output is structurally grouped by the paper taxonomy so a reader can see
/// at a glance which layer fired:
///
/// ```text
/// Overall Quality: severe | Turns: 7 (efficiency: 71.4%)
///  | Interaction — misalignment: 2 (sev 1), stagnation: 0, disengagement: 2 (sev 1), satisfaction: 0
///  | Execution — failure: 0, loops: 0
///  | Environment — exhaustion: 0
///  | High misalignment rate: 50.0% of user turns
///  | Escalation requested: 1
/// ```
///
/// Layer headers are always present (even when their counts are all zero) so
/// the taxonomy is visible by inspection. Quality-driving callouts —
/// "high misalignment rate", "looping detected", "escalation requested" —
/// are appended after the layer summary as a separate "alerts" tail.
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
        "Turns: {} (efficiency: {:.1}%)",
        turn_metrics.total_turns,
        turn_metrics.efficiency_score * 100.0
    ));

    parts.push(format!(
        "Interaction \u{2014} {}, {}, {}, {}",
        fmt_group("misalignment", &interaction.misalignment),
        fmt_group("stagnation", &interaction.stagnation),
        fmt_group("disengagement", &interaction.disengagement),
        fmt_group("satisfaction", &interaction.satisfaction),
    ));
    parts.push(format!(
        "Execution \u{2014} {}, {}",
        fmt_group("failure", &execution.failure),
        fmt_group("loops", &execution.loops),
    ));
    parts.push(format!(
        "Environment \u{2014} {}",
        fmt_group("exhaustion", &environment.exhaustion),
    ));

    if interaction.misalignment.count > 0 {
        let misalignment_ratio = interaction.misalignment_ratio(turn_metrics.user_turns);
        if misalignment_ratio > 0.3 {
            parts.push(format!(
                "High misalignment rate: {:.1}% of user turns",
                misalignment_ratio * 100.0
            ));
        }
    }
    if interaction.stagnation.count > 2 {
        parts.push(format!(
            "Looping detected: {} repetitions",
            interaction.stagnation.count
        ));
    }
    let escalation_count = interaction
        .disengagement
        .signals
        .iter()
        .filter(|s| matches!(s.signal_type, SignalType::DisengagementEscalation))
        .count();
    if escalation_count > 0 {
        parts.push(format!("Escalation requested: {}", escalation_count));
    }

    parts.join(" | ")
}

/// Render `"<name>: <count> (sev <severity>)"`, dropping the severity suffix
/// when the count is zero (keeps the summary readable for clean conversations).
fn fmt_group(name: &str, group: &super::schemas::SignalGroup) -> String {
    if group.count == 0 {
        format!("{}: 0", name)
    } else {
        format!("{}: {} (sev {})", name, group.count, group.severity)
    }
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
    fn summary_groups_signals_by_taxonomy() {
        // Even on a clean conversation the summary should expose the three
        // layer headers so the taxonomy is visible.
        let msgs = vec![
            user("Hello"),
            assistant("Hi! How can I help?"),
            user("What's 2 + 2?"),
            assistant("4"),
        ];
        let r = SignalAnalyzer::default().analyze_openai(&msgs);
        assert!(
            r.summary.contains("Interaction \u{2014}"),
            "missing Interaction header in: {}",
            r.summary
        );
        assert!(
            r.summary.contains("Execution \u{2014}"),
            "missing Execution header in: {}",
            r.summary
        );
        assert!(
            r.summary.contains("Environment \u{2014}"),
            "missing Environment header in: {}",
            r.summary
        );
        assert!(r.summary.contains("misalignment: 0"));
        assert!(r.summary.contains("loops: 0"));
        assert!(r.summary.contains("exhaustion: 0"));
    }

    #[test]
    fn summary_includes_severity_when_signals_fire() {
        let msgs = vec![
            user("This isn't helpful at all"),
            assistant("I'm sorry, can you tell me more?"),
            user("Get me a human, this is useless"),
        ];
        let r = SignalAnalyzer::default().analyze_openai(&msgs);
        // Disengagement fires; should render with `(sev N)` and the
        // escalation-requested alert tail.
        assert!(
            r.summary.contains("disengagement:") && r.summary.contains("(sev "),
            "expected severity rendered for disengagement: {}",
            r.summary
        );
        assert!(
            r.summary.contains("Escalation requested:"),
            "expected escalation alert in: {}",
            r.summary
        );
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

    // -----------------------------------------------------------------
    // Incremental analysis (analyze_step / get_message_reports) parity
    // with the whole-conversation analyze_sharegpt. Mirrors the
    // reference's `tests/test_incremental.py` equivalence property:
    // stepping through a conversation message-by-message must land on
    // the same signal counts and overall quality as batch analysis.
    // -----------------------------------------------------------------

    fn step_through(messages: &[ShareGptMessage<'_>]) -> SignalReport {
        let analyzer = SignalAnalyzer::default();
        let mut prev: Option<SignalReport> = None;
        for i in 0..messages.len() {
            let (_, report) = analyzer.step(messages, i, prev.as_ref());
            prev = Some(report);
        }
        prev.expect("non-empty conversation")
    }

    fn assert_same_signal_counts(batch: &SignalReport, incremental: &SignalReport, case: &str) {
        assert_eq!(
            signal_counts(batch),
            signal_counts(incremental),
            "{case}: signal counts diverged between batch and incremental analysis"
        );
        assert_eq!(
            batch.overall_quality, incremental.overall_quality,
            "{case}: overall quality diverged"
        );
    }

    #[test]
    fn incremental_matches_batch_for_misalignment() {
        let convo = vec![
            ShareGptMessage {
                from: "human",
                value: "Book me a flight to Paris for next Tuesday",
            },
            ShareGptMessage {
                from: "gpt",
                value: "Booking a flight to Paris for next Monday.",
            },
            ShareGptMessage {
                from: "human",
                value: "No, I meant Tuesday, not Monday",
            },
            ShareGptMessage {
                from: "gpt",
                value: "Apologies, booking for Tuesday instead.",
            },
        ];
        let batch = SignalAnalyzer::default().analyze_sharegpt(&convo);
        let incremental = step_through(&convo);
        assert_same_signal_counts(&batch, &incremental, "misalignment");
        assert!(batch.interaction.misalignment.count > 0);
    }

    #[test]
    fn incremental_matches_batch_for_dragging() {
        let convo: Vec<ShareGptMessage<'_>> = (0..15)
            .flat_map(|_| {
                [
                    ShareGptMessage {
                        from: "human",
                        value: "Can you try that again?",
                    },
                    ShareGptMessage {
                        from: "gpt",
                        value: "Sure, let me try again.",
                    },
                ]
            })
            .collect();
        let batch = SignalAnalyzer::default().analyze_sharegpt(&convo);
        let incremental = step_through(&convo);
        assert_same_signal_counts(&batch, &incremental, "dragging");
        assert!(batch.turn_metrics.is_dragging);
    }

    #[test]
    fn incremental_matches_batch_for_repetition() {
        let convo = vec![
            ShareGptMessage {
                from: "human",
                value: "This widget is broken and needs repair right now",
            },
            ShareGptMessage {
                from: "gpt",
                value: "Sorry to hear that. Let me look into it.",
            },
            ShareGptMessage {
                from: "human",
                value: "This widget is broken and needs repair right now",
            },
            ShareGptMessage {
                from: "gpt",
                value: "I understand, looking into the widget issue now.",
            },
        ];
        let batch = SignalAnalyzer::default().analyze_sharegpt(&convo);
        let incremental = step_through(&convo);
        assert_same_signal_counts(&batch, &incremental, "repetition");
        assert!(batch.interaction.stagnation.count > 0);
    }

    #[test]
    fn incremental_matches_batch_for_disengagement() {
        let convo = vec![
            ShareGptMessage {
                from: "human",
                value: "This isn't helpful at all",
            },
            ShareGptMessage {
                from: "gpt",
                value: "I'm sorry, can you tell me more?",
            },
            ShareGptMessage {
                from: "human",
                value: "Get me a human, this is useless",
            },
        ];
        let batch = SignalAnalyzer::default().analyze_sharegpt(&convo);
        let incremental = step_through(&convo);
        assert_same_signal_counts(&batch, &incremental, "disengagement");
        assert_eq!(batch.overall_quality, InteractionQuality::Severe);
    }

    #[test]
    fn incremental_matches_batch_for_satisfaction() {
        let convo = vec![
            ShareGptMessage {
                from: "human",
                value: "Can you summarize this report?",
            },
            ShareGptMessage {
                from: "gpt",
                value: "Here's a summary: ...",
            },
            ShareGptMessage {
                from: "human",
                value: "That's perfect, exactly what I needed, you're awesome!",
            },
        ];
        let batch = SignalAnalyzer::default().analyze_sharegpt(&convo);
        let incremental = step_through(&convo);
        assert_same_signal_counts(&batch, &incremental, "satisfaction");
        assert!(batch.interaction.satisfaction.count > 0);
    }

    #[test]
    fn incremental_matches_batch_for_failure_and_loops() {
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
                value: r#"{"name":"create_user","arguments":{"age":"twelve"}}"#,
            },
            ShareGptMessage {
                from: "observation",
                value: "Error: validation failed - expected integer got string",
            },
            ShareGptMessage {
                from: "function_call",
                value: r#"{"name":"create_user","arguments":{"age":"twelve"}}"#,
            },
            ShareGptMessage {
                from: "observation",
                value: "Error: validation failed - expected integer got string",
            },
        ];
        let batch = SignalAnalyzer::default().analyze_sharegpt(&convo);
        let incremental = step_through(&convo);
        assert_same_signal_counts(&batch, &incremental, "failure_and_loops");
        assert!(batch.execution.failure.count > 0);
        assert!(batch.execution.loops.count > 0);
    }

    #[test]
    fn incremental_matches_batch_for_exhaustion() {
        let convo = vec![
            ShareGptMessage {
                from: "human",
                value: "fetch the latest price",
            },
            ShareGptMessage {
                from: "function_call",
                value: r#"{"name":"get_price","arguments":{}}"#,
            },
            ShareGptMessage {
                from: "observation",
                value: "503 Service Unavailable: try again later",
            },
        ];
        let batch = SignalAnalyzer::default().analyze_sharegpt(&convo);
        let incremental = step_through(&convo);
        assert_same_signal_counts(&batch, &incremental, "exhaustion");
        assert!(batch.environment.exhaustion.count > 0);
    }

    #[test]
    fn incremental_matches_batch_for_clean_conversation() {
        let convo = vec![
            ShareGptMessage {
                from: "human",
                value: "Hello, can you help me with a question?",
            },
            ShareGptMessage {
                from: "gpt",
                value: "Of course, what's your question?",
            },
            ShareGptMessage {
                from: "human",
                value: "How does X work?",
            },
            ShareGptMessage {
                from: "gpt",
                value: "X works by ...",
            },
        ];
        let batch = SignalAnalyzer::default().analyze_sharegpt(&convo);
        let incremental = step_through(&convo);
        assert_same_signal_counts(&batch, &incremental, "clean_conversation");
    }

    #[test]
    fn get_message_reports_returns_one_entry_per_message() {
        let convo = vec![
            ShareGptMessage {
                from: "human",
                value: "Hello",
            },
            ShareGptMessage {
                from: "gpt",
                value: "Hi! How can I help?",
            },
            ShareGptMessage {
                from: "human",
                value: "That's perfect, thank you!",
            },
        ];
        let reports = SignalAnalyzer::default().get_message_reports(&convo);
        assert_eq!(reports.len(), convo.len());
        assert_eq!(reports[0].role, "human");
        assert_eq!(reports[2].label, "positive");
    }

    #[test]
    fn analyze_step_single_message_matches_batch() {
        let convo = vec![ShareGptMessage {
            from: "human",
            value: "Hello there",
        }];
        let batch = SignalAnalyzer::default().analyze_sharegpt(&convo);
        let (_, incremental) = SignalAnalyzer::default().analyze_step(&convo, None);
        assert_same_signal_counts(&batch, &incremental, "single_message");
    }
}
