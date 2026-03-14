---
name: build-wasm
description: Build the WASM plugins for Envoy. Use when WASM plugin code changes.
---

Build the WASM plugins:

```
cd crates && cargo build --release --target=wasm32-wasip1 -p llm_gateway -p prompt_gateway
```

If the build fails, diagnose and fix the errors.
