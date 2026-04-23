//! Execution loops detector. Direct port of `signals/execution/loops.py`.

use serde_json::json;

use crate::signals::analyzer::ShareGptMessage;
use crate::signals::schemas::{SignalGroup, SignalInstance, SignalType};

pub const RETRY_THRESHOLD: usize = 3;
pub const PARAMETER_DRIFT_THRESHOLD: usize = 3;
pub const OSCILLATION_CYCLES_THRESHOLD: usize = 3;

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
