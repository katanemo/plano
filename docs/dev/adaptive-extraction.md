# Adaptive Extraction

## Overview

The WASM filter uses an `ExtractionLevel` enum to explicitly select the parsing strategy for each request. This replaces implicit `if firewall_mode` branching with a pipeline-aware decision made once during request header processing.

## ExtractionLevel Enum

```rust
#[derive(Clone, Copy, PartialEq)]
enum ExtractionLevel {
    UsageOnly,    // byte scan: model, stream flag, input/output tokens
    FullContent,  // serde: messages, tools, metadata, tokens from parsed struct
}
```

### Decision Logic

```rust
impl ExtractionLevel {
    fn from_pipeline(firewall_mode: bool) -> Self {
        if firewall_mode {
            ExtractionLevel::UsageOnly
        } else {
            ExtractionLevel::FullContent
        }
    }
}
```

The enum is set once in `on_http_request_headers` and controls both request and response body processing:

- **`UsageOnly`** (firewall mode): byte-scan for `"model"`, `"stream"`, and token counts. Zero-alloc backward search. No full JSON parse.
- **`FullContent`** (managed mode): full serde deserialization into typed structs. Extracts messages, tools, metadata. Token counts come from the parsed `ProviderResponse` struct.

Key invariant: `FullContent` never also byte-scans. `UsageOnly` never also serde-parses. The two paths are mutually exclusive.

### Request Path

| ExtractionLevel | What happens |
|---|---|
| `UsageOnly` | Byte-scan for `"model"` and `"stream"` fields. DLP scan if enabled. Inject `stream_options` for OpenAI-compatible streaming. |
| `FullContent` | Deserialize into `ProviderRequestType`. Model resolution, ratelimit check, format conversion for upstream provider. |

### Response Path

| ExtractionLevel | What happens |
|---|---|
| `UsageOnly` | Byte-scan each SSE `data:` line (streaming) or full body (non-streaming) for token fields using `UsageFields`. Fire-and-forget usage callout. |
| `FullContent` | `SseChunkProcessor` with incomplete-event buffering (streaming) or `ProviderResponse` deserialization (non-streaming). Token estimation from `content.len() / 4`. |

### Streaming Details

- **`UsageOnly` streaming**: Each SSE chunk is scanned independently. The last chunk with usage fields overwrites previous values (correct — OpenAI sends final usage in the last chunk). No incomplete-line buffering needed since token fields always appear in complete chunks.
- **`FullContent` streaming**: `SseChunkProcessor` handles incomplete SSE events across chunk boundaries with buffering.

## Benchmark Results

Benchmarks comparing three token extraction approaches on OpenAI-format responses (`crates/llm_gateway/benches/extraction_bench.rs`):

| Size | Byte Scan | Regex | Serde |
|---|---|---|---|
| Small (~1KB) | ~187 ns | ~387 ns (2.1x) | ~1,530 ns (8.2x) |
| Medium (~10KB) | ~201 ns | ~616 ns (3.1x) | ~2,980 ns (14.8x) |
| Large (~100KB) | ~170 ns | ~6,560 ns (38.6x) | ~11,020 ns (64.8x) |
| Streaming chunk | ~152 ns | ~280 ns (1.8x) | ~950 ns (6.3x) |

Byte scanning is constant-time because it searches backward from the end — `usage` is always near the tail of the response. Regex and serde scale linearly with payload size.

Run benchmarks: `cd crates && cargo bench -p llm_gateway`

## Egress IP Isolation

Different API keys can route through different outbound IP addresses via Envoy cluster `bind_config`.

### Architecture

```
Key A (egress: default)  → cluster "openai"          → 0.0.0.0       → api.openai.com
Key B (egress: isolated) → cluster "openai-isolated"  → 10.0.1.100   → api.openai.com
```

### How It Works

1. **Config**: `egress_ips` array in plano config defines named IP addresses
2. **DB**: `registered_api_keys.egress_ip` column (default: `"default"`)
3. **Auth check**: builds cluster name as `{provider}-{egress_name}` (or just `{provider}` if default)
4. **Envoy template**: generates cross-product clusters with `bind_config.source_address`

### Config Example

```yaml
egress_ips:
  - name: default
    address: "0.0.0.0"
  - name: isolated
    address: "10.0.1.100"
```

### Cluster Generation

For each non-default egress IP, the Envoy template generates a cluster per built-in provider:

```yaml
- name: openai-isolated
  bind_config:
    source_address:
      address: "10.0.1.100"
      port_value: 0
  # ... same upstream config as "openai" cluster ...
```

### Infrastructure Requirement

The host must have multiple IP addresses configured (secondary network interfaces or IP aliases). In Docker, this requires `--network host` or multiple network attachments.

### API

Register a key with egress IP:

```bash
curl -X POST /api/v1/projects/{id}/api-keys \
  -d '{"api_key": "sk-...", "provider": "openai", "upstream_url": "https://api.openai.com", "egress_ip": "isolated"}'
```

## Key Files

| File | What changed |
|---|---|
| `crates/llm_gateway/src/stream_context.rs` | `ExtractionLevel` enum, replaces `if firewall_mode` branching |
| `crates/llm_gateway/benches/extraction_bench.rs` | Criterion benchmarks: byte scan vs regex vs serde |
| `crates/brightstaff/src/registry.rs` | `egress_ip` field in `RegisteredKeyInfo` |
| `crates/brightstaff/src/handlers/auth_check.rs` | Cluster name includes egress suffix |
| `crates/brightstaff/src/handlers/management.rs` | `egress_ip` in register API key request |
| `crates/brightstaff/src/db/queries.rs` | `egress_ip` column in INSERT query |
| `config/plano_config_schema.yaml` | `egress_ips` array schema |
| `config/envoy.template.yaml` | Cross-product cluster generation with `bind_config` |
| `cli/planoai/config_generator.py` | Pass `egress_ips` to template context |
| `migrations/003_egress_ip.sql` | ALTER TABLE for `egress_ip` column |
