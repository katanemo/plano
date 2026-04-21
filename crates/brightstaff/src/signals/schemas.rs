//! Data shapes for the signal analyzer.
//!
//! Mirrors `signals/schemas.py` from the reference implementation. Where the
//! Python library exposes a `Dict[str, SignalGroup]` partitioned by category,
//! the Rust port uses strongly-typed sub-structs (`InteractionSignals`,
//! `ExecutionSignals`, `EnvironmentSignals`) for the same partitioning.

use serde::{Deserialize, Serialize};

/// Hierarchical signal type. The 20 leaf variants mirror the paper taxonomy
/// and the Python reference's `SignalType` string enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SignalType {
    // Interaction > Misalignment
    MisalignmentCorrection,
    MisalignmentRephrase,
    MisalignmentClarification,

    // Interaction > Stagnation
    StagnationDragging,
    StagnationRepetition,

    // Interaction > Disengagement
    DisengagementEscalation,
    DisengagementQuit,
    DisengagementNegativeStance,

    // Interaction > Satisfaction
    SatisfactionGratitude,
    SatisfactionConfirmation,
    SatisfactionSuccess,

    // Execution > Failure
    ExecutionFailureInvalidArgs,
    ExecutionFailureBadQuery,
    ExecutionFailureToolNotFound,
    ExecutionFailureAuthMisuse,
    ExecutionFailureStateError,

    // Execution > Loops
    ExecutionLoopsRetry,
    ExecutionLoopsParameterDrift,
    ExecutionLoopsOscillation,

    // Environment > Exhaustion
    EnvironmentExhaustionApiError,
    EnvironmentExhaustionTimeout,
    EnvironmentExhaustionRateLimit,
    EnvironmentExhaustionNetwork,
    EnvironmentExhaustionMalformed,
    EnvironmentExhaustionContextOverflow,
}

impl SignalType {
    /// Dotted hierarchical string identifier, e.g.
    /// `"interaction.misalignment.correction"`. Matches the Python reference's
    /// `SignalType` enum *value* strings byte-for-byte.
    pub fn as_str(&self) -> &'static str {
        match self {
            SignalType::MisalignmentCorrection => "interaction.misalignment.correction",
            SignalType::MisalignmentRephrase => "interaction.misalignment.rephrase",
            SignalType::MisalignmentClarification => "interaction.misalignment.clarification",
            SignalType::StagnationDragging => "interaction.stagnation.dragging",
            SignalType::StagnationRepetition => "interaction.stagnation.repetition",
            SignalType::DisengagementEscalation => "interaction.disengagement.escalation",
            SignalType::DisengagementQuit => "interaction.disengagement.quit",
            SignalType::DisengagementNegativeStance => "interaction.disengagement.negative_stance",
            SignalType::SatisfactionGratitude => "interaction.satisfaction.gratitude",
            SignalType::SatisfactionConfirmation => "interaction.satisfaction.confirmation",
            SignalType::SatisfactionSuccess => "interaction.satisfaction.success",
            SignalType::ExecutionFailureInvalidArgs => "execution.failure.invalid_args",
            SignalType::ExecutionFailureBadQuery => "execution.failure.bad_query",
            SignalType::ExecutionFailureToolNotFound => "execution.failure.tool_not_found",
            SignalType::ExecutionFailureAuthMisuse => "execution.failure.auth_misuse",
            SignalType::ExecutionFailureStateError => "execution.failure.state_error",
            SignalType::ExecutionLoopsRetry => "execution.loops.retry",
            SignalType::ExecutionLoopsParameterDrift => "execution.loops.parameter_drift",
            SignalType::ExecutionLoopsOscillation => "execution.loops.oscillation",
            SignalType::EnvironmentExhaustionApiError => "environment.exhaustion.api_error",
            SignalType::EnvironmentExhaustionTimeout => "environment.exhaustion.timeout",
            SignalType::EnvironmentExhaustionRateLimit => "environment.exhaustion.rate_limit",
            SignalType::EnvironmentExhaustionNetwork => "environment.exhaustion.network",
            SignalType::EnvironmentExhaustionMalformed => {
                "environment.exhaustion.malformed_response"
            }
            SignalType::EnvironmentExhaustionContextOverflow => {
                "environment.exhaustion.context_overflow"
            }
        }
    }

    pub fn layer(&self) -> SignalLayer {
        match self {
            SignalType::MisalignmentCorrection
            | SignalType::MisalignmentRephrase
            | SignalType::MisalignmentClarification
            | SignalType::StagnationDragging
            | SignalType::StagnationRepetition
            | SignalType::DisengagementEscalation
            | SignalType::DisengagementQuit
            | SignalType::DisengagementNegativeStance
            | SignalType::SatisfactionGratitude
            | SignalType::SatisfactionConfirmation
            | SignalType::SatisfactionSuccess => SignalLayer::Interaction,
            SignalType::ExecutionFailureInvalidArgs
            | SignalType::ExecutionFailureBadQuery
            | SignalType::ExecutionFailureToolNotFound
            | SignalType::ExecutionFailureAuthMisuse
            | SignalType::ExecutionFailureStateError
            | SignalType::ExecutionLoopsRetry
            | SignalType::ExecutionLoopsParameterDrift
            | SignalType::ExecutionLoopsOscillation => SignalLayer::Execution,
            SignalType::EnvironmentExhaustionApiError
            | SignalType::EnvironmentExhaustionTimeout
            | SignalType::EnvironmentExhaustionRateLimit
            | SignalType::EnvironmentExhaustionNetwork
            | SignalType::EnvironmentExhaustionMalformed
            | SignalType::EnvironmentExhaustionContextOverflow => SignalLayer::Environment,
        }
    }

    /// Category name within the layer (e.g. `"misalignment"`, `"failure"`).
    pub fn category(&self) -> &'static str {
        // Strip the layer prefix and take everything before the next dot.
        let s = self.as_str();
        let after_layer = s.split_once('.').map(|(_, rest)| rest).unwrap_or(s);
        after_layer
            .split_once('.')
            .map(|(c, _)| c)
            .unwrap_or(after_layer)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SignalLayer {
    Interaction,
    Execution,
    Environment,
}

impl SignalLayer {
    pub fn as_str(&self) -> &'static str {
        match self {
            SignalLayer::Interaction => "interaction",
            SignalLayer::Execution => "execution",
            SignalLayer::Environment => "environment",
        }
    }
}

/// Overall quality assessment for an agent interaction session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InteractionQuality {
    Excellent,
    Good,
    Neutral,
    Poor,
    Severe,
}

impl InteractionQuality {
    pub fn as_str(&self) -> &'static str {
        match self {
            InteractionQuality::Excellent => "excellent",
            InteractionQuality::Good => "good",
            InteractionQuality::Neutral => "neutral",
            InteractionQuality::Poor => "poor",
            InteractionQuality::Severe => "severe",
        }
    }
}

/// A single detected signal instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalInstance {
    pub signal_type: SignalType,
    /// Absolute index into the original conversation `Vec<Message>`.
    pub message_index: usize,
    pub snippet: String,
    pub confidence: f32,
    /// Free-form metadata payload mirroring the Python `Dict[str, Any]`.
    /// Stored as a JSON object so we can faithfully reproduce the reference's
    /// flexible per-detector metadata.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl SignalInstance {
    pub fn new(signal_type: SignalType, message_index: usize, snippet: impl Into<String>) -> Self {
        Self {
            signal_type,
            message_index,
            snippet: snippet.into(),
            confidence: 1.0,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        }
    }

    pub fn with_confidence(mut self, c: f32) -> Self {
        self.confidence = c;
        self
    }

    pub fn with_metadata(mut self, m: serde_json::Value) -> Self {
        self.metadata = m;
        self
    }
}

/// Aggregated signals for a specific category.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalGroup {
    pub category: String,
    pub count: usize,
    pub signals: Vec<SignalInstance>,
    /// Severity level (0-3: none, mild, moderate, severe).
    pub severity: u8,
}

impl SignalGroup {
    pub fn new(category: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            count: 0,
            signals: Vec::new(),
            severity: 0,
        }
    }

    pub fn add_signal(&mut self, signal: SignalInstance) {
        self.signals.push(signal);
        self.count = self.signals.len();
        self.update_severity();
    }

    fn update_severity(&mut self) {
        self.severity = match self.count {
            0 => 0,
            1..=2 => 1,
            3..=4 => 2,
            _ => 3,
        };
    }
}

/// Turn count and efficiency metrics, used by stagnation.dragging.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TurnMetrics {
    pub total_turns: usize,
    pub user_turns: usize,
    pub assistant_turns: usize,
    pub is_dragging: bool,
    pub efficiency_score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionSignals {
    pub misalignment: SignalGroup,
    pub stagnation: SignalGroup,
    pub disengagement: SignalGroup,
    pub satisfaction: SignalGroup,
}

impl Default for InteractionSignals {
    fn default() -> Self {
        Self {
            misalignment: SignalGroup::new("misalignment"),
            stagnation: SignalGroup::new("stagnation"),
            disengagement: SignalGroup::new("disengagement"),
            satisfaction: SignalGroup::new("satisfaction"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionSignals {
    pub failure: SignalGroup,
    pub loops: SignalGroup,
}

impl Default for ExecutionSignals {
    fn default() -> Self {
        Self {
            failure: SignalGroup::new("failure"),
            loops: SignalGroup::new("loops"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentSignals {
    pub exhaustion: SignalGroup,
}

impl Default for EnvironmentSignals {
    fn default() -> Self {
        Self {
            exhaustion: SignalGroup::new("exhaustion"),
        }
    }
}

/// Complete signal analysis report for a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalReport {
    pub interaction: InteractionSignals,
    pub execution: ExecutionSignals,
    pub environment: EnvironmentSignals,
    pub overall_quality: InteractionQuality,
    pub quality_score: f32,
    pub turn_metrics: TurnMetrics,
    pub summary: String,
}

impl Default for SignalReport {
    fn default() -> Self {
        Self {
            interaction: InteractionSignals::default(),
            execution: ExecutionSignals::default(),
            environment: EnvironmentSignals::default(),
            overall_quality: InteractionQuality::Neutral,
            quality_score: 50.0,
            turn_metrics: TurnMetrics::default(),
            summary: String::new(),
        }
    }
}

impl SignalReport {
    /// Iterate over every `SignalInstance` across all layers and groups.
    pub fn iter_signals(&self) -> impl Iterator<Item = &SignalInstance> {
        self.interaction
            .misalignment
            .signals
            .iter()
            .chain(self.interaction.stagnation.signals.iter())
            .chain(self.interaction.disengagement.signals.iter())
            .chain(self.interaction.satisfaction.signals.iter())
            .chain(self.execution.failure.signals.iter())
            .chain(self.execution.loops.signals.iter())
            .chain(self.environment.exhaustion.signals.iter())
    }

    pub fn has_signal_type(&self, t: SignalType) -> bool {
        self.iter_signals().any(|s| s.signal_type == t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_type_strings_match_paper_taxonomy() {
        assert_eq!(
            SignalType::MisalignmentCorrection.as_str(),
            "interaction.misalignment.correction"
        );
        assert_eq!(
            SignalType::ExecutionFailureInvalidArgs.as_str(),
            "execution.failure.invalid_args"
        );
        assert_eq!(
            SignalType::EnvironmentExhaustionMalformed.as_str(),
            "environment.exhaustion.malformed_response"
        );
    }

    #[test]
    fn signal_type_layer_and_category() {
        assert_eq!(
            SignalType::MisalignmentRephrase.layer(),
            SignalLayer::Interaction
        );
        assert_eq!(SignalType::MisalignmentRephrase.category(), "misalignment");
        assert_eq!(
            SignalType::ExecutionLoopsRetry.layer(),
            SignalLayer::Execution
        );
        assert_eq!(SignalType::ExecutionLoopsRetry.category(), "loops");
        assert_eq!(
            SignalType::EnvironmentExhaustionTimeout.layer(),
            SignalLayer::Environment
        );
        assert_eq!(
            SignalType::EnvironmentExhaustionTimeout.category(),
            "exhaustion"
        );
    }

    #[test]
    fn signal_group_severity_buckets_match_python() {
        let mut g = SignalGroup::new("misalignment");
        assert_eq!(g.severity, 0);
        for n in 1..=2 {
            g.add_signal(SignalInstance::new(
                SignalType::MisalignmentCorrection,
                n,
                "x",
            ));
        }
        assert_eq!(g.severity, 1);
        for n in 3..=4 {
            g.add_signal(SignalInstance::new(
                SignalType::MisalignmentCorrection,
                n,
                "x",
            ));
        }
        assert_eq!(g.severity, 2);
        for n in 5..=6 {
            g.add_signal(SignalInstance::new(
                SignalType::MisalignmentCorrection,
                n,
                "x",
            ));
        }
        assert_eq!(g.severity, 3);
    }
}
