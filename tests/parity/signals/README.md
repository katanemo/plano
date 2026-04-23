# Signals Parity Harness

Validates that `crates/brightstaff/src/signals/` (Rust port) produces the same
`SignalReport` as the Python reference at <https://github.com/katanemo/signals>
on a fixed sample of `lmsys/lmsys-chat-1m` conversations.

This harness is **not** part of normal CI. It downloads several GB and is run
on demand to gate releases of the signals subsystem (or to investigate
regressions reported in production).

## What gets compared

For each conversation, both analyzers emit a `SignalReport`. The comparator
classifies any divergence into three tiers:

| Tier | Field                                          | Action on divergence |
|------|------------------------------------------------|----------------------|
| A    | set of `SignalType` present, per-type counts, `overall_quality` | Fail the run |
| B    | per-instance `message_index`, instance counts per type          | Log + collect, do not fail |
| C    | metadata, snippet text, summary                                  | Information only |

Quality buckets are compared by string (`excellent` / `good` / ...).

## What this harness does *not* cover

`lmsys-chat-1m` is plain user/assistant chat. It exercises the **interaction**
layer well (misalignment, stagnation, disengagement, satisfaction) but does
**not** exercise:

- `execution.failure.*`
- `execution.loops.*`
- `environment.exhaustion.*`

Those signals require `function_call` / `observation` ShareGPT roles. They are
covered by the Rust unit tests and the Python repo's own test fixtures, both
of which run on every PR. A synthetic tool-trace dataset for full coverage is
deferred to a follow-up.

## One-time setup

```bash
# 1. Build the Rust replay binary.
cd ../../../crates && cargo build --release -p brightstaff --bin signals_replay

# 2. Set up the Python environment for the harness driver.
cd ../tests/parity/signals
python3 -m venv .venv && source .venv/bin/activate
pip install -r requirements.txt

# 3. Install the Python signals reference.
#    Either point at a local checkout:
pip install -e /path/to/signals
#    or pull from git:
pip install 'signals @ git+https://github.com/katanemo/signals@<sha>'
```

## Running

```bash
source .venv/bin/activate

python run_parity.py \
    --num-samples 2000 \
    --seed 42 \
    --dataset-revision <hf-dataset-revision-sha> \
    --rust-binary ../../../crates/target/release/signals_replay \
    --output-dir out/

python compare.py --output-dir out/
```

`run_parity.py` will:

1. Download `lmsys/lmsys-chat-1m` (cached in `~/.cache/huggingface`).
2. Pick `--num-samples` rows under `--seed`.
3. Convert each to ShareGPT, write `out/conversations.jsonl`.
4. Run the Rust binary as a subprocess → `out/rust_reports.jsonl`.
5. Run the Python analyzer in-process → `out/python_reports.jsonl`.

`compare.py` reads both report files and writes:

- `out/diffs.jsonl`     — one record per mismatched conversation, with tier + structural diff
- `out/metrics.json`    — agreement %, per-`SignalType` confusion matrix, quality-bucket confusion matrix
- `out/summary.md`      — human-readable PR-ready report

Exit code is non-zero iff any Tier-A divergence is observed.

## Reproducibility

Every run pins:

- `dataset_revision` — the HF dataset commit
- `seed` — RNG seed for sampling
- `signals_python_version` — `pip show signals` version
- `plano_git_sha` — `git rev-parse HEAD` of this repo
- `signals_replay_binary_sha256` — the hash of the Rust bin

All are stamped into `metrics.json`.
