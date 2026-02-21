# Byte-Scan Parsing for Token Extraction

## Problem

WASM filters in Envoy have limited memory and must process LLM responses that can exceed 100KB. We need to extract exactly two integers — input tokens and output tokens — from the response body. Allocating the entire response as a parsed JSON tree is wasteful when we only need two small fields buried in the `usage` object.

## Options Considered

| Approach | Allocation | CPU | Correctness |
|---|---|---|---|
| `serde_json::Value` (full parse) | Entire response in memory as a tree | O(n) parse + tree traversal | Correct |
| Typed struct with `#[serde(skip)]` | Still buffers full input for serde | O(n) deserialization | Correct |
| Byte scan from end (`rfind`) | Zero extra allocation | O(small) — usage is near the end | Correct for all known providers |

## Decision

Byte scan with `UsageFields` const struct pattern.

The `usage` / `usageMetadata` object is always near the end of an LLM response. By scanning backwards with `rfind_bytes()`, we find the token fields in O(small) time without allocating a JSON tree.

## How It Works

### Provider Field Mappings

Each LLM provider uses different JSON field names for token counts. These are encoded as `const UsageFields` values:

```rust
struct UsageFields {
    input: &'static [u8],
    output: &'static [u8],
}

const OPENAI_FIELDS: UsageFields = UsageFields {
    input: b"\"prompt_tokens\"",
    output: b"\"completion_tokens\"",
};

const ANTHROPIC_FIELDS: UsageFields = UsageFields {
    input: b"\"input_tokens\"",
    output: b"\"output_tokens\"",
};

const GEMINI_FIELDS: UsageFields = UsageFields {
    input: b"\"promptTokenCount\"",
    output: b"\"candidatesTokenCount\"",
};
```

### Provider Resolution

```rust
fn for_provider(provider: &str) -> &'static UsageFields {
    match provider {
        "anthropic" => &ANTHROPIC_FIELDS,
        "gemini" | "google" => &GEMINI_FIELDS,
        _ => &OPENAI_FIELDS,  // Default for OpenAI-compatible providers
    }
}
```

### Scanning Functions

- **`rfind_bytes(haystack, needle)`** — backward search for a byte pattern
- **`parse_number_after_colon(bytes, start)`** — parses the integer after `: ` in `"field": 123`
- **`scan_field_i64(bytes, field)`** — combines rfind + parse to extract an integer value
- **`scan_field_str(bytes, field)`** — extracts a string value for fields like `"model"`
- **`scan_field_bool(bytes, field)`** — extracts a boolean value for fields like `"stream"`

### Usage Extraction

```rust
fn extract_usage_by_scan(&mut self, bytes: &[u8]) {
    let provider = self.xproxy_provider_hint.as_deref().unwrap_or("");
    let fields = UsageFields::for_provider(provider);

    if let Some(v) = scan_field_i64(bytes, fields.input) {
        self.firewall_input_tokens = v;
    }
    if let Some(v) = scan_field_i64(bytes, fields.output) {
        self.firewall_output_tokens = v;
    }
}
```

For streaming responses, each SSE `data:` line is scanned individually. For non-streaming, the full response body is scanned once.

## Safety: Why This Doesn't Match Inside Strings

The field patterns include the surrounding quotes (e.g., `b"\"prompt_tokens\""`). In JSON, a literal quote inside a string value is escaped as `\"`, which means the byte sequence `"prompt_tokens"` (with real unescaped quotes) cannot appear inside a JSON string value. This naturally prevents false matches against user content.

## Why DLP/PII Scanning Is Different

PII detection needs the actual text content of messages — it must read and analyze string values, not skip them. Byte scanning is the wrong tool for that problem because PII patterns (emails, phone numbers, SSNs) could appear anywhere in the text. DLP uses its own dedicated scanning path with regex-based detection.

## Adding a New Provider

1. Add a `const UsageFields` with the provider's field names:
   ```rust
   const NEWPROVIDER_FIELDS: UsageFields = UsageFields {
       input: b"\"input_token_count\"",
       output: b"\"output_token_count\"",
   };
   ```

2. Add a match arm in `for_provider()`:
   ```rust
   "newprovider" => &NEWPROVIDER_FIELDS,
   ```

That's it. No struct changes, no deserialization logic, no new dependencies.

## Extraction Level

Byte scanning is used when `ExtractionLevel::UsageOnly` is selected (firewall mode). Managed mode uses `ExtractionLevel::FullContent` which does full serde parsing instead. See [adaptive-extraction.md](adaptive-extraction.md) for the enum design and benchmark results.

## Code Location

`crates/llm_gateway/src/stream_context.rs` — scanning functions near the top, `extract_usage_by_scan()` in the `StreamContext` impl.
