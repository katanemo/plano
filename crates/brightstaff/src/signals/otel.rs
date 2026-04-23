//! Helpers for emitting `SignalReport` data to OpenTelemetry spans.
//!
//! Two sets of attributes are emitted:
//!
//! - **Legacy** keys under `signals.*` (e.g. `signals.frustration.count`),
//!   computed from the new layered counts. Preserved for one release for
//!   backward compatibility with existing dashboards.
//! - **New** layered keys (e.g. `signals.interaction.misalignment.count`),
//!   one set of `count`/`severity` attributes per category, plus per-instance
//!   span events named `signal.<dotted_signal_type>`.

use opentelemetry::trace::SpanRef;
use opentelemetry::KeyValue;

use crate::signals::schemas::{SignalGroup, SignalReport, SignalType};

/// Emit both legacy and layered OTel attributes/events for a `SignalReport`.
///
/// Returns `true` if any "concerning" signal was found, mirroring the previous
/// behavior used to flag the span operation name.
pub fn emit_signals_to_span(span: &SpanRef<'_>, report: &SignalReport) -> bool {
    emit_overall(span, report);
    emit_layered_attributes(span, report);
    emit_legacy_attributes(span, report);
    emit_signal_events(span, report);

    is_concerning(report)
}

fn emit_overall(span: &SpanRef<'_>, report: &SignalReport) {
    span.set_attribute(KeyValue::new(
        "signals.quality",
        report.overall_quality.as_str().to_string(),
    ));
    span.set_attribute(KeyValue::new(
        "signals.quality_score",
        report.quality_score as f64,
    ));
    span.set_attribute(KeyValue::new(
        "signals.turn_count",
        report.turn_metrics.total_turns as i64,
    ));
    span.set_attribute(KeyValue::new(
        "signals.efficiency_score",
        report.turn_metrics.efficiency_score as f64,
    ));
}

fn emit_group(span: &SpanRef<'_>, prefix: &str, group: &SignalGroup) {
    if group.count == 0 {
        return;
    }
    span.set_attribute(KeyValue::new(
        format!("{}.count", prefix),
        group.count as i64,
    ));
    span.set_attribute(KeyValue::new(
        format!("{}.severity", prefix),
        group.severity as i64,
    ));
}

fn emit_layered_attributes(span: &SpanRef<'_>, report: &SignalReport) {
    emit_group(
        span,
        "signals.interaction.misalignment",
        &report.interaction.misalignment,
    );
    emit_group(
        span,
        "signals.interaction.stagnation",
        &report.interaction.stagnation,
    );
    emit_group(
        span,
        "signals.interaction.disengagement",
        &report.interaction.disengagement,
    );
    emit_group(
        span,
        "signals.interaction.satisfaction",
        &report.interaction.satisfaction,
    );
    emit_group(span, "signals.execution.failure", &report.execution.failure);
    emit_group(span, "signals.execution.loops", &report.execution.loops);
    emit_group(
        span,
        "signals.environment.exhaustion",
        &report.environment.exhaustion,
    );
}

fn count_of(report: &SignalReport, t: SignalType) -> usize {
    report.iter_signals().filter(|s| s.signal_type == t).count()
}

/// Emit the legacy attribute keys consumed by existing dashboards. These are
/// derived from the new `SignalReport` so no detector contract is broken.
fn emit_legacy_attributes(span: &SpanRef<'_>, report: &SignalReport) {
    use crate::tracing::signals as legacy;

    // signals.follow_up.repair.{count,ratio} - misalignment proxies repairs.
    let repair_count = report.interaction.misalignment.count;
    let user_turns = report.turn_metrics.user_turns.max(1) as f32;
    if repair_count > 0 {
        span.set_attribute(KeyValue::new(legacy::REPAIR_COUNT, repair_count as i64));
        let ratio = repair_count as f32 / user_turns;
        span.set_attribute(KeyValue::new(legacy::REPAIR_RATIO, format!("{:.3}", ratio)));
    }

    // signals.frustration.{count,severity} - disengagement.negative_stance is
    // the closest legacy analog of "frustration".
    let frustration_count = count_of(report, SignalType::DisengagementNegativeStance);
    if frustration_count > 0 {
        span.set_attribute(KeyValue::new(
            legacy::FRUSTRATION_COUNT,
            frustration_count as i64,
        ));
        let severity = match frustration_count {
            0 => 0,
            1..=2 => 1,
            3..=4 => 2,
            _ => 3,
        };
        span.set_attribute(KeyValue::new(legacy::FRUSTRATION_SEVERITY, severity as i64));
    }

    // signals.repetition.count - stagnation (repetition + dragging).
    if report.interaction.stagnation.count > 0 {
        span.set_attribute(KeyValue::new(
            legacy::REPETITION_COUNT,
            report.interaction.stagnation.count as i64,
        ));
    }

    // signals.escalation.requested - any escalation/quit signal.
    let escalated = report.interaction.disengagement.signals.iter().any(|s| {
        matches!(
            s.signal_type,
            SignalType::DisengagementEscalation | SignalType::DisengagementQuit
        )
    });
    if escalated {
        span.set_attribute(KeyValue::new(legacy::ESCALATION_REQUESTED, true));
    }

    // signals.positive_feedback.count - satisfaction signals.
    if report.interaction.satisfaction.count > 0 {
        span.set_attribute(KeyValue::new(
            legacy::POSITIVE_FEEDBACK_COUNT,
            report.interaction.satisfaction.count as i64,
        ));
    }
}

fn emit_signal_events(span: &SpanRef<'_>, report: &SignalReport) {
    for sig in report.iter_signals() {
        let event_name = format!("signal.{}", sig.signal_type.as_str());
        let mut attrs: Vec<KeyValue> = vec![
            KeyValue::new("signal.type", sig.signal_type.as_str().to_string()),
            KeyValue::new("signal.message_index", sig.message_index as i64),
            KeyValue::new("signal.confidence", sig.confidence as f64),
        ];
        if !sig.snippet.is_empty() {
            attrs.push(KeyValue::new("signal.snippet", sig.snippet.clone()));
        }
        if !sig.metadata.is_null() {
            attrs.push(KeyValue::new("signal.metadata", sig.metadata.to_string()));
        }
        span.add_event(event_name, attrs);
    }
}

fn is_concerning(report: &SignalReport) -> bool {
    use crate::signals::schemas::InteractionQuality;
    if matches!(
        report.overall_quality,
        InteractionQuality::Poor | InteractionQuality::Severe
    ) {
        return true;
    }
    if report.interaction.disengagement.count > 0 {
        return true;
    }
    if report.interaction.stagnation.count > 2 {
        return true;
    }
    if report.execution.failure.count > 0 || report.execution.loops.count > 0 {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signals::schemas::{
        EnvironmentSignals, ExecutionSignals, InteractionQuality, InteractionSignals, SignalGroup,
        SignalInstance, SignalReport, SignalType, TurnMetrics,
    };

    fn report_with_escalation() -> SignalReport {
        let mut diseng = SignalGroup::new("disengagement");
        diseng.add_signal(SignalInstance::new(
            SignalType::DisengagementEscalation,
            3,
            "get me a human",
        ));
        SignalReport {
            interaction: InteractionSignals {
                disengagement: diseng,
                ..InteractionSignals::default()
            },
            execution: ExecutionSignals::default(),
            environment: EnvironmentSignals::default(),
            overall_quality: InteractionQuality::Severe,
            quality_score: 0.0,
            turn_metrics: TurnMetrics {
                total_turns: 3,
                user_turns: 2,
                assistant_turns: 1,
                is_dragging: false,
                efficiency_score: 1.0,
            },
            summary: String::new(),
        }
    }

    #[test]
    fn is_concerning_flags_disengagement() {
        let r = report_with_escalation();
        assert!(is_concerning(&r));
    }

    #[test]
    fn count_of_returns_per_type_count() {
        let r = report_with_escalation();
        assert_eq!(count_of(&r, SignalType::DisengagementEscalation), 1);
        assert_eq!(count_of(&r, SignalType::DisengagementNegativeStance), 0);
    }
}
