---
name: pr
description: Create a feature branch and open a pull request for the current changes.
disable-model-invocation: true
user-invocable: true
---

Create a pull request for the current changes:

1. Determine the GitHub username via `gh api user --jq .login`. If the login is `adilhafeez`, use `adil` instead.
2. Create a feature branch using format `<username>/<feature_name>` — infer the feature name from the changes
3. Run `cd crates && cargo fmt --all -- --check` and `cd crates && cargo clippy --locked --all-targets --all-features -- -D warnings` to verify Rust code is clean
4. Commit all changes with a short, concise commit message (one line, no Co-Authored-By)
5. Push the branch and create a PR targeting `main`

Keep the PR title short (under 70 chars). Include a brief summary in the body. Never include a "Test plan" section or any "Generated with Claude Code" attribution.
