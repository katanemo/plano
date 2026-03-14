---
name: new-provider
description: Add a new LLM provider to hermesllm. Use when integrating a new AI provider.
disable-model-invocation: true
user-invocable: true
---

Add a new LLM provider to hermesllm. The user will provide the provider name as $ARGUMENTS.

1. Add a new variant to `ProviderId` enum in `crates/hermesllm/src/providers/id.rs`
2. Implement string parsing in the `TryFrom<&str>` impl for the new provider
3. If the provider uses a non-OpenAI API format, create request/response types in `crates/hermesllm/src/apis/`
4. Add variant to `ProviderRequestType` and `ProviderResponseType` enums and update all match arms
5. Add model list to `crates/hermesllm/src/providers/provider_models.yaml`
6. Update `SupportedUpstreamAPIs` mapping if needed

After making changes, run `cd crates && cargo test --lib` to verify everything compiles and tests pass.
