//! `signals-equivalence` — validates that the incremental analysis path
//! (`SignalAnalyzer::analyze_step`) added in PR 1032 produces the same final
//! report as the existing batch path (`SignalAnalyzer::analyze_sharegpt`),
//! on real sampled conversations rather than the synthetic fixtures already
//! covered by `analyzer.rs`'s unit tests.
//!
//! This is the Rust-side counterpart of the Python equivalence test run
//! against `lmsys/lmsys-chat-1m` referenced in the PR thread (65.8s -> 1.8s,
//! same report/signal counts).
//!
//! Reads JSONL from stdin, one conversation per line, same shape as
//! `signals-replay`'s input:
//! ```json
//! {"id": "convo-42", "messages": [{"from": "human", "value": "..."}, ...]}
//! ```
//!
//! For each conversation: runs `analyze_sharegpt` once (batch) and
//! `analyze_step` once per message over the growing prefix (incremental,
//! mirroring how `get_message_reports` drives it), times both, and compares
//! the two final `SignalReport`s field-by-field (per-category signal counts,
//! severities, `overall_quality`).
//!
//! Emits one result line per input line to stdout, and a final summary line
//! (`"summary"` object) after all input is consumed.

use std::io::{self, BufRead, BufWriter, Write};
use std::time::Instant;

use serde::Deserialize;
use serde_json::{json, Value};

use brightstaff::signals::analyzer::ShareGptMessage;
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

#[derive(Default)]
struct Totals {
    conversations: usize,
    matches: usize,
    mismatches: usize,
    errors: usize,
    batch_nanos: u128,
    incremental_nanos: u128,
}

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    let analyzer = SignalAnalyzer::default();
    let mut totals = Totals::default();

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
        let result = process_line(&analyzer, trimmed, &mut totals);
        if let Err(e) = writeln!(out, "{result}") {
            eprintln!("write error: {e}");
            std::process::exit(1);
        }
    }

    let summary = json!({
        "summary": {
            "conversations": totals.conversations,
            "matches": totals.matches,
            "mismatches": totals.mismatches,
            "errors": totals.errors,
            "batch_repeated_total_ms": totals.batch_nanos as f64 / 1_000_000.0,
            "incremental_total_ms": totals.incremental_nanos as f64 / 1_000_000.0,
            "speedup_x": if totals.incremental_nanos > 0 {
                totals.batch_nanos as f64 / totals.incremental_nanos as f64
            } else {
                0.0
            },
        }
    });
    let _ = writeln!(out, "{summary}");
    let _ = out.flush();
}

fn process_line(analyzer: &SignalAnalyzer, line: &str, totals: &mut Totals) -> Value {
    let parsed: InputLine = match serde_json::from_str(line) {
        Ok(p) => p,
        Err(e) => {
            totals.errors += 1;
            return json!({ "id": Value::Null, "error": format!("input parse: {e}") });
        }
    };
    let id = parsed.id.clone();

    let view: Vec<ShareGptMessage<'_>> = parsed
        .messages
        .iter()
        .map(|m| ShareGptMessage {
            from: m.from.as_str(),
            value: m.value.as_str(),
        })
        .collect();

    if view.is_empty() {
        totals.errors += 1;
        return json!({ "id": id, "error": "empty conversation" });
    }

    // Correctness check: one-shot batch call vs. the incremental path's final
    // carried-forward report. This is what must be *equivalent*.
    let batch_report = analyzer.analyze_sharegpt(&view);

    // Latency check: mirrors the actual production comparison from the PR
    // thread (and the Python-side benchmark) — the "current approach" of
    // re-running full batch analysis on the growing prefix after every new
    // message (what `streaming.rs::on_complete` does today, O(n^2) over a
    // session) vs. the "new approach" of calling `analyze_step` once per
    // message with the previous report carried forward (O(n)). A single
    // one-shot `analyze_sharegpt` call on the complete conversation is not
    // the right baseline for that comparison.
    let batch_repeated_start = Instant::now();
    for i in 0..view.len() {
        let _ = analyzer.analyze_sharegpt(&view[..=i]);
    }
    let batch_repeated_elapsed = batch_repeated_start.elapsed();

    let incr_start = Instant::now();
    let mut prev: Option<SignalReport> = None;
    for i in 0..view.len() {
        let (_entry, report) = analyzer.analyze_step(&view[..=i], prev.as_ref());
        prev = Some(report);
    }
    let incr_elapsed = incr_start.elapsed();
    let incremental_report = prev.expect("non-empty conversation produces a report");

    totals.conversations += 1;
    totals.batch_nanos += batch_repeated_elapsed.as_nanos();
    totals.incremental_nanos += incr_elapsed.as_nanos();

    let diffs = diff_reports(&batch_report, &incremental_report);
    if diffs.is_empty() {
        totals.matches += 1;
    } else {
        totals.mismatches += 1;
    }

    json!({
        "id": id,
        "num_messages": view.len(),
        "batch_repeated_us": batch_repeated_elapsed.as_micros(),
        "incremental_us": incr_elapsed.as_micros(),
        "equivalent": diffs.is_empty(),
        "diffs": diffs,
        "batch_quality": batch_report.overall_quality.as_str(),
        "incremental_quality": incremental_report.overall_quality.as_str(),
    })
}

/// Compare the two reports on the fields that matter for "same report":
/// per-category signal counts + severity, and overall quality. Signal
/// ordering/snippet text is allowed to differ trivially; counts and severity
/// are the equivalence bar the PR claims to meet.
fn diff_reports(batch: &SignalReport, incremental: &SignalReport) -> Vec<String> {
    let mut diffs = Vec::new();

    diff_group(
        "misalignment",
        &batch.interaction.misalignment,
        &incremental.interaction.misalignment,
        &mut diffs,
    );
    diff_group(
        "stagnation",
        &batch.interaction.stagnation,
        &incremental.interaction.stagnation,
        &mut diffs,
    );
    diff_group(
        "disengagement",
        &batch.interaction.disengagement,
        &incremental.interaction.disengagement,
        &mut diffs,
    );
    diff_group(
        "satisfaction",
        &batch.interaction.satisfaction,
        &incremental.interaction.satisfaction,
        &mut diffs,
    );
    diff_group(
        "failure",
        &batch.execution.failure,
        &incremental.execution.failure,
        &mut diffs,
    );
    diff_group(
        "loops",
        &batch.execution.loops,
        &incremental.execution.loops,
        &mut diffs,
    );
    diff_group(
        "exhaustion",
        &batch.environment.exhaustion,
        &incremental.environment.exhaustion,
        &mut diffs,
    );

    if batch.overall_quality.as_str() != incremental.overall_quality.as_str() {
        diffs.push(format!(
            "overall_quality: batch={} incremental={}",
            batch.overall_quality.as_str(),
            incremental.overall_quality.as_str()
        ));
    }

    diffs
}

fn diff_group(
    name: &str,
    batch: &SignalGroup,
    incremental: &SignalGroup,
    diffs: &mut Vec<String>,
) {
    if batch.count != incremental.count {
        diffs.push(format!(
            "{name}.count: batch={} incremental={}",
            batch.count, incremental.count
        ));
    }
    if batch.severity != incremental.severity {
        diffs.push(format!(
            "{name}.severity: batch={} incremental={}",
            batch.severity, incremental.severity
        ));
    }
}
