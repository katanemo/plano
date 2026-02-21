//! Benchmarks comparing three token extraction strategies:
//! 1. Byte scan (current production path) — backward search, zero alloc
//! 2. Regex — compiled-once regex patterns
//! 3. Full serde — serde_json::from_slice + tree traversal

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use regex::Regex;

// ── Byte-scan implementation (mirrors stream_context.rs) ─────────────────────

fn rfind_bytes(haystack: &[u8], needle: &[u8], start_pos: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    let end = start_pos.min(haystack.len() - needle.len());
    for i in (0..=end).rev() {
        if haystack[i..].starts_with(needle) {
            return Some(i);
        }
    }
    None
}

fn parse_number_after_colon(bytes: &[u8]) -> Option<i64> {
    let mut i = 0;
    while i < bytes.len()
        && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\n' || bytes[i] == b'\r')
    {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b':' {
        return None;
    }
    i += 1;
    while i < bytes.len()
        && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\n' || bytes[i] == b'\r')
    {
        i += 1;
    }
    let start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == start {
        return None;
    }
    std::str::from_utf8(&bytes[start..i])
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
}

fn scan_field_i64(bytes: &[u8], field: &[u8]) -> Option<i64> {
    let idx = rfind_bytes(bytes, field, bytes.len())?;
    parse_number_after_colon(&bytes[idx + field.len()..])
}

fn extract_byte_scan(bytes: &[u8]) -> (Option<i64>, Option<i64>) {
    let prompt = scan_field_i64(bytes, b"\"prompt_tokens\"");
    let completion = scan_field_i64(bytes, b"\"completion_tokens\"");
    (prompt, completion)
}

// ── Regex implementation ─────────────────────────────────────────────────────

fn extract_regex(bytes: &[u8], re_prompt: &Regex, re_completion: &Regex) -> (Option<i64>, Option<i64>) {
    let text = std::str::from_utf8(bytes).unwrap_or("");
    let prompt = re_prompt
        .captures(text)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<i64>().ok());
    let completion = re_completion
        .captures(text)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<i64>().ok());
    (prompt, completion)
}

// ── Serde implementation ─────────────────────────────────────────────────────

fn extract_serde(bytes: &[u8]) -> (Option<i64>, Option<i64>) {
    let val: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(_) => return (None, None),
    };
    let usage = &val["usage"];
    let prompt = usage["prompt_tokens"].as_i64();
    let completion = usage["completion_tokens"].as_i64();
    (prompt, completion)
}

// ── Test data ────────────────────────────────────────────────────────────────

const SMALL_RESPONSE: &[u8] = br#"{
  "id": "chatcmpl-abc123",
  "object": "chat.completion",
  "created": 1677858242,
  "model": "gpt-4o-mini",
  "choices": [
    {
      "index": 0,
      "message": {
        "role": "assistant",
        "content": "Hello! How can I help you today?"
      },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 12,
    "completion_tokens": 8,
    "total_tokens": 20
  }
}"#;

const MEDIUM_RESPONSE: &[u8] = br#"{
  "id": "chatcmpl-med456",
  "object": "chat.completion",
  "created": 1677858242,
  "model": "gpt-4o",
  "choices": [
    {
      "index": 0,
      "message": {
        "role": "assistant",
        "content": "Here's a Python implementation of a binary search tree:\n\n```python\nclass Node:\n    def __init__(self, key):\n        self.left = None\n        self.right = None\n        self.val = key\n\nclass BST:\n    def __init__(self):\n        self.root = None\n\n    def insert(self, key):\n        if self.root is None:\n            self.root = Node(key)\n        else:\n            self._insert_recursive(self.root, key)\n\n    def _insert_recursive(self, node, key):\n        if key < node.val:\n            if node.left is None:\n                node.left = Node(key)\n            else:\n                self._insert_recursive(node.left, key)\n        else:\n            if node.right is None:\n                node.right = Node(key)\n            else:\n                self._insert_recursive(node.right, key)\n\n    def search(self, key):\n        return self._search_recursive(self.root, key)\n\n    def _search_recursive(self, node, key):\n        if node is None or node.val == key:\n            return node\n        if key < node.val:\n            return self._search_recursive(node.left, key)\n        return self._search_recursive(node.right, key)\n\n    def inorder(self):\n        result = []\n        self._inorder_recursive(self.root, result)\n        return result\n\n    def _inorder_recursive(self, node, result):\n        if node:\n            self._inorder_recursive(node.left, result)\n            result.append(node.val)\n            self._inorder_recursive(node.right, result)\n\n    def delete(self, key):\n        self.root = self._delete_recursive(self.root, key)\n\n    def _delete_recursive(self, node, key):\n        if node is None:\n            return node\n        if key < node.val:\n            node.left = self._delete_recursive(node.left, key)\n        elif key > node.val:\n            node.right = self._delete_recursive(node.right, key)\n        else:\n            if node.left is None:\n                return node.right\n            elif node.right is None:\n                return node.left\n            temp = self._min_value_node(node.right)\n            node.val = temp.val\n            node.right = self._delete_recursive(node.right, temp.val)\n        return node\n\n    def _min_value_node(self, node):\n        current = node\n        while current.left is not None:\n            current = current.left\n        return current\n```\n\nThis implementation provides O(log n) average time complexity for insert, search, and delete operations."
      },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 45,
    "completion_tokens": 512,
    "total_tokens": 557
  }
}"#;

const STREAMING_CHUNK: &[u8] = br#"{"id":"chatcmpl-s789","object":"chat.completion.chunk","created":1677858242,"model":"gpt-4o","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":25,"completion_tokens":142,"total_tokens":167}}"#;

fn make_large_response() -> Vec<u8> {
    // ~100KB response with large content and usage at the end
    let filler = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ".repeat(1500);
    format!(
        r#"{{"id":"chatcmpl-lg000","object":"chat.completion","created":1677858242,"model":"gpt-4o","choices":[{{"index":0,"message":{{"role":"assistant","content":"{}"}},"finish_reason":"stop"}}],"usage":{{"prompt_tokens":100,"completion_tokens":25000,"total_tokens":25100}}}}"#,
        filler
    )
    .into_bytes()
}

// ── Benchmarks ───────────────────────────────────────────────────────────────

fn bench_extraction(c: &mut Criterion) {
    let large_response = make_large_response();

    let cases: Vec<(&str, &[u8])> = vec![
        ("small_1KB", SMALL_RESPONSE),
        ("medium_10KB", MEDIUM_RESPONSE),
        ("large_100KB", &large_response),
        ("streaming_chunk", STREAMING_CHUNK),
    ];

    let re_prompt = Regex::new(r#""prompt_tokens"\s*:\s*(\d+)"#).unwrap();
    let re_completion = Regex::new(r#""completion_tokens"\s*:\s*(\d+)"#).unwrap();

    let mut group = c.benchmark_group("token_extraction");

    for (label, data) in &cases {
        group.bench_with_input(BenchmarkId::new("byte_scan", label), data, |b, data| {
            b.iter(|| extract_byte_scan(black_box(data)))
        });

        group.bench_with_input(
            BenchmarkId::new("regex", label),
            data,
            |b, data| {
                b.iter(|| extract_regex(black_box(data), &re_prompt, &re_completion))
            },
        );

        group.bench_with_input(BenchmarkId::new("serde", label), data, |b, data| {
            b.iter(|| extract_serde(black_box(data)))
        });
    }

    group.finish();
}

criterion_group!(benches, bench_extraction);
criterion_main!(benches);
