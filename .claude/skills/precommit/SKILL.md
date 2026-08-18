---
name: precommit
description: Run pre-commit hooks and fix any failures. Use when the user asks to run pre-commit, fix CI, fix clippy, fix formatting, or when preparing code for a PR.
---

# Fix Pre-commit Failures

Run pre-commit and fix all failures before committing or pushing.

## Run it

```bash
cd /Users/ahafeez/dev/plano && pre-commit run --all-files
```

Requires `["all"]` sandbox permissions (jemalloc build needs temp dir access).

## Hooks and how to fix each

### check-yaml
YAML syntax errors. Fix the YAML file directly. Excludes `config/envoy.template*`.

### end-of-file-fixer / trailing-whitespace
Auto-fixed by the hook. Just re-stage the files and re-run.

### cargo-fmt
```bash
cd crates && cargo fmt --all
```
Then re-stage changed files.

### cargo-clippy
Runs `cargo clippy --locked --all-targets --all-features -- -D warnings`.

Common fixes:
- `collapsible_match`: Collapse `if` inside a `match` arm into a guard clause. Add a fallthrough arm if the guard makes the match non-exhaustive.
- `dead_code`: Remove unused fields/functions, or add `#[allow(dead_code)]` if intentional.
- `unused_imports`: Remove the import.
- `clippy::too_many_arguments`: Add `#[allow(clippy::too_many_arguments)]`.

**Important**: Clippy runs on the full workspace including `hermesllm`, `common`, `llm_gateway`, `prompt_gateway` — not just `brightstaff`. Pre-existing warnings in other crates will fail CI too. Fix them.

### cargo-test
```bash
cd crates && cargo test --lib
```
Fix failing tests. If a test fails due to your change, update the test. If it's pre-existing, still fix it.

### gitleaks
Hardcoded secrets detected. Remove the secret, use env vars instead.

### black
Python formatting. Auto-fixes on first run. Re-stage and re-run.

## Workflow

1. Run `pre-commit run --all-files`
2. For each failure: fix the issue
3. Stage fixes with `git add`
4. Run `pre-commit run --all-files` again until all pass
5. Then commit
