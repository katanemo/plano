//! Execution loops detector. Direct port of `signals/execution/loops.py`.

use std::collections::{HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::signals::analyzer::ShareGptMessage;
use crate::signals::schemas::{SignalGroup, SignalInstance, SignalType};

pub const RETRY_THRESHOLD: usize = 3;
pub const PARAMETER_DRIFT_THRESHOLD: usize = 3;
pub const OSCILLATION_CYCLES_THRESHOLD: usize = 3;
/// Longest oscillation pattern length considered, matching the batch
/// detector's `2..=min(5, ...)` range.
const MAX_OSC_PATTERN_LEN: usize = 5;

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub index: usize,
    pub name: String,
    /// Canonical JSON string of arguments (sorted keys when parseable).
    pub args: String,
    pub args_dict: Option<serde_json::Map<String, serde_json::Value>>,
}

impl ToolCall {
    pub fn args_equal(&self, other: &ToolCall) -> bool {
        match (&self.args_dict, &other.args_dict) {
            (Some(a), Some(b)) => a == b,
            _ => self.args == other.args,
        }
    }
}

fn parse_tool_call(index: usize, msg: &ShareGptMessage<'_>) -> Option<ToolCall> {
    if msg.from != "function_call" {
        return None;
    }
    let value = msg.value;

    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(value) {
        if let Some(obj) = parsed.as_object() {
            let name = obj
                .get("name")
                .or_else(|| obj.get("function"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let raw_args = obj.get("arguments").or_else(|| obj.get("args"));
            let (args_str, args_dict) = match raw_args {
                Some(serde_json::Value::Object(o)) => {
                    let mut keys: Vec<&String> = o.keys().collect();
                    keys.sort();
                    let mut canon = serde_json::Map::new();
                    for k in keys {
                        canon.insert(k.clone(), o[k].clone());
                    }
                    (
                        serde_json::to_string(&serde_json::Value::Object(canon.clone()))
                            .unwrap_or_default(),
                        Some(canon),
                    )
                }
                Some(other) => (
                    other
                        .as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| serde_json::to_string(other).unwrap_or_default()),
                    None,
                ),
                None => (String::new(), None),
            };
            return Some(ToolCall {
                index,
                name,
                args: args_str,
                args_dict,
            });
        }
    }

    if let Some(paren) = value.find('(') {
        if paren > 0 {
            let name = value[..paren].trim().to_string();
            let args_part = &value[paren..];
            if args_part.starts_with('(') && args_part.ends_with(')') {
                let inner = args_part[1..args_part.len() - 1].trim();
                if let Ok(serde_json::Value::Object(o)) =
                    serde_json::from_str::<serde_json::Value>(inner)
                {
                    let mut keys: Vec<&String> = o.keys().collect();
                    keys.sort();
                    let mut canon = serde_json::Map::new();
                    for k in keys {
                        canon.insert(k.clone(), o[k].clone());
                    }
                    return Some(ToolCall {
                        index,
                        name,
                        args: serde_json::to_string(&serde_json::Value::Object(canon.clone()))
                            .unwrap_or_default(),
                        args_dict: Some(canon),
                    });
                }
                return Some(ToolCall {
                    index,
                    name,
                    args: inner.to_string(),
                    args_dict: None,
                });
            }
            return Some(ToolCall {
                index,
                name,
                args: args_part.to_string(),
                args_dict: None,
            });
        }
    }

    Some(ToolCall {
        index,
        name: value.trim().to_string(),
        args: String::new(),
        args_dict: None,
    })
}

fn extract_tool_calls(messages: &[ShareGptMessage<'_>]) -> Vec<ToolCall> {
    let mut out = Vec::new();
    for (i, msg) in messages.iter().enumerate() {
        if let Some(c) = parse_tool_call(i, msg) {
            out.push(c);
        }
    }
    out
}

fn detect_retry(calls: &[ToolCall]) -> Vec<(usize, usize, String)> {
    if calls.len() < RETRY_THRESHOLD {
        return Vec::new();
    }
    let mut patterns = Vec::new();
    let mut i = 0;
    while i < calls.len() {
        let current = &calls[i];
        let mut j = i + 1;
        let mut run_length = 1;
        while j < calls.len() {
            if calls[j].name == current.name && calls[j].args_equal(current) {
                run_length += 1;
                j += 1;
            } else {
                break;
            }
        }
        if run_length >= RETRY_THRESHOLD {
            patterns.push((calls[i].index, calls[j - 1].index, current.name.clone()));
            i = j;
        } else {
            i += 1;
        }
    }
    patterns
}

fn detect_parameter_drift(calls: &[ToolCall]) -> Vec<(usize, usize, String, usize)> {
    if calls.len() < PARAMETER_DRIFT_THRESHOLD {
        return Vec::new();
    }
    let mut patterns = Vec::new();
    let mut i = 0;
    while i < calls.len() {
        let current_name = calls[i].name.clone();
        let mut seen_args: Vec<String> = vec![calls[i].args.clone()];
        let mut unique_args = 1;
        let mut j = i + 1;
        while j < calls.len() {
            if calls[j].name != current_name {
                break;
            }
            if !seen_args.iter().any(|a| a == &calls[j].args) {
                seen_args.push(calls[j].args.clone());
                unique_args += 1;
            }
            j += 1;
        }
        let run_length = j - i;
        if run_length >= PARAMETER_DRIFT_THRESHOLD && unique_args >= 2 {
            patterns.push((
                calls[i].index,
                calls[j - 1].index,
                current_name,
                unique_args,
            ));
            i = j;
        } else {
            i += 1;
        }
    }
    patterns
}

fn detect_oscillation(calls: &[ToolCall]) -> Vec<(usize, usize, Vec<String>, usize)> {
    let min_calls = 2 * OSCILLATION_CYCLES_THRESHOLD;
    if calls.len() < min_calls {
        return Vec::new();
    }
    let mut patterns = Vec::new();
    let mut i: usize = 0;
    while i + min_calls <= calls.len() {
        let max_pat_len = (5usize).min(calls.len() - i);
        let mut found_for_i = false;
        for pat_len in 2..=max_pat_len {
            let pattern_names: Vec<String> =
                (0..pat_len).map(|k| calls[i + k].name.clone()).collect();
            let unique: std::collections::HashSet<&String> = pattern_names.iter().collect();
            if unique.len() < 2 {
                continue;
            }
            let mut cycles = 1;
            let mut pos = i + pat_len;
            while pos + pat_len <= calls.len() {
                let mut all_match = true;
                for k in 0..pat_len {
                    if calls[pos + k].name != pattern_names[k] {
                        all_match = false;
                        break;
                    }
                }
                if all_match {
                    cycles += 1;
                    pos += pat_len;
                } else {
                    break;
                }
            }
            if cycles >= OSCILLATION_CYCLES_THRESHOLD {
                let end_idx_in_calls = i + (cycles * pat_len) - 1;
                patterns.push((
                    calls[i].index,
                    calls[end_idx_in_calls].index,
                    pattern_names,
                    cycles,
                ));
                // Mirror Python: `i = end_idx + 1 - pattern_len`. We set `i` so that
                // the next outer iteration begins after we account for overlap.
                i = end_idx_in_calls + 1 - pat_len;
                found_for_i = true;
                break;
            }
        }
        if !found_for_i {
            i += 1;
        } else {
            // Match Python's `i = end_idx + 1 - pattern_len; break` then loop.
            // We'll continue; the outer while re-checks i.
        }
    }
    if patterns.len() > 1 {
        patterns = deduplicate_patterns(patterns);
    }
    patterns
}

fn deduplicate_patterns(
    mut patterns: Vec<(usize, usize, Vec<String>, usize)>,
) -> Vec<(usize, usize, Vec<String>, usize)> {
    if patterns.is_empty() {
        return patterns;
    }
    patterns.sort_by(|a, b| {
        let ord = a.0.cmp(&b.0);
        if ord != std::cmp::Ordering::Equal {
            ord
        } else {
            (b.1 - b.0).cmp(&(a.1 - a.0))
        }
    });
    let mut result = Vec::new();
    let mut last_end: i64 = -1;
    for p in patterns {
        if (p.0 as i64) > last_end {
            last_end = p.1 as i64;
            result.push(p);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Incremental loop detection.
//
// `analyze_loops` above rescans every tool call in the conversation on every
// `function_call` message, which is quadratic across a session. The types
// below track just enough state to detect retry / parameter_drift /
// oscillation in O(1) per new tool call, carried forward in
// `SignalReport::loop_state` across `SignalAnalyzer::analyze_step` calls.
//
// retry and parameter_drift are exact: both are properties of the maximal
// run of consecutive same-tool-name calls containing the newest call, and
// that run's qualifying signal(s) are fully determined the moment the run
// closes (or by its current extent while still open) — no rescan needed.
//
// oscillation is a best-effort approximation. The batch detector's greedy,
// variable-start, variable-period-length scan has no known O(1)-per-call
// incremental formulation that is exact for adversarial inputs (multiple
// simultaneously-valid candidate periods starting at different offsets).
// This tracks one candidate segment per period (2..=5) using only a bounded
// window of recent tool names, and exposes the shortest currently-qualifying
// period — which matches the batch detector on realistic tool-oscillation
// traces (a single active period at a time), but can diverge from it on
// contrived inputs with overlapping multi-period patterns. Divergences are
// caught by `fuzz_step_matches_batch_for_loops` below; residual known
// mismatches are documented there.

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SameToolRun {
    tool: String,
    start: usize,
    end: usize,
    len: usize,
    unique_args: Vec<String>,
    ident_args: String,
    ident_start: usize,
    ident_end: usize,
    ident_len: usize,
    /// Set once any identical-args sub-run within this run has hit
    /// `RETRY_THRESHOLD`. Mirrors the batch detector's rule that a
    /// parameter_drift pattern is suppressed if it overlaps a retry pattern
    /// (retry runs are always nested inside a drift run's same-tool span).
    overlaps_retry: bool,
}

impl SameToolRun {
    fn start_new(call: &ToolCall) -> Self {
        Self {
            tool: call.name.clone(),
            start: call.index,
            end: call.index,
            len: 1,
            unique_args: vec![call.args.clone()],
            ident_args: call.args.clone(),
            ident_start: call.index,
            ident_end: call.index,
            ident_len: 1,
            overlaps_retry: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LagTracker {
    period: usize,
    pattern: Vec<String>,
    seg_start: usize,
    run_len: usize,
    last_full_cycle_end: usize,
    seed_ok: bool,
}

/// Incremental loop-detection state. Opaque outside this module; carried in
/// `SignalReport::loop_state` and fed one tool call at a time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolCallState {
    recent: VecDeque<(usize, String)>,
    current_run: Option<SameToolRun>,
    osc_trackers: Vec<LagTracker>,
    closed_retry: Vec<(usize, usize, String, usize)>,
    /// `(start, end, tool_name, variation_count, call_count)`.
    closed_drift: Vec<(usize, usize, String, usize, usize)>,
    closed_osc: Vec<(usize, usize, Vec<String>, usize)>,
}

impl ToolCallState {
    /// Feed one parsed tool call into the state.
    fn push(&mut self, call: &ToolCall) {
        let recent_before = self.recent.clone();
        self.step_same_tool_run(call);
        self.step_oscillation(call, &recent_before);
        self.recent.push_back((call.index, call.name.clone()));
        while self.recent.len() > MAX_OSC_PATTERN_LEN {
            self.recent.pop_front();
        }
    }

    fn step_same_tool_run(&mut self, call: &ToolCall) {
        let continues = matches!(&self.current_run, Some(run) if run.tool == call.name);
        if continues {
            let run = self.current_run.as_mut().expect("checked by `continues`");
            run.len += 1;
            run.end = call.index;
            if !run.unique_args.iter().any(|a| a == &call.args) {
                run.unique_args.push(call.args.clone());
            }
            if call.args == run.ident_args {
                run.ident_len += 1;
                run.ident_end = call.index;
            } else {
                if run.ident_len >= RETRY_THRESHOLD {
                    self.closed_retry.push((
                        run.ident_start,
                        run.ident_end,
                        run.tool.clone(),
                        run.ident_len,
                    ));
                    run.overlaps_retry = true;
                }
                run.ident_args = call.args.clone();
                run.ident_start = call.index;
                run.ident_end = call.index;
                run.ident_len = 1;
            }
        } else {
            if let Some(old) = self.current_run.take() {
                self.finalize_run(old);
            }
            self.current_run = Some(SameToolRun::start_new(call));
        }
    }

    fn finalize_run(&mut self, run: SameToolRun) {
        let final_ident_qualifies = run.ident_len >= RETRY_THRESHOLD;
        if final_ident_qualifies {
            self.closed_retry.push((
                run.ident_start,
                run.ident_end,
                run.tool.clone(),
                run.ident_len,
            ));
        }
        let overlaps_retry = run.overlaps_retry || final_ident_qualifies;
        if run.len >= PARAMETER_DRIFT_THRESHOLD && run.unique_args.len() >= 2 && !overlaps_retry {
            self.closed_drift.push((
                run.start,
                run.end,
                run.tool.clone(),
                run.unique_args.len(),
                run.len,
            ));
        }
    }

    fn step_oscillation(&mut self, call: &ToolCall, recent_before: &VecDeque<(usize, String)>) {
        let recent_len = recent_before.len();
        for period in 2..=MAX_OSC_PATTERN_LEN {
            let tracker_idx = self.osc_trackers.iter().position(|t| t.period == period);
            let matches = recent_len >= period && recent_before[recent_len - period].1 == call.name;

            if matches {
                if let Some(idx) = tracker_idx {
                    let t = &mut self.osc_trackers[idx];
                    t.run_len += 1;
                    if t.run_len.is_multiple_of(period) {
                        t.last_full_cycle_end = call.index;
                    }
                }
                continue;
            }

            if let Some(idx) = tracker_idx {
                let old = self.osc_trackers.remove(idx);
                if old.seed_ok && old.run_len / old.period >= OSCILLATION_CYCLES_THRESHOLD {
                    self.closed_osc.push((
                        old.seg_start,
                        old.last_full_cycle_end,
                        old.pattern.clone(),
                        old.run_len / old.period,
                    ));
                }
            }

            if recent_len + 1 >= period {
                let mut seed: Vec<String> = recent_before
                    .iter()
                    .skip(recent_len - (period - 1))
                    .map(|(_, name)| name.clone())
                    .collect();
                let seg_start = recent_before[recent_len - (period - 1)].0;
                seed.push(call.name.clone());
                let unique_count = seed.iter().collect::<HashSet<&String>>().len();
                self.osc_trackers.push(LagTracker {
                    period,
                    pattern: seed,
                    seg_start,
                    run_len: period,
                    last_full_cycle_end: call.index,
                    seed_ok: unique_count >= 2,
                });
            }
        }
    }

    fn current_osc_candidate(&self) -> Option<(usize, usize, Vec<String>, usize)> {
        let mut sorted: Vec<&LagTracker> = self.osc_trackers.iter().collect();
        sorted.sort_by_key(|t| t.period);
        sorted.into_iter().find_map(|t| {
            if t.seed_ok && t.run_len / t.period >= OSCILLATION_CYCLES_THRESHOLD {
                Some((
                    t.seg_start,
                    t.last_full_cycle_end,
                    t.pattern.clone(),
                    t.run_len / t.period,
                ))
            } else {
                None
            }
        })
    }

    /// Render the current "loops" `SignalGroup` from accumulated + in-progress state.
    fn current_signals(&self) -> SignalGroup {
        let mut group = SignalGroup::new("loops");

        for (start, end, name, count) in &self.closed_retry {
            group.add_signal(retry_signal(*start, *end, name, *count));
        }
        if let Some(run) = &self.current_run {
            if run.ident_len >= RETRY_THRESHOLD {
                group.add_signal(retry_signal(
                    run.ident_start,
                    run.ident_end,
                    &run.tool,
                    run.ident_len,
                ));
            }
        }

        for (start, end, name, variation_count, call_count) in &self.closed_drift {
            group.add_signal(drift_signal(
                *start,
                *end,
                name,
                *variation_count,
                *call_count,
            ));
        }
        if let Some(run) = &self.current_run {
            let overlaps_retry = run.overlaps_retry || run.ident_len >= RETRY_THRESHOLD;
            if run.len >= PARAMETER_DRIFT_THRESHOLD && run.unique_args.len() >= 2 && !overlaps_retry
            {
                group.add_signal(drift_signal(
                    run.start,
                    run.end,
                    &run.tool,
                    run.unique_args.len(),
                    run.len,
                ));
            }
        }

        let mut osc_patterns = self.closed_osc.clone();
        if let Some(cand) = self.current_osc_candidate() {
            osc_patterns.push(cand);
        }
        let osc_patterns = deduplicate_patterns(osc_patterns);
        for (start, end, pattern, cycles) in &osc_patterns {
            group.add_signal(oscillation_signal(*start, *end, pattern, *cycles));
        }

        group
    }
}

fn retry_signal(start: usize, end: usize, tool_name: &str, call_count: usize) -> SignalInstance {
    SignalInstance::new(
        SignalType::ExecutionLoopsRetry,
        start,
        format!(
            "Tool '{}' called {} times with identical arguments",
            tool_name, call_count
        ),
    )
    .with_confidence(0.95)
    .with_metadata(json!({
        "tool_name": tool_name,
        "start_index": start,
        "end_index": end,
        "call_count": call_count,
        "loop_type": "retry",
    }))
}

fn drift_signal(
    start: usize,
    end: usize,
    tool_name: &str,
    variation_count: usize,
    call_count: usize,
) -> SignalInstance {
    SignalInstance::new(
        SignalType::ExecutionLoopsParameterDrift,
        start,
        format!(
            "Tool '{}' called {} times with {} different argument variations",
            tool_name, call_count, variation_count
        ),
    )
    .with_confidence(0.85)
    .with_metadata(json!({
        "tool_name": tool_name,
        "start_index": start,
        "end_index": end,
        "call_count": call_count,
        "variation_count": variation_count,
        "loop_type": "parameter_drift",
    }))
}

fn oscillation_signal(
    start: usize,
    end: usize,
    pattern: &[String],
    cycles: usize,
) -> SignalInstance {
    let pattern_str = pattern.join(" \u{2192} ");
    SignalInstance::new(
        SignalType::ExecutionLoopsOscillation,
        start,
        format!(
            "Oscillation pattern [{}] repeated {} times",
            pattern_str, cycles
        ),
    )
    .with_confidence(0.9)
    .with_metadata(json!({
        "pattern": pattern,
        "start_index": start,
        "end_index": end,
        "cycle_count": cycles,
        "loop_type": "oscillation",
    }))
}

/// Incremental entry point: feed one message (only `function_call` messages
/// actually update state) and return the up-to-date "loops" `SignalGroup`.
/// Callers on non-`function_call` messages should keep the previous report's
/// `execution.loops` group unchanged instead of calling this.
pub fn analyze_loops_step(
    state: &mut ToolCallState,
    message_index: usize,
    msg: &ShareGptMessage<'_>,
) -> SignalGroup {
    if let Some(call) = parse_tool_call(message_index, msg) {
        state.push(&call);
    }
    state.current_signals()
}

pub fn analyze_loops(messages: &[ShareGptMessage<'_>]) -> SignalGroup {
    let mut group = SignalGroup::new("loops");
    let calls = extract_tool_calls(messages);
    if calls.len() < RETRY_THRESHOLD {
        return group;
    }

    let retries = detect_retry(&calls);
    for (start_idx, end_idx, tool_name) in &retries {
        let call_count = calls
            .iter()
            .filter(|c| *start_idx <= c.index && c.index <= *end_idx)
            .count();
        group.add_signal(
            SignalInstance::new(
                SignalType::ExecutionLoopsRetry,
                *start_idx,
                format!(
                    "Tool '{}' called {} times with identical arguments",
                    tool_name, call_count
                ),
            )
            .with_confidence(0.95)
            .with_metadata(json!({
                "tool_name": tool_name,
                "start_index": start_idx,
                "end_index": end_idx,
                "call_count": call_count,
                "loop_type": "retry",
            })),
        );
    }

    let drifts = detect_parameter_drift(&calls);
    for (start_idx, end_idx, tool_name, variation_count) in &drifts {
        let overlaps_retry = retries
            .iter()
            .any(|r| !(*end_idx < r.0 || *start_idx > r.1));
        if overlaps_retry {
            continue;
        }
        let call_count = calls
            .iter()
            .filter(|c| *start_idx <= c.index && c.index <= *end_idx)
            .count();
        group.add_signal(
            SignalInstance::new(
                SignalType::ExecutionLoopsParameterDrift,
                *start_idx,
                format!(
                    "Tool '{}' called {} times with {} different argument variations",
                    tool_name, call_count, variation_count
                ),
            )
            .with_confidence(0.85)
            .with_metadata(json!({
                "tool_name": tool_name,
                "start_index": start_idx,
                "end_index": end_idx,
                "call_count": call_count,
                "variation_count": variation_count,
                "loop_type": "parameter_drift",
            })),
        );
    }

    let oscillations = detect_oscillation(&calls);
    for (start_idx, end_idx, tool_names, cycle_count) in &oscillations {
        let pattern_str = tool_names.join(" \u{2192} ");
        group.add_signal(
            SignalInstance::new(
                SignalType::ExecutionLoopsOscillation,
                *start_idx,
                format!(
                    "Oscillation pattern [{}] repeated {} times",
                    pattern_str, cycle_count
                ),
            )
            .with_confidence(0.9)
            .with_metadata(json!({
                "pattern": tool_names,
                "start_index": start_idx,
                "end_index": end_idx,
                "cycle_count": cycle_count,
                "loop_type": "oscillation",
            })),
        );
    }

    group
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fc(value: &str) -> ShareGptMessage<'_> {
        ShareGptMessage {
            from: "function_call",
            value,
        }
    }

    #[test]
    fn detects_retry_loop() {
        let arg = r#"{"name":"check_status","arguments":{"id":"abc"}}"#;
        let msgs = vec![fc(arg), fc(arg), fc(arg), fc(arg)];
        let g = analyze_loops(&msgs);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::ExecutionLoopsRetry)));
    }

    #[test]
    fn detects_parameter_drift() {
        let msgs = vec![
            fc(r#"{"name":"search","arguments":{"q":"a"}}"#),
            fc(r#"{"name":"search","arguments":{"q":"ab"}}"#),
            fc(r#"{"name":"search","arguments":{"q":"abc"}}"#),
            fc(r#"{"name":"search","arguments":{"q":"abcd"}}"#),
        ];
        let g = analyze_loops(&msgs);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::ExecutionLoopsParameterDrift)));
    }

    #[test]
    fn detects_oscillation() {
        let a = r#"{"name":"toolA","arguments":{}}"#;
        let b = r#"{"name":"toolB","arguments":{}}"#;
        let msgs = vec![fc(a), fc(b), fc(a), fc(b), fc(a), fc(b)];
        let g = analyze_loops(&msgs);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::ExecutionLoopsOscillation)));
    }

    #[test]
    fn no_signals_when_few_calls() {
        let msgs = vec![fc(r#"{"name":"only_once","arguments":{}}"#)];
        let g = analyze_loops(&msgs);
        assert!(g.signals.is_empty());
    }
}

#[cfg(test)]
mod incremental_tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    fn signal_key(s: &SignalInstance) -> (String, usize, String) {
        (
            s.signal_type.as_str().to_string(),
            s.message_index,
            s.metadata.to_string(),
        )
    }

    /// Owned (from, value) rows so the fuzz driver can build arbitrary
    /// randomized conversations without lifetime headaches.
    fn random_conversation(
        rng: &mut StdRng,
        len: usize,
        tool_names: &[&str],
    ) -> Vec<(String, String)> {
        let mut out = Vec::with_capacity(len);
        for _ in 0..len {
            let kind = rng.random_range(0..10u32);
            if kind < 7 {
                let name = tool_names[rng.random_range(0..tool_names.len())];
                let arg_variant = rng.random_range(0..3u32);
                out.push((
                    "function_call".to_string(),
                    format!(
                        r#"{{"name":"{}","arguments":{{"v":{}}}}}"#,
                        name, arg_variant
                    ),
                ));
            } else if kind < 8 {
                out.push(("observation".to_string(), "ok".to_string()));
            } else if kind < 9 {
                out.push(("human".to_string(), "hi".to_string()));
            } else {
                out.push(("gpt".to_string(), "hi".to_string()));
            }
        }
        out
    }

    /// Runs both the batch detector (rescanning the growing prefix every
    /// step) and the incremental `ToolCallState` (one call per new message)
    /// over the same conversation, asserting they agree at every prefix.
    /// Returns `Ok(())` on full agreement, or `Err(mismatch_count)`.
    fn compare_batch_vs_incremental(rows: &[(String, String)]) -> usize {
        let mut state = ToolCallState::default();
        let mut mismatches = 0;
        for i in 0..rows.len() {
            let msgs: Vec<ShareGptMessage<'_>> = rows[..=i]
                .iter()
                .map(|(from, value)| ShareGptMessage {
                    from: from.as_str(),
                    value: value.as_str(),
                })
                .collect();
            let batch = analyze_loops(&msgs);
            let step = analyze_loops_step(&mut state, i, &msgs[i]);
            let mut a: Vec<_> = step.signals.iter().map(signal_key).collect();
            let mut b: Vec<_> = batch.signals.iter().map(signal_key).collect();
            a.sort();
            b.sort();
            if a != b {
                mismatches += 1;
            }
        }
        mismatches
    }

    #[test]
    fn incremental_matches_batch_for_retry() {
        let arg = r#"{"name":"check_status","arguments":{"id":"abc"}}"#;
        let rows: Vec<(String, String)> = vec![arg, arg, arg, arg]
            .into_iter()
            .map(|v| ("function_call".to_string(), v.to_string()))
            .collect();
        assert_eq!(compare_batch_vs_incremental(&rows), 0);
    }

    #[test]
    fn incremental_matches_batch_for_parameter_drift() {
        let rows: Vec<(String, String)> = vec![
            r#"{"name":"search","arguments":{"q":"a"}}"#,
            r#"{"name":"search","arguments":{"q":"ab"}}"#,
            r#"{"name":"search","arguments":{"q":"abc"}}"#,
            r#"{"name":"search","arguments":{"q":"abcd"}}"#,
        ]
        .into_iter()
        .map(|v| ("function_call".to_string(), v.to_string()))
        .collect();
        assert_eq!(compare_batch_vs_incremental(&rows), 0);
    }

    #[test]
    fn incremental_matches_batch_for_retry_then_drift_in_same_run() {
        // Same tool throughout: A,A,A (retry) then B,C,D (drift, args all
        // distinct) — exercises closing an identical-args sub-run mid-run
        // without closing the whole same-tool-name run.
        let rows: Vec<(String, String)> = vec!["a", "a", "a", "b", "c", "d"]
            .into_iter()
            .map(|v| {
                (
                    "function_call".to_string(),
                    format!(r#"{{"name":"search","arguments":{{"q":"{}"}}}}"#, v),
                )
            })
            .collect();
        assert_eq!(compare_batch_vs_incremental(&rows), 0);
    }

    #[test]
    fn incremental_matches_batch_for_oscillation() {
        let rows: Vec<(String, String)> =
            vec!["toolA", "toolB", "toolA", "toolB", "toolA", "toolB"]
                .into_iter()
                .map(|name| {
                    (
                        "function_call".to_string(),
                        format!(r#"{{"name":"{}","arguments":{{}}}}"#, name),
                    )
                })
                .collect();
        assert_eq!(compare_batch_vs_incremental(&rows), 0);
    }

    #[test]
    fn incremental_matches_batch_with_interspersed_non_call_messages() {
        let rows: Vec<(String, String)> = vec![
            ("human".to_string(), "hi".to_string()),
            (
                "function_call".to_string(),
                r#"{"name":"x","arguments":{"a":1}}"#.to_string(),
            ),
            ("observation".to_string(), "ok".to_string()),
            (
                "function_call".to_string(),
                r#"{"name":"x","arguments":{"a":1}}"#.to_string(),
            ),
            ("observation".to_string(), "ok".to_string()),
            (
                "function_call".to_string(),
                r#"{"name":"x","arguments":{"a":1}}"#.to_string(),
            ),
            ("gpt".to_string(), "done".to_string()),
        ];
        assert_eq!(compare_batch_vs_incremental(&rows), 0);
    }

    /// Randomized fuzz comparison against the batch reference. Oscillation
    /// is a best-effort approximation (see module docs above), so a small
    /// number of mismatches on adversarial multi-period patterns is
    /// tolerated and reported rather than silently ignored. As of this
    /// writing, 200 trials of length-40 conversations over a small tool
    /// vocabulary (2-3 distinct names, the realistic "agent oscillating
    /// between tools" shape) produce zero mismatches; this test fails loudly
    /// if that regresses.
    #[test]
    fn fuzz_step_matches_batch_for_loops() {
        let mut total_mismatches = 0usize;
        for seed in 0..200u64 {
            let mut rng = StdRng::seed_from_u64(seed);
            let tool_names: &[&str] = if seed % 2 == 0 {
                &["toolA", "toolB"]
            } else {
                &["toolA", "toolB", "toolC"]
            };
            let rows = random_conversation(&mut rng, 40, tool_names);
            total_mismatches += compare_batch_vs_incremental(&rows);
        }
        assert_eq!(
            total_mismatches, 0,
            "incremental loop detection diverged from the batch reference on {} \
             prefixes across the fuzz corpus; see module docs for the known \
             oscillation-approximation caveat",
            total_mismatches
        );
    }
}
