//! Environment exhaustion detector. Direct port of
//! `signals/environment/exhaustion.py`.

use std::sync::OnceLock;

use regex::Regex;
use serde_json::json;

use crate::signals::analyzer::ShareGptMessage;
use crate::signals::schemas::{SignalGroup, SignalInstance, SignalType};

pub const API_ERROR_PATTERNS: &[&str] = &[
    r"500\s*(internal\s+)?server\s+error",
    r"502\s*bad\s+gateway",
    r"503\s*service\s+unavailable",
    r"504\s*gateway\s+timeout",
    r"internal\s+server\s+error",
    r"service\s+unavailable",
    r"server\s+error",
    r"backend\s+error",
    r"upstream\s+error",
    r"service\s+temporarily\s+unavailable",
    r"maintenance\s+mode",
    r"under\s+maintenance",
    r"try\s+again\s+later",
    r"temporarily\s+unavailable",
    r"system\s+error",
    r"unexpected\s+error",
    r"unhandled\s+exception",
];

pub const TIMEOUT_PATTERNS: &[&str] = &[
    r"timeout",
    r"timed?\s*out",
    r"etimedout",
    r"connection\s+timed?\s*out",
    r"read\s+timed?\s*out",
    r"request\s+timed?\s*out",
    r"gateway\s+timeout",
    r"deadline\s+exceeded",
    r"took\s+too\s+long",
    r"operation\s+timed?\s*out",
    r"socket\s+timeout",
];

pub const RATE_LIMIT_PATTERNS: &[&str] = &[
    r"rate\s+limit",
    r"rate.limited",
    r"(status|error|http)\s*:?\s*429",
    r"429\s+(too\s+many|rate|limit)",
    r"too\s+many\s+requests?",
    r"quota\s+exceeded",
    r"quota\s+limit",
    r"throttl(ed|ing)",
    r"request\s+limit",
    r"api\s+limit",
    r"calls?\s+per\s+(second|minute|hour|day)",
    r"exceeded\s+.*\s+limit",
    r"slow\s+down",
    r"retry\s+after",
    r"requests?\s+exceeded",
];

pub const NETWORK_PATTERNS: &[&str] = &[
    r"connection\s+refused",
    r"econnrefused",
    r"econnreset",
    r"connection\s+reset",
    r"enotfound",
    r"dns\s+(error|failure|lookup)",
    r"host\s+not\s+found",
    r"network\s+(error|failure|unreachable)",
    r"no\s+route\s+to\s+host",
    r"socket\s+error",
    r"connection\s+failed",
    r"unable\s+to\s+connect",
    r"cannot\s+connect",
    r"could\s+not\s+connect",
    r"connect\s+error",
    r"ssl\s+(error|handshake|certificate)",
    r"certificate\s+(error|invalid|expired)",
];

pub const MALFORMED_PATTERNS: &[&str] = &[
    r"json\s+parse\s+error",
    r"invalid\s+json",
    r"unexpected\s+token",
    r"syntax\s+error.*json",
    r"malformed\s+(response|json|data)",
    r"unexpected\s+end\s+of",
    r"parse\s+error",
    r"parsing\s+failed",
    r"invalid\s+response",
    r"unexpected\s+response",
    r"response\s+format",
    r"missing\s+field.*response",
    r"unexpected\s+schema",
    r"schema\s+validation",
    r"deserialization\s+error",
    r"failed\s+to\s+decode",
];

pub const CONTEXT_OVERFLOW_PATTERNS: &[&str] = &[
    r"context\s+(length|limit|overflow|exceeded)",
    r"token\s+(limit|overflow|exceeded)",
    r"max(imum)?\s+tokens?",
    r"input\s+too\s+(long|large)",
    r"exceeds?\s+(context|token|character|input)\s+limit",
    r"message\s+too\s+(long|large)",
    r"content\s+too\s+(long|large)",
    r"truncat(ed|ion)\s+(due\s+to|because|for)\s+(length|size|limit)",
    r"maximum\s+context",
    r"prompt\s+too\s+(long|large)",
];

fn compile(patterns: &[&str]) -> Regex {
    let combined = patterns
        .iter()
        .map(|p| format!("({})", p))
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&format!("(?i){}", combined)).expect("exhaustion pattern regex must compile")
}

fn api_error_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| compile(API_ERROR_PATTERNS))
}
fn timeout_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| compile(TIMEOUT_PATTERNS))
}
fn rate_limit_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| compile(RATE_LIMIT_PATTERNS))
}
fn network_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| compile(NETWORK_PATTERNS))
}
fn malformed_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| compile(MALFORMED_PATTERNS))
}
fn context_overflow_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| compile(CONTEXT_OVERFLOW_PATTERNS))
}

fn snippet_around(text: &str, m: regex::Match<'_>, context: usize) -> String {
    let start = m.start().saturating_sub(context);
    let end = (m.end() + context).min(text.len());
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

pub fn analyze_exhaustion(messages: &[ShareGptMessage<'_>]) -> SignalGroup {
    let mut group = SignalGroup::new("exhaustion");

    for (i, msg) in messages.iter().enumerate() {
        if msg.from != "observation" {
            continue;
        }
        let value = msg.value;
        let lower = value.to_lowercase();

        if let Some(m) = rate_limit_re().find(&lower) {
            group.add_signal(emit(
                SignalType::EnvironmentExhaustionRateLimit,
                i,
                snippet_around(value, m, 50),
                0.95,
                "rate_limit",
                m.as_str(),
            ));
            continue;
        }

        if let Some(m) = api_error_re().find(&lower) {
            group.add_signal(emit(
                SignalType::EnvironmentExhaustionApiError,
                i,
                snippet_around(value, m, 50),
                0.9,
                "api_error",
                m.as_str(),
            ));
            continue;
        }

        if let Some(m) = timeout_re().find(&lower) {
            group.add_signal(emit(
                SignalType::EnvironmentExhaustionTimeout,
                i,
                snippet_around(value, m, 50),
                0.9,
                "timeout",
                m.as_str(),
            ));
            continue;
        }

        if let Some(m) = network_re().find(&lower) {
            group.add_signal(emit(
                SignalType::EnvironmentExhaustionNetwork,
                i,
                snippet_around(value, m, 50),
                0.9,
                "network",
                m.as_str(),
            ));
            continue;
        }

        if let Some(m) = malformed_re().find(&lower) {
            group.add_signal(emit(
                SignalType::EnvironmentExhaustionMalformed,
                i,
                snippet_around(value, m, 50),
                0.85,
                "malformed_response",
                m.as_str(),
            ));
            continue;
        }

        if let Some(m) = context_overflow_re().find(&lower) {
            group.add_signal(emit(
                SignalType::EnvironmentExhaustionContextOverflow,
                i,
                snippet_around(value, m, 50),
                0.9,
                "context_overflow",
                m.as_str(),
            ));
        }
    }

    group
}

fn emit(
    t: SignalType,
    idx: usize,
    snippet: String,
    confidence: f32,
    kind: &str,
    matched: &str,
) -> SignalInstance {
    SignalInstance::new(t, idx, snippet)
        .with_confidence(confidence)
        .with_metadata(json!({
            "exhaustion_type": kind,
            "matched": matched,
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(value: &str) -> ShareGptMessage<'_> {
        ShareGptMessage {
            from: "observation",
            value,
        }
    }

    #[test]
    fn detects_rate_limit() {
        let g = analyze_exhaustion(&[obs("HTTP 429: too many requests, retry after 30s")]);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::EnvironmentExhaustionRateLimit)));
    }

    #[test]
    fn detects_api_error() {
        let g = analyze_exhaustion(&[obs("503 service unavailable - try again later")]);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::EnvironmentExhaustionApiError)));
    }

    #[test]
    fn detects_timeout() {
        let g = analyze_exhaustion(&[obs("Connection timed out after 30 seconds")]);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::EnvironmentExhaustionTimeout)));
    }

    #[test]
    fn detects_network_failure() {
        let g = analyze_exhaustion(&[obs("ECONNREFUSED: connection refused by remote host")]);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::EnvironmentExhaustionNetwork)));
    }

    #[test]
    fn detects_malformed_response() {
        let g = analyze_exhaustion(&[obs("Invalid JSON: unexpected token at position 42")]);
        assert!(g
            .signals
            .iter()
            .any(|s| matches!(s.signal_type, SignalType::EnvironmentExhaustionMalformed)));
    }

    #[test]
    fn detects_context_overflow() {
        let g = analyze_exhaustion(&[obs("Maximum context length exceeded for this model")]);
        assert!(g.signals.iter().any(|s| matches!(
            s.signal_type,
            SignalType::EnvironmentExhaustionContextOverflow
        )));
    }
}
