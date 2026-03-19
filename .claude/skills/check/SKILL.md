---
name: check
description: Run Rust fmt, clippy, and unit tests. Use after making Rust code changes.
---

Run all local checks in order:

1. `cd crates && cargo fmt --all -- --check` — if formatting fails, run `cargo fmt --all` to fix it
2. `cd crates && cargo clippy --locked --all-targets --all-features -- -D warnings` — fix any warnings
3. `cd crates && cargo test --lib` — ensure all unit tests pass

Report a summary of what passed/failed.
