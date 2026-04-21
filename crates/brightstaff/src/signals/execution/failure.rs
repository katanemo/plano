//! Execution failure detector. Direct port of `signals/execution/failure.py`.

use std::sync::OnceLock;

use regex::Regex;
use serde_json::json;

use crate::signals::analyzer::ShareGptMessage;
use crate::signals::schemas::{SignalGroup, SignalInstance, SignalType};

pub const INVALID_ARGS_PATTERNS: &[&str] = &[
    r"invalid\s+argument",
    r"invalid\s+parameter",
    r"invalid\s+type",
    r"type\s*error",
    r"expected\s+\w+\s*,?\s*got\s+\w+",
    r"required\s+field",
    r"required\s+parameter",
    r"missing\s+required",
    r"missing\s+argument",
    r"validation\s+failed",
    r"validation\s+error",
    r"invalid\s+value",
    r"invalid\s+format",
    r"must\s+be\s+(a|an)\s+\w+",
    r"cannot\s+be\s+(null|empty|none)",
    r"is\s+not\s+valid",
    r"does\s+not\s+match",
    r"out\s+of\s+range",
    r"invalid\s+date",
    r"invalid\s+json",
    r"malformed\s+request",
];

pub const BAD_QUERY_PATTERNS: &[&str] = &[
    r"invalid\s+query",
    r"query\s+syntax\s+error",
    r"malformed\s+query",
    r"unknown\s+field",
    r"invalid\s+field",
    r"invalid\s+filter",
    r"invalid\s+search",
    r"unknown\s+id",
    r"invalid\s+id",
    r"id\s+format\s+error",
    r"invalid\s+identifier",
    r"query\s+failed",
    r"search\s+error",
    r"invalid\s+operator",
    r"unsupported\s+query",
];

pub const TOOL_NOT_FOUND_PATTERNS: &[&str] = &[
    r"unknown\s+function",
    r"unknown\s+tool",
    r"function\s+not\s+found",
    r"tool\s+not\s+found",
    r"no\s+such\s+function",
    r"no\s+such\s+tool",
    r"undefined\s+function",
    r"action\s+not\s+supported",
    r"invalid\s+tool",
    r"invalid\s+function",
    r"unrecognized\s+function",
];

pub const AUTH_MISUSE_PATTERNS: &[&str] = &[
    r"\bunauthorized\b",
    r"(status|error|http|code)\s*:?\s*401",
    r"401\s+unauthorized",
    r"403\s+forbidden",
    r"permission\s+denied",
    r"access\s+denied",
    r"authentication\s+required",
    r"invalid\s+credentials",
    r"invalid\s+token",
    r"token\s+expired",
    r"missing\s+authorization",
    r"\bforbidden\b",
    r"not\s+authorized",
    r"insufficient\s+permissions?",
];

pub const STATE_ERROR_PATTERNS: &[&str] = &[
    r"invalid\s+state",
    r"illegal\s+state",
    r"must\s+call\s+\w+\s+first",
    r"must\s+\w+\s+before",
    r"cannot\s+\w+\s+before",
    r"already\s+(exists?|created|started|finished)",
    r"not\s+initialized",
    r"not\s+started",
    r"already\s+in\s+progress",
    r"operation\s+in\s+progress",
    r"sequence\s+error",
    r"precondition\s+failed",
    r"(status|error|http)\s*:?\s*409",
    r"409\s+conflict",
    r"\bconflict\b",
];

fn compile(patterns: &[&str]) -> Regex {
    // Use `(?i)` flag for case-insensitive matching, matching Python's `re.IGNORECASE`.
    let combined = patterns
        .iter()
        .map(|p| format!("({})", p))
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&format!("(?i){}", combined)).expect("failure pattern regex must compile")
}

fn invalid_args_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| compile(INVALID_ARGS_PATTERNS))
}
fn bad_query_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| compile(BAD_QUERY_PATTERNS))
}
fn tool_not_found_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| compile(TOOL_NOT_FOUND_PATTERNS))
}
fn auth_misuse_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| compile(AUTH_MISUSE_PATTERNS))
}
fn state_error_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| compile(STATE_ERROR_PATTERNS))
}

/// Pull tool name + args from a `function_call` message. Mirrors
/// `_extract_tool_info` in the reference.
pub(crate) fn extract_tool_info(value: &str) -> (String, String) {
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(value) {
        if let Some(obj) = parsed.as_object() {
            let name = obj
                .get("name")
                .or_else(|| obj.get("function"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let args = match obj.get("arguments").or_else(|| obj.get("args")) {
                Some(serde_json::Value::Object(o)) => {
                    serde_json::to_string(&serde_json::Value::Object(o.clone())).unwrap_or_default()
                }
                Some(other) => other
                    .as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| serde_json::to_string(other).unwrap_or_default()),
                None => String::new(),
            };
            return (name, args);
        }
    }
    let mut snippet: String = value.chars().take(200).collect();
    snippet.shrink_to_fit();
    ("unknown".to_string(), snippet)
}

/// Build a context-window snippet around a regex match, with leading/trailing
/// ellipses when truncated. Mirrors `_get_snippet`.
fn snippet_around(text: &str, m: regex::Match<'_>, context: usize) -> String {
    let start = m.start().saturating_sub(context);
    let end = (m.end() + context).min(text.len());
    // Ensure we cut on UTF-8 boundaries.
    let start = align_char_boundary(text, start, false);
    let end = align_char_boundary(text, end, true);
    let mut snippet = String::new();
    if start > 0 {
        snippet.push_str("...");
    }
    snippet.push_str(&text[start..end]);
    if end < text.len() {
        snippet.push_str("...");
    }
    snippet
}

fn align_char_boundary(s: &str, mut idx: usize, forward: bool) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    while !s.is_char_boundary(idx) {
        if forward {
            idx += 1;
        } else if idx == 0 {
            break;
        } else {
            idx -= 1;
        }
    }
    idx
}

pub fn analyze_failure(messages: &[ShareGptMessage<'_>]) -> SignalGroup {
    let mut group = SignalGroup::new("failure");
    let mut last_call: Option<(usize, String, String)> = None;

    for (i, msg) in messages.iter().enumerate() {
        match msg.from {
            "function_call" => {
                let (name, args) = extract_tool_info(msg.value);
                last_call = Some((i, name, args));
                continue;
            }
            "observation" => {}
            _ => continue,
        }

        let value = msg.value;
        let lower = value.to_lowercase();
        let (call_index, tool_name) = match &last_call {
            Some((idx, name, _)) => (*idx, name.clone()),
            None => (i.saturating_sub(1), "unknown".to_string()),
        };

        if let Some(m) = invalid_args_re().find(&lower) {
            group.add_signal(
                SignalInstance::new(
                    SignalType::ExecutionFailureInvalidArgs,
                    i,
                    snippet_around(value, m, 50),
                )
                .with_confidence(0.9)
                .with_metadata(json!({
                    "tool_name": tool_name,
                    "call_index": call_index,
                    "error_type": "invalid_args",
                    "matched": m.as_str(),
                })),
            );
            continue;
        }

        if let Some(m) = tool_not_found_re().find(&lower) {
            group.add_signal(
                SignalInstance::new(
                    SignalType::ExecutionFailureToolNotFound,
                    i,
                    snippet_around(value, m, 50),
                )
                .with_confidence(0.95)
                .with_metadata(json!({
                    "tool_name": tool_name,
                    "call_index": call_index,
                    "error_type": "tool_not_found",
                    "matched": m.as_str(),
                })),
            );
            continue;
        }

        if let Some(m) = auth_misuse_re().find(&lower) {
            group.add_signal(
                SignalInstance::new(
                    SignalType::ExecutionFailureAuthMisuse,
                    i,
                    snippet_around(value, m, 50),
                )
                .with_confidence(0.8)
                .with_metadata(json!({
                    "tool_name": tool_name,
                    "call_index": call_index,
                    "error_type": "auth_misuse",
                    "matched": m.as_str(),
                })),
            );
            continue;
        }

        if let Some(m) = state_error_re().find(&lower) {
            group.add_signal(
                SignalInstance::new(
                    SignalType::ExecutionFailureStateError,
                    i,
                    snippet_around(value, m, 50),
                )
                .with_confidence(0.85)
                .with_metadata(json!({
                    "tool_name": tool_name,
                    "call_index": call_index,
                    "error_type": "state_error",
                    "matched": m.as_str(),
                })),
            );
            continue;
        }

        if let Some(m) = bad_query_re().find(&lower) {
            let confidence = if ["error", "invalid", "failed"]
                .iter()
                .any(|w| lower.contains(w))
            {
                0.8
            } else {
                0.6
            };
            group.add_signal(
                SignalInstance::new(
                    SignalType::ExecutionFailureBadQuery,
                    i,
                    snippet_around(value, m, 50),
                )
                .with_confidence(confidence)
                .with_metadata(json!({
                    "tool_name": tool_name,
                    "call_index": call_index,
                    "error_type": "bad_query",
                    "matched": m.as_str(),
                })),
            );
        }
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
    fn obs(value: &str) -> ShareGptMessage<'_> {
        ShareGptMessage {
            from: "observation",
            value,
        }
    }

    #[test]
    fn detects_invalid_args() {
        let msgs = vec![
            fc(r#"{"name":"create_user","arguments":{"age":"twelve"}}"#),
            obs("Error: validation failed - expected integer got string for field age"),
        ];
        let g = analyze_failure(&msgs);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::ExecutionFailureInvalidArgs)));
    }

    #[test]
    fn detects_tool_not_found() {
        let msgs = vec![
            fc(r#"{"name":"send_thought","arguments":{}}"#),
            obs("Error: unknown function 'send_thought'"),
        ];
        let g = analyze_failure(&msgs);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::ExecutionFailureToolNotFound)));
    }

    #[test]
    fn detects_auth_misuse() {
        let msgs = vec![
            fc(r#"{"name":"get_secret","arguments":{}}"#),
            obs("HTTP 401 Unauthorized"),
        ];
        let g = analyze_failure(&msgs);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::ExecutionFailureAuthMisuse)));
    }

    #[test]
    fn detects_state_error() {
        let msgs = vec![
            fc(r#"{"name":"commit_tx","arguments":{}}"#),
            obs("must call begin_tx first"),
        ];
        let g = analyze_failure(&msgs);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::ExecutionFailureStateError)));
    }
}
