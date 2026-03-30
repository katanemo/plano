---
name: build-cli
description: Build and install the Rust CLI (planoai). Use after making changes to plano-cli code to install locally.
---

1. `cd crates && cargo build --release -p plano-cli` — build the CLI binary
2. Verify the installation: `./crates/target/release/planoai --help`

If the build fails, diagnose and fix the issues.
