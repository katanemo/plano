#!/usr/bin/env python3
"""
Local smoke test for the parity harness — runs both runners on a tiny
hand-picked set of conversations without touching the lmsys dataset.

Run from this directory:
    python _smoke_test.py --rust-binary <path>
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

from signals.analyzer import SignalAnalyzer

SAMPLES = [
    {
        "id": "smoke-gratitude",
        "messages": [
            {"from": "human", "value": "What is the weather in Istanbul?"},
            {"from": "gpt", "value": "Istanbul is 14C and partly cloudy."},
            {"from": "human", "value": "That worked, exactly what I needed. Thanks!"},
        ],
    },
    {
        "id": "smoke-escalation",
        "messages": [
            {"from": "human", "value": "This isn't helpful at all"},
            {"from": "gpt", "value": "I'm sorry, can you tell me more?"},
            {"from": "human", "value": "Get me a human, this is useless"},
        ],
    },
    {
        "id": "smoke-correction",
        "messages": [
            {"from": "human", "value": "Book me a flight to NYC for tomorrow"},
            {"from": "gpt", "value": "Sure, here are flights to NYC for Friday."},
            {"from": "human", "value": "No, I meant flights for Saturday, not tomorrow"},
        ],
    },
    {
        "id": "smoke-clean",
        "messages": [
            {"from": "human", "value": "Hi"},
            {"from": "gpt", "value": "Hello, how can I help?"},
        ],
    },
    {
        "id": "smoke-rephrase",
        "messages": [
            {"from": "human", "value": "Can you summarize the news please"},
            {"from": "gpt", "value": "Sure, here is a summary."},
            {"from": "human", "value": "Could you please summarize the news"},
        ],
    },
]


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--rust-binary", required=True, type=Path)
    args = p.parse_args()

    out_dir = Path("out_smoke")
    out_dir.mkdir(exist_ok=True)
    conv_path = out_dir / "conversations.jsonl"
    rust_path = out_dir / "rust_reports.jsonl"
    py_path = out_dir / "python_reports.jsonl"

    with conv_path.open("w") as f:
        for s in SAMPLES:
            f.write(json.dumps(s) + "\n")

    with conv_path.open("rb") as fin, rust_path.open("wb") as fout:
        proc = subprocess.run([str(args.rust_binary)], stdin=fin, stdout=fout, stderr=subprocess.PIPE)
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr.decode("utf-8", errors="replace"))
        return 2

    analyzer = SignalAnalyzer()
    with conv_path.open() as fin, py_path.open("w") as fout:
        for line in fin:
            obj = json.loads(line)
            r = analyzer.analyze(obj["messages"])
            fout.write(json.dumps({"id": obj["id"], "report": r.to_dict()}) + "\n")

    rc = subprocess.call(
        [sys.executable, "compare.py", "--output-dir", str(out_dir)],
    )
    return rc


if __name__ == "__main__":
    sys.exit(main())
