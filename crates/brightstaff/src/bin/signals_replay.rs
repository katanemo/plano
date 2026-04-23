//! `signals-replay` — batch driver for the `brightstaff` signal analyzer.
//!
//! Reads JSONL conversations from stdin (one per line) and emits matching
//! JSONL reports on stdout, one per input conversation, in the same order.
//!
//! Input shape (per line):
//! ```json
//! {"id": "convo-42", "messages": [{"from": "human", "value": "..."}, ...]}
//! ```
//!
//! Output shape (per line, success):
//! ```json
//! {"id": "convo-42", "report": { ...python-compatible SignalReport dict... }}
//! ```
//!
//! On per-line failure (parse / analyzer error), emits:
//! ```json
//! {"id": "convo-42", "error": "..."}
//! ```
//!
//! The output report dict is shaped to match the Python reference's
//! `SignalReport.to_dict()` byte-for-byte so the parity comparator can do a
//! direct structural diff.

use std::io::{self, BufRead, BufWriter, Write};

use serde::Deserialize;
use serde_json::{json, Map, Value};

use brightstaff::signals::{SignalAnalyzer, SignalGroup, SignalReport};

#[derive(Debug, Deserialize)]
struct InputLine {
    id: Value,
    messages: Vec<MessageRow>,
}

#[derive(Debug, Deserialize)]
struct MessageRow {
    #[serde(default)]
    from: String,
    #[serde(default)]
    value: String,
}

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    let analyzer = SignalAnalyzer::default();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("read error: {e}");
                std::process::exit(1);
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let result = process_line(&analyzer, trimmed);
        // Always emit one line per input line so id ordering stays aligned.
        if let Err(e) = writeln!(out, "{result}") {
            eprintln!("write error: {e}");
            std::process::exit(1);
        }
        // Flush periodically isn't strictly needed — BufWriter handles it,
        // and the parent process reads the whole stream when we're done.
    }
    let _ = out.flush();
}

fn process_line(analyzer: &SignalAnalyzer, line: &str) -> Value {
    let parsed: InputLine = match serde_json::from_str(line) {
        Ok(p) => p,
        Err(e) => {
            return json!({
                "id": Value::Null,
                "error": format!("input parse: {e}"),
            });
        }
    };

    let id = parsed.id.clone();

    let view: Vec<brightstaff::signals::analyzer::ShareGptMessage<'_>> = parsed
        .messages
        .iter()
        .map(|m| brightstaff::signals::analyzer::ShareGptMessage {
            from: m.from.as_str(),
            value: m.value.as_str(),
        })
        .collect();

    let report = analyzer.analyze_sharegpt(&view);
    let report_dict = report_to_python_dict(&report);
    json!({
        "id": id,
        "report": report_dict,
    })
}

/// Convert a `SignalReport` into the Python reference's `to_dict()` shape.
///
/// Ordering of category keys in each layer dict follows the Python source
/// exactly so even string-equality comparisons behave deterministically.
fn report_to_python_dict(r: &SignalReport) -> Value {
    let mut interaction = Map::new();
    interaction.insert(
        "misalignment".to_string(),
        signal_group_to_python(&r.interaction.misalignment),
    );
    interaction.insert(
        "stagnation".to_string(),
        signal_group_to_python(&r.interaction.stagnation),
    );
    interaction.insert(
        "disengagement".to_string(),
        signal_group_to_python(&r.interaction.disengagement),
    );
    interaction.insert(
        "satisfaction".to_string(),
        signal_group_to_python(&r.interaction.satisfaction),
    );

    let mut execution = Map::new();
    execution.insert(
        "failure".to_string(),
        signal_group_to_python(&r.execution.failure),
    );
    execution.insert(
        "loops".to_string(),
        signal_group_to_python(&r.execution.loops),
    );

    let mut environment = Map::new();
    environment.insert(
        "exhaustion".to_string(),
        signal_group_to_python(&r.environment.exhaustion),
    );

    json!({
        "interaction_signals": Value::Object(interaction),
        "execution_signals": Value::Object(execution),
        "environment_signals": Value::Object(environment),
        "overall_quality": r.overall_quality.as_str(),
        "summary": r.summary,
    })
}

fn signal_group_to_python(g: &SignalGroup) -> Value {
    let signals: Vec<Value> = g
        .signals
        .iter()
        .map(|s| {
            json!({
                "signal_type": s.signal_type.as_str(),
                "message_index": s.message_index,
                "snippet": s.snippet,
                "confidence": s.confidence,
                "metadata": s.metadata,
            })
        })
        .collect();

    json!({
        "category": g.category,
        "count": g.count,
        "severity": g.severity,
        "signals": signals,
    })
}
