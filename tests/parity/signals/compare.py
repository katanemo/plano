#!/usr/bin/env python3
"""
Diff Rust vs Python signal reports produced by run_parity.py.

See README.md for the tier definitions. Exits non-zero iff any Tier-A
divergence is found.
"""
from __future__ import annotations

import argparse
import json
import sys
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Dict, List, Tuple

CATEGORIES_BY_LAYER = {
    "interaction_signals": ["misalignment", "stagnation", "disengagement", "satisfaction"],
    "execution_signals": ["failure", "loops"],
    "environment_signals": ["exhaustion"],
}


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--output-dir", type=Path, default=Path("out"))
    return p.parse_args()


def load_jsonl(path: Path) -> Dict[str, Dict[str, Any]]:
    """Load a JSONL file keyed by `id`. Lines with errors are still indexed."""
    out: Dict[str, Dict[str, Any]] = {}
    with path.open() as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            obj = json.loads(line)
            out[str(obj.get("id"))] = obj
    return out


def per_type_counts(report: Dict[str, Any]) -> Dict[str, int]:
    """Return {signal_type: count} across all groups in a report dict."""
    counts: Counter[str] = Counter()
    for layer in CATEGORIES_BY_LAYER:
        groups = report.get(layer, {}) or {}
        for category in CATEGORIES_BY_LAYER[layer]:
            group = groups.get(category)
            if not group:
                continue
            for sig in group.get("signals", []) or []:
                counts[sig["signal_type"]] += 1
    return dict(counts)


def per_type_indices(report: Dict[str, Any]) -> Dict[str, List[int]]:
    out: Dict[str, List[int]] = defaultdict(list)
    for layer in CATEGORIES_BY_LAYER:
        groups = report.get(layer, {}) or {}
        for category in CATEGORIES_BY_LAYER[layer]:
            group = groups.get(category)
            if not group:
                continue
            for sig in group.get("signals", []) or []:
                out[sig["signal_type"]].append(sig.get("message_index"))
    for k in out:
        out[k].sort(key=lambda x: (x is None, x))
    return dict(out)


def diff_counts(
    a: Dict[str, int], b: Dict[str, int]
) -> List[Tuple[str, int, int]]:
    """Return [(signal_type, a_count, b_count)] for entries that differ."""
    keys = set(a) | set(b)
    out = []
    for k in sorted(keys):
        ac = a.get(k, 0)
        bc = b.get(k, 0)
        if ac != bc:
            out.append((k, ac, bc))
    return out


def diff_indices(
    a: Dict[str, List[int]], b: Dict[str, List[int]]
) -> List[Tuple[str, List[int], List[int]]]:
    keys = set(a) | set(b)
    out = []
    for k in sorted(keys):
        ai = a.get(k, [])
        bi = b.get(k, [])
        if ai != bi:
            out.append((k, ai, bi))
    return out


def compare_one(
    convo_id: str, py: Dict[str, Any], rust: Dict[str, Any]
) -> Dict[str, Any] | None:
    """Compare a single conversation. Return diff record, or None if identical."""
    if "error" in py or "error" in rust:
        return {
            "id": convo_id,
            "tier": "A",
            "kind": "error_in_runner",
            "python_error": py.get("error"),
            "rust_error": rust.get("error"),
        }
    py_report = py["report"]
    rust_report = rust["report"]

    py_counts = per_type_counts(py_report)
    rust_counts = per_type_counts(rust_report)
    count_diff = diff_counts(py_counts, rust_counts)

    py_quality = py_report.get("overall_quality")
    rust_quality = rust_report.get("overall_quality")
    quality_mismatch = py_quality != rust_quality

    if count_diff or quality_mismatch:
        return {
            "id": convo_id,
            "tier": "A",
            "kind": "signal_or_quality_mismatch",
            "quality": {"python": py_quality, "rust": rust_quality},
            "count_diff": [
                {"signal_type": st, "python": pc, "rust": rc}
                for (st, pc, rc) in count_diff
            ],
        }

    py_idx = per_type_indices(py_report)
    rust_idx = per_type_indices(rust_report)
    idx_diff = diff_indices(py_idx, rust_idx)
    if idx_diff:
        return {
            "id": convo_id,
            "tier": "B",
            "kind": "instance_index_mismatch",
            "diff": [
                {"signal_type": st, "python_indices": pi, "rust_indices": ri}
                for (st, pi, ri) in idx_diff
            ],
        }

    return None


def confusion_matrix(
    pairs: List[Tuple[str, str]], labels: List[str]
) -> Dict[str, Dict[str, int]]:
    cm: Dict[str, Dict[str, int]] = {a: {b: 0 for b in labels} for a in labels}
    for py, rust in pairs:
        if py not in cm:
            cm[py] = {b: 0 for b in labels}
        if rust not in cm[py]:
            cm[py][rust] = 0
        cm[py][rust] += 1
    return cm


def main() -> int:
    args = parse_args()
    out_dir = args.output_dir

    py_reports = load_jsonl(out_dir / "python_reports.jsonl")
    rust_reports = load_jsonl(out_dir / "rust_reports.jsonl")

    common_ids = sorted(set(py_reports) & set(rust_reports))
    only_py = sorted(set(py_reports) - set(rust_reports))
    only_rust = sorted(set(rust_reports) - set(py_reports))

    diffs: List[Dict[str, Any]] = []
    quality_pairs: List[Tuple[str, str]] = []
    per_type_total = Counter()
    per_type_disagree = Counter()

    tier_a = 0
    tier_b = 0
    for cid in common_ids:
        d = compare_one(cid, py_reports[cid], rust_reports[cid])
        if d is None:
            quality_pairs.append(
                (
                    py_reports[cid]["report"]["overall_quality"],
                    rust_reports[cid]["report"]["overall_quality"],
                )
            )
            for st, _ in per_type_counts(py_reports[cid]["report"]).items():
                per_type_total[st] += 1
        else:
            diffs.append(d)
            if d["tier"] == "A":
                tier_a += 1
            elif d["tier"] == "B":
                tier_b += 1
            if "report" in py_reports[cid] and "report" in rust_reports[cid]:
                quality_pairs.append(
                    (
                        py_reports[cid]["report"].get("overall_quality", "?"),
                        rust_reports[cid]["report"].get("overall_quality", "?"),
                    )
                )
            for cd in d.get("count_diff", []) or []:
                per_type_disagree[cd["signal_type"]] += 1
                per_type_total[cd["signal_type"]] += 1

    n_total = len(common_ids)
    n_match = n_total - len(diffs)
    agreement = (n_match / n_total) if n_total else 0.0

    quality_labels = ["excellent", "good", "neutral", "poor", "severe"]
    cm = confusion_matrix(quality_pairs, quality_labels)

    metrics = {
        "n_python_reports": len(py_reports),
        "n_rust_reports": len(rust_reports),
        "n_common": n_total,
        "n_only_python": len(only_py),
        "n_only_rust": len(only_rust),
        "n_full_match": n_match,
        "agreement_pct": round(100.0 * agreement, 4),
        "tier_a_divergences": tier_a,
        "tier_b_divergences": tier_b,
        "quality_confusion_matrix": cm,
        "per_signal_type_total": dict(per_type_total),
        "per_signal_type_disagree": dict(per_type_disagree),
    }

    # Pull in run metadata if present.
    rm_path = out_dir / "run_metadata.json"
    if rm_path.exists():
        metrics["run_metadata"] = json.loads(rm_path.read_text())

    (out_dir / "metrics.json").write_text(json.dumps(metrics, indent=2))
    with (out_dir / "diffs.jsonl").open("w") as f:
        for d in diffs:
            f.write(json.dumps(d, ensure_ascii=False))
            f.write("\n")

    write_summary_md(out_dir / "summary.md", metrics, diffs[:20])

    print(json.dumps({k: v for k, v in metrics.items() if k != "quality_confusion_matrix"}, indent=2))
    print(f"\ndiffs: {out_dir / 'diffs.jsonl'}  metrics: {out_dir / 'metrics.json'}")
    print(f"summary: {out_dir / 'summary.md'}")

    if tier_a > 0:
        print(f"\nFAIL: {tier_a} Tier-A divergence(s) detected.", file=sys.stderr)
        return 1
    return 0


def write_summary_md(path: Path, metrics: Dict[str, Any], sample_diffs: List[Dict[str, Any]]) -> None:
    lines: List[str] = []
    lines.append("# Signals Parity Report")
    lines.append("")
    rm = metrics.get("run_metadata", {})
    if rm:
        lines.append("## Run metadata")
        lines.append("")
        for k in (
            "dataset_name",
            "dataset_revision",
            "seed",
            "num_samples_actual",
            "plano_git_sha",
            "signals_python_version",
            "rust_binary_sha256",
        ):
            if k in rm:
                lines.append(f"- **{k}**: `{rm[k]}`")
        lines.append("")

    lines.append("## Summary")
    lines.append("")
    lines.append(f"- Conversations compared: **{metrics['n_common']}**")
    lines.append(f"- Full matches: **{metrics['n_full_match']}**")
    lines.append(f"- Agreement: **{metrics['agreement_pct']}%**")
    lines.append(f"- Tier-A divergences: **{metrics['tier_a_divergences']}**")
    lines.append(f"- Tier-B divergences: **{metrics['tier_b_divergences']}**")
    lines.append("")

    lines.append("## Per-signal-type disagreement")
    lines.append("")
    lines.append("| Signal type | Total reports | Disagreements |")
    lines.append("|---|---:|---:|")
    totals = metrics["per_signal_type_total"]
    disagrees = metrics["per_signal_type_disagree"]
    for k in sorted(set(totals) | set(disagrees)):
        lines.append(f"| `{k}` | {totals.get(k, 0)} | {disagrees.get(k, 0)} |")
    lines.append("")

    lines.append("## Quality bucket confusion matrix (rows = python, cols = rust)")
    lines.append("")
    cm = metrics["quality_confusion_matrix"]
    labels = list(cm.keys())
    lines.append("| | " + " | ".join(labels) + " |")
    lines.append("|---|" + "|".join(["---:"] * len(labels)) + "|")
    for r in labels:
        lines.append(f"| {r} | " + " | ".join(str(cm[r].get(c, 0)) for c in labels) + " |")
    lines.append("")

    if sample_diffs:
        lines.append("## Sample divergences (first 20)")
        lines.append("")
        for d in sample_diffs:
            lines.append(f"### `{d['id']}` — tier {d['tier']} — {d['kind']}")
            lines.append("")
            lines.append("```json")
            lines.append(json.dumps(d, indent=2))
            lines.append("```")
            lines.append("")

    path.write_text("\n".join(lines))


if __name__ == "__main__":
    sys.exit(main())
