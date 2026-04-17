//! Signal event atoms — the foundational representation of detections.
//!
//! A `SignalEvent` is a single detected indicator in a conversation (one
//! frustration indicator, one repetition instance, etc.). Aggregate metrics
//! in `SignalReport` are unchanged; events are emitted alongside them so
//! downstream consumers can drill from an aggregate counter to the specific
//! message or tool call that triggered it.
//!
//! Phase 1 populates only the `Interaction` layer variants; the `Execution`
//! and `Environment` variants are declared here so callers can match
//! exhaustively, but they are not emitted until Phase 2.

use chrono::{DateTime, Utc};
use opentelemetry::KeyValue;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use super::analyzer::{EscalationType, FrustrationType, PositiveType, RepetitionType};

/// OpenTelemetry attribute keys used on `SignalEvent` span events.
///
/// These are intentionally flat strings rather than constants pulled from
/// `tracing::constants` because they are only meaningful on span events, not
/// on the outer span attributes. Keeping them local to `event.rs` avoids
/// coupling the constants module to event-specific schema.
mod otel_keys {
    pub const EVENT_ID: &str = "signal.event_id";
    pub const TYPE: &str = "signal.type";
    pub const SUBTYPE: &str = "signal.subtype";
    pub const SOURCE_MESSAGE_IDX: &str = "signal.source_message_idx";
    pub const EVIDENCE_SNIPPET: &str = "signal.evidence.snippet";
    pub const EVIDENCE_INDICATOR_TYPE: &str = "signal.evidence.indicator_type";
    pub const EVIDENCE_ESCALATION_TYPE: &str = "signal.evidence.escalation_type";
    pub const EVIDENCE_REPETITION_TYPE: &str = "signal.evidence.repetition_type";
    pub const EVIDENCE_SIMILARITY: &str = "signal.evidence.similarity";
    pub const EVIDENCE_OTHER_MESSAGE_IDX: &str = "signal.evidence.other_message_idx";
    pub const EVIDENCE_SIMILAR_TO_PREV_USER_TURN: &str =
        "signal.evidence.similar_to_prev_user_turn";
}

/// Top-level signal layer from the Signals paper taxonomy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SignalType {
    Interaction,
    Execution,
    Environment,
}

impl SignalType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Interaction => "interaction",
            Self::Execution => "execution",
            Self::Environment => "environment",
        }
    }
}

impl std::fmt::Display for SignalType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Specific signal subtype. Each subtype belongs to exactly one `SignalType`
/// layer (see [`SignalSubtype::layer`]).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SignalSubtype {
    // Interaction layer (Phase 1).
    Repair,
    Frustration,
    Repetition,
    PositiveFeedback,
    Escalation,
    // Execution layer (declared for Phase 2; not emitted yet).
    ToolFailure,
    ExecutionLoop,
    // Environment layer (declared for Phase 2; not emitted yet).
    Exhaustion,
}

impl SignalSubtype {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Repair => "repair",
            Self::Frustration => "frustration",
            Self::Repetition => "repetition",
            Self::PositiveFeedback => "positive_feedback",
            Self::Escalation => "escalation",
            Self::ToolFailure => "tool_failure",
            Self::ExecutionLoop => "execution_loop",
            Self::Exhaustion => "exhaustion",
        }
    }

    /// Returns the signal layer this subtype belongs to.
    pub fn layer(&self) -> SignalType {
        match self {
            Self::Repair
            | Self::Frustration
            | Self::Repetition
            | Self::PositiveFeedback
            | Self::Escalation => SignalType::Interaction,
            Self::ToolFailure | Self::ExecutionLoop => SignalType::Execution,
            Self::Exhaustion => SignalType::Environment,
        }
    }
}

impl std::fmt::Display for SignalSubtype {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Structured evidence payload. Variants mirror [`SignalSubtype`] — a
/// `Frustration` subtype always carries `Frustration` evidence, and so on.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SignalEvidence {
    Repair {
        snippet: String,
        /// True when the repair was detected by semantic rephrase of the
        /// previous user turn (rather than a lexical repair pattern).
        similar_to_prev_user_turn: bool,
    },
    Frustration {
        indicator_type: FrustrationType,
        snippet: String,
    },
    Repetition {
        /// Absolute index of the other message in the detected pair.
        other_message_idx: usize,
        similarity: f64,
        repetition_type: RepetitionType,
    },
    PositiveFeedback {
        indicator_type: PositiveType,
        snippet: String,
    },
    Escalation {
        escalation_type: EscalationType,
        snippet: String,
    },
}

/// Single detected signal atom.
///
/// `source_message_idx` is an absolute index into the input `Vec<Message>`
/// the analyzer was called with — stable across the analyzer's internal
/// truncation window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalEvent {
    pub event_id: Ulid,
    pub signal_type: SignalType,
    pub signal_subtype: SignalSubtype,
    pub source_message_idx: usize,
    pub timestamp: DateTime<Utc>,
    pub evidence: SignalEvidence,
}

impl SignalEvent {
    pub fn new(
        signal_subtype: SignalSubtype,
        source_message_idx: usize,
        evidence: SignalEvidence,
    ) -> Self {
        Self {
            event_id: Ulid::new(),
            signal_type: signal_subtype.layer(),
            signal_subtype,
            source_message_idx,
            timestamp: Utc::now(),
            evidence,
        }
    }

    /// Canonical span-event name for this signal.
    ///
    /// Shape: `signal.{type}.{subtype}` (e.g. `signal.interaction.frustration`).
    /// External consumers can key off these stable names.
    pub fn otel_event_name(&self) -> String {
        format!("signal.{}.{}", self.signal_type, self.signal_subtype)
    }

    /// Flatten this event into OpenTelemetry key-value attributes suitable
    /// for `span.add_event(name, attrs)`.
    pub fn to_otel_attributes(&self) -> Vec<KeyValue> {
        let mut attrs = vec![
            KeyValue::new(otel_keys::EVENT_ID, self.event_id.to_string()),
            KeyValue::new(otel_keys::TYPE, self.signal_type.as_str()),
            KeyValue::new(otel_keys::SUBTYPE, self.signal_subtype.as_str()),
            KeyValue::new(
                otel_keys::SOURCE_MESSAGE_IDX,
                self.source_message_idx as i64,
            ),
        ];

        match &self.evidence {
            SignalEvidence::Repair {
                snippet,
                similar_to_prev_user_turn,
            } => {
                attrs.push(KeyValue::new(otel_keys::EVIDENCE_SNIPPET, snippet.clone()));
                attrs.push(KeyValue::new(
                    otel_keys::EVIDENCE_SIMILAR_TO_PREV_USER_TURN,
                    *similar_to_prev_user_turn,
                ));
            }
            SignalEvidence::Frustration {
                indicator_type,
                snippet,
            } => {
                attrs.push(KeyValue::new(
                    otel_keys::EVIDENCE_INDICATOR_TYPE,
                    format!("{:?}", indicator_type),
                ));
                attrs.push(KeyValue::new(otel_keys::EVIDENCE_SNIPPET, snippet.clone()));
            }
            SignalEvidence::Repetition {
                other_message_idx,
                similarity,
                repetition_type,
            } => {
                attrs.push(KeyValue::new(
                    otel_keys::EVIDENCE_OTHER_MESSAGE_IDX,
                    *other_message_idx as i64,
                ));
                attrs.push(KeyValue::new(
                    otel_keys::EVIDENCE_SIMILARITY,
                    format!("{:.3}", similarity),
                ));
                attrs.push(KeyValue::new(
                    otel_keys::EVIDENCE_REPETITION_TYPE,
                    format!("{:?}", repetition_type),
                ));
            }
            SignalEvidence::PositiveFeedback {
                indicator_type,
                snippet,
            } => {
                attrs.push(KeyValue::new(
                    otel_keys::EVIDENCE_INDICATOR_TYPE,
                    format!("{:?}", indicator_type),
                ));
                attrs.push(KeyValue::new(otel_keys::EVIDENCE_SNIPPET, snippet.clone()));
            }
            SignalEvidence::Escalation {
                escalation_type,
                snippet,
            } => {
                attrs.push(KeyValue::new(
                    otel_keys::EVIDENCE_ESCALATION_TYPE,
                    format!("{:?}", escalation_type),
                ));
                attrs.push(KeyValue::new(otel_keys::EVIDENCE_SNIPPET, snippet.clone()));
            }
        }

        attrs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtype_layer_mapping() {
        assert_eq!(SignalSubtype::Repair.layer(), SignalType::Interaction);
        assert_eq!(SignalSubtype::Frustration.layer(), SignalType::Interaction);
        assert_eq!(SignalSubtype::Repetition.layer(), SignalType::Interaction);
        assert_eq!(
            SignalSubtype::PositiveFeedback.layer(),
            SignalType::Interaction
        );
        assert_eq!(SignalSubtype::Escalation.layer(), SignalType::Interaction);
        assert_eq!(SignalSubtype::ToolFailure.layer(), SignalType::Execution);
        assert_eq!(SignalSubtype::ExecutionLoop.layer(), SignalType::Execution);
        assert_eq!(SignalSubtype::Exhaustion.layer(), SignalType::Environment);
    }

    #[test]
    fn new_event_sets_layer_from_subtype() {
        let event = SignalEvent::new(
            SignalSubtype::Frustration,
            7,
            SignalEvidence::Frustration {
                indicator_type: FrustrationType::AllCaps,
                snippet: "WHY ISN'T THIS WORKING".to_string(),
            },
        );
        assert_eq!(event.signal_type, SignalType::Interaction);
        assert_eq!(event.signal_subtype, SignalSubtype::Frustration);
        assert_eq!(event.source_message_idx, 7);
    }

    #[test]
    fn otel_event_name_shape() {
        let event = SignalEvent::new(
            SignalSubtype::Frustration,
            7,
            SignalEvidence::Frustration {
                indicator_type: FrustrationType::AllCaps,
                snippet: "WHY".to_string(),
            },
        );
        assert_eq!(event.otel_event_name(), "signal.interaction.frustration");

        let tool_failure_event = SignalEvent {
            event_id: Ulid::new(),
            signal_type: SignalType::Execution,
            signal_subtype: SignalSubtype::ToolFailure,
            source_message_idx: 3,
            timestamp: Utc::now(),
            // Evidence variant is not yet defined for ToolFailure in Phase 1;
            // use Frustration as a stand-in purely to exercise name formatting.
            evidence: SignalEvidence::Frustration {
                indicator_type: FrustrationType::AllCaps,
                snippet: String::new(),
            },
        };
        assert_eq!(
            tool_failure_event.otel_event_name(),
            "signal.execution.tool_failure"
        );
    }

    #[test]
    fn to_otel_attributes_includes_base_keys() {
        let event = SignalEvent::new(
            SignalSubtype::Frustration,
            7,
            SignalEvidence::Frustration {
                indicator_type: FrustrationType::AllCaps,
                snippet: "WHY".to_string(),
            },
        );
        let attrs = event.to_otel_attributes();
        let keys: std::collections::HashSet<String> =
            attrs.iter().map(|kv| kv.key.as_str().to_string()).collect();
        for required in [
            "signal.event_id",
            "signal.type",
            "signal.subtype",
            "signal.source_message_idx",
        ] {
            assert!(
                keys.contains(required),
                "missing required attribute {}",
                required
            );
        }
    }

    #[test]
    fn to_otel_attributes_includes_evidence_fields_per_variant() {
        let repair = SignalEvent::new(
            SignalSubtype::Repair,
            2,
            SignalEvidence::Repair {
                snippet: "that's not what i meant".to_string(),
                similar_to_prev_user_turn: false,
            },
        );
        let repetition = SignalEvent::new(
            SignalSubtype::Repetition,
            4,
            SignalEvidence::Repetition {
                other_message_idx: 2,
                similarity: 0.91,
                repetition_type: RepetitionType::Exact,
            },
        );
        let escalation = SignalEvent::new(
            SignalSubtype::Escalation,
            5,
            SignalEvidence::Escalation {
                escalation_type: EscalationType::HumanAgent,
                snippet: "speak to a human".to_string(),
            },
        );

        let keyset = |e: &SignalEvent| -> std::collections::HashSet<String> {
            e.to_otel_attributes()
                .iter()
                .map(|kv| kv.key.as_str().to_string())
                .collect()
        };

        assert!(keyset(&repair).contains("signal.evidence.snippet"));
        assert!(keyset(&repair).contains("signal.evidence.similar_to_prev_user_turn"));

        let rep_keys = keyset(&repetition);
        assert!(rep_keys.contains("signal.evidence.other_message_idx"));
        assert!(rep_keys.contains("signal.evidence.similarity"));
        assert!(rep_keys.contains("signal.evidence.repetition_type"));

        let esc_keys = keyset(&escalation);
        assert!(esc_keys.contains("signal.evidence.escalation_type"));
        assert!(esc_keys.contains("signal.evidence.snippet"));
    }

    #[test]
    fn event_serialization_round_trip() {
        let event = SignalEvent::new(
            SignalSubtype::Repetition,
            12,
            SignalEvidence::Repetition {
                other_message_idx: 8,
                similarity: 0.91,
                repetition_type: RepetitionType::Exact,
            },
        );
        let serialized = serde_json::to_string(&event).expect("serialize");
        let deserialized: SignalEvent = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(deserialized.event_id, event.event_id);
        assert_eq!(deserialized.signal_subtype, event.signal_subtype);
        assert_eq!(deserialized.source_message_idx, event.source_message_idx);
    }
}
