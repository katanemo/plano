#!/usr/bin/env python3
"""
Parity harness driver.

Samples conversations from `lmsys/lmsys-chat-1m`, runs both the Python
reference analyzer (in-process) and the Rust port (subprocess), writes both
reports to disk for `compare.py` to diff.

Usage:
    python run_parity.py \\
        --num-samples 2000 \\
        --seed 42 \\
        --dataset-revision <hf-revision-sha> \\
        --rust-binary ../../../crates/target/release/signals_replay \\
        --output-dir out/
"""

from __future__ import annotations

import argparse
import hashlib
import json
import random
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Dict, Iterator, List

try:
    import pyarrow.parquet as pq
    from huggingface_hub import hf_hub_download, list_repo_files
except ImportError:
    print(
        "error: install dependencies first: pip install -r requirements.txt",
        file=sys.stderr,
    )
    sys.exit(2)

try:
    from signals.analyzer import SignalAnalyzer
except ImportError:
    print(
        "error: the python `signals` package is not installed. "
        "install it from your local checkout: pip install -e /path/to/signals",
        file=sys.stderr,
    )
    sys.exit(2)

try:
    from tqdm import tqdm
except ImportError:

    def tqdm(it, **_kwargs):  # type: ignore[no-redef]
        return it


DATASET_NAME = "lmsys/lmsys-chat-1m"


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--num-samples", type=int, default=2000)
    p.add_argument("--seed", type=int, default=42)
    p.add_argument(
        "--dataset-revision",
        default=None,
        help="HF dataset revision to pin (default: latest, NOT recommended for reproducibility)",
    )
    p.add_argument(
        "--rust-binary",
        type=Path,
        required=True,
        help="path to the `signals_replay` binary built from crates/brightstaff",
    )
    p.add_argument(
        "--output-dir",
        type=Path,
        default=Path("out"),
        help="directory to write the conversations + both runners' outputs",
    )
    p.add_argument(
        "--max-conv-messages",
        type=int,
        default=200,
        help="drop conversations with more than this many messages (the analyzer "
        "truncates to last 100 anyway; this is a sanity cap on input parsing)",
    )
    return p.parse_args()


def lmsys_to_sharegpt(conversation: List[Dict[str, str]]) -> List[Dict[str, str]]:
    """Convert lmsys-chat-1m's `[{role, content}]` to ShareGPT's `[{from, value}]`.

    lmsys uses `user` / `assistant` (no tools, no system role in `conversation`).
    """
    out = []
    for m in conversation:
        role = m.get("role", "")
        content = m.get("content", "")
        if not isinstance(content, str):
            content = str(content) if content is not None else ""
        if role == "user":
            from_ = "human"
        elif role == "assistant":
            from_ = "gpt"
        else:
            # lmsys is human/assistant only; skip anything else defensively.
            continue
        out.append({"from": from_, "value": content})
    return out


def _list_parquet_files(revision: str | None) -> List[str]:
    """Return the list of parquet shard paths in the dataset repo."""
    files = list_repo_files(DATASET_NAME, repo_type="dataset", revision=revision)
    return sorted(f for f in files if f.endswith(".parquet"))


def _download_shards(paths: List[str], revision: str | None) -> List[Path]:
    """Download each parquet shard to the HF cache, return local paths."""
    local: List[Path] = []
    for rel in tqdm(paths, desc="downloading shards", unit="shard"):
        p = hf_hub_download(
            DATASET_NAME,
            filename=rel,
            repo_type="dataset",
            revision=revision,
        )
        local.append(Path(p))
    return local


def sample_conversations(
    *,
    num_samples: int,
    seed: int,
    revision: str | None,
    max_conv_messages: int,
) -> Iterator[Dict[str, Any]]:
    """Yield `num_samples` conversations sampled uniformly across the dataset.

    We bypass the `datasets` loader (which has a Python 3.14 pickle issue)
    and read the parquet shards directly via pyarrow.
    """
    print(
        f"listing {DATASET_NAME}"
        f"{' @ ' + revision if revision else ' (no revision pinned!)'}",
        file=sys.stderr,
    )
    shard_paths = _list_parquet_files(revision)
    if not shard_paths:
        raise SystemExit(f"no parquet shards found for {DATASET_NAME}")
    local_paths = _download_shards(shard_paths, revision)

    # Collect row counts without reading data.
    shard_row_counts: List[int] = []
    for p in local_paths:
        pf = pq.ParquetFile(str(p))
        shard_row_counts.append(pf.metadata.num_rows)
    total_rows = sum(shard_row_counts)
    print(
        f"dataset has {total_rows:,} rows across {len(local_paths)} shards",
        file=sys.stderr,
    )

    rng = random.Random(seed)
    global_indices = sorted(rng.sample(range(total_rows), num_samples))

    # Bucket indices by shard.
    by_shard: Dict[int, List[int]] = {}
    cumulative = 0
    shard_offsets = []
    for c in shard_row_counts:
        shard_offsets.append(cumulative)
        cumulative += c
    for gi in global_indices:
        # Find which shard this index belongs to.
        for si, off in enumerate(shard_offsets):
            if gi < off + shard_row_counts[si]:
                by_shard.setdefault(si, []).append(gi - off)
                break

    yielded = 0
    for si in sorted(by_shard.keys()):
        local_rows = by_shard[si]
        pf = pq.ParquetFile(str(local_paths[si]))
        table = pf.read(columns=["conversation"])
        conv_col = table.column("conversation")
        for local_idx in local_rows:
            raw = conv_col[local_idx].as_py()
            if not raw:
                continue
            conversation = raw if isinstance(raw, list) else raw.get("conversation", [])
            if len(conversation) > max_conv_messages:
                continue
            messages = lmsys_to_sharegpt(conversation)
            if not messages:
                continue
            global_idx = shard_offsets[si] + local_idx
            yield {
                "id": f"lmsys-{global_idx}",
                "messages": messages,
            }
            yielded += 1
    print(f"yielded {yielded} conversations after filtering", file=sys.stderr)


def write_conversations(out_path: Path, samples: Iterator[Dict[str, Any]]) -> int:
    n = 0
    with out_path.open("w") as f:
        for s in tqdm(samples, desc="sampling", unit="convo"):
            f.write(json.dumps(s, ensure_ascii=False))
            f.write("\n")
            n += 1
    return n


def run_rust(rust_binary: Path, conv_path: Path, out_path: Path) -> None:
    print(f"running rust analyzer: {rust_binary}", file=sys.stderr)
    t0 = time.monotonic()
    with conv_path.open("rb") as fin, out_path.open("wb") as fout:
        proc = subprocess.run(
            [str(rust_binary)],
            stdin=fin,
            stdout=fout,
            stderr=subprocess.PIPE,
            check=False,
        )
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr.decode("utf-8", errors="replace"))
        raise SystemExit(f"rust runner exited {proc.returncode}")
    elapsed = time.monotonic() - t0
    print(f"  rust runner: {elapsed:.1f}s", file=sys.stderr)


def run_python(conv_path: Path, out_path: Path) -> None:
    print("running python analyzer...", file=sys.stderr)
    t0 = time.monotonic()
    analyzer = SignalAnalyzer()
    with conv_path.open() as fin, out_path.open("w") as fout:
        for line in tqdm(fin, desc="python", unit="convo"):
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
                report = analyzer.analyze(obj["messages"])
                fout.write(
                    json.dumps(
                        {"id": obj["id"], "report": report.to_dict()},
                        ensure_ascii=False,
                    )
                )
            except Exception as e:
                fout.write(json.dumps({"id": obj.get("id"), "error": str(e)}))
            fout.write("\n")
    elapsed = time.monotonic() - t0
    print(f"  python runner: {elapsed:.1f}s", file=sys.stderr)


def stamp_metadata(args: argparse.Namespace, output_dir: Path, n_samples: int) -> None:
    """Write the input metadata so compare.py can include it in the report."""
    binary_sha = hashlib.sha256(args.rust_binary.read_bytes()).hexdigest()
    try:
        plano_sha = (
            subprocess.check_output(
                ["git", "rev-parse", "HEAD"], cwd=Path(__file__).parent
            )
            .decode()
            .strip()
        )
    except Exception:
        plano_sha = "unknown"
    try:
        signals_version = subprocess.check_output(
            [sys.executable, "-m", "pip", "show", "signals"]
        ).decode()
        signals_version = next(
            (
                l.split(":", 1)[1].strip()
                for l in signals_version.splitlines()
                if l.startswith("Version")
            ),
            "unknown",
        )
    except Exception:
        signals_version = "unknown"

    meta = {
        "dataset_name": DATASET_NAME,
        "dataset_revision": args.dataset_revision,
        "seed": args.seed,
        "num_samples_requested": args.num_samples,
        "num_samples_actual": n_samples,
        "rust_binary": str(args.rust_binary.resolve()),
        "rust_binary_sha256": binary_sha,
        "plano_git_sha": plano_sha,
        "signals_python_version": signals_version,
        "max_conv_messages": args.max_conv_messages,
    }
    (output_dir / "run_metadata.json").write_text(json.dumps(meta, indent=2))
    print(f"wrote {output_dir / 'run_metadata.json'}", file=sys.stderr)


def main() -> None:
    args = parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    if not args.rust_binary.exists():
        raise SystemExit(f"rust binary not found at {args.rust_binary}")

    conv_path = args.output_dir / "conversations.jsonl"
    rust_path = args.output_dir / "rust_reports.jsonl"
    py_path = args.output_dir / "python_reports.jsonl"

    samples = sample_conversations(
        num_samples=args.num_samples,
        seed=args.seed,
        revision=args.dataset_revision,
        max_conv_messages=args.max_conv_messages,
    )
    n = write_conversations(conv_path, samples)
    print(f"wrote {n} conversations to {conv_path}", file=sys.stderr)

    run_rust(args.rust_binary, conv_path, rust_path)
    run_python(conv_path, py_path)
    stamp_metadata(args, args.output_dir, n)
    print("done. now run: python compare.py --output-dir " + str(args.output_dir))


if __name__ == "__main__":
    main()
