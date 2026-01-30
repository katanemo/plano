use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::{Method, Request, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Default)]
struct TraceQuery {
    filter: Vec<String>,
    r#where: Vec<(String, String)>,
    list: bool,
    limit: Option<usize>,
    json: bool,
    since_seconds: Option<u64>,
}

#[derive(Default, Serialize, Deserialize)]
struct TraceRecord {
    trace_id: String,
    request_ids: HashSet<String>,
    spans: Vec<serde_json::Value>,
}

fn full<T: Into<Bytes>>(chunk: T) -> BoxBody<Bytes, hyper::Error> {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}

fn json_response(
    value: serde_json::Value,
    status: StatusCode,
) -> Response<BoxBody<Bytes, hyper::Error>> {
    let mut response = Response::new(full(value.to_string()));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert("Content-Type", "application/json".parse().unwrap());
    response
}

fn parse_bool(value: &str) -> bool {
    matches!(value, "1" | "true" | "yes" | "on")
}

/// Convert an ASCII hex nibble to its value.
fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Decode query-string percent escapes and '+' to spaces.
fn percent_decode(input: &str) -> String {
    let mut out: Vec<u8> = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                // Try to decode %HH; fall through if invalid.
                if let (Some(high), Some(low)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2])) {
                    out.push(high * 16 + low);
                    i += 3;
                    continue;
                }
            }
            b'+' => {
                // Query strings often encode spaces as '+'.
                out.push(b' ');
                i += 1;
                continue;
            }
            _ => {}
        }

        // Default: copy byte through unchanged.
        out.push(bytes[i]);
        i += 1;
    }

    String::from_utf8_lossy(&out).to_string()
}

fn parse_query(query: Option<&str>) -> TraceQuery {
    let mut parsed = TraceQuery::default();
    let Some(query) = query else {
        return parsed;
    };

    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let mut parts = pair.splitn(2, '=');
        let key = percent_decode(parts.next().unwrap_or("").trim());
        let value = percent_decode(parts.next().unwrap_or("").trim());
        match key.as_str() {
            "filter" => {
                parsed.filter.extend(
                    value
                        .split(',')
                        .filter(|v| !v.is_empty())
                        .map(|v| v.to_string()),
                );
            }
            "where" => {
                if let Some((k, v)) = value.split_once('=') {
                    if !k.is_empty() {
                        parsed.r#where.push((k.to_string(), v.to_string()));
                    }
                }
            }
            "list" => {
                parsed.list = value.is_empty() || parse_bool(&value);
            }
            "limit" => {
                parsed.limit = value.parse::<usize>().ok();
            }
            "json" => {
                parsed.json = value.is_empty() || parse_bool(&value);
            }
            "since" => {
                parsed.since_seconds = parse_since_seconds(&value);
            }
            _ => {}
        }
    }

    parsed
}

fn parse_since_seconds(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    let (number, unit) = value.split_at(value.len() - 1);
    let qty = number.parse::<u64>().ok()?;
    let multiplier = match unit {
        "m" => 60,
        "h" => 60 * 60,
        "d" => 60 * 60 * 24,
        _ => return None,
    };
    Some(qty.saturating_mul(multiplier))
}

fn matches_pattern(value: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return value == pattern;
    }

    let parts: Vec<&str> = pattern.split('*').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return true;
    }

    let mut remaining = value;
    for (idx, part) in parts.iter().enumerate() {
        if let Some(pos) = remaining.find(part) {
            if idx == 0 && !pattern.starts_with('*') && pos != 0 {
                return false;
            }
            remaining = &remaining[pos + part.len()..];
        } else {
            return false;
        }
    }

    if !pattern.ends_with('*') && !remaining.is_empty() {
        return false;
    }

    true
}

fn attribute_map(span: &serde_json::Value) -> HashMap<String, String> {
    let mut attrs = HashMap::new();
    let Some(attr_list) = span.get("attributes").and_then(|v| v.as_array()) else {
        return attrs;
    };

    for attr in attr_list {
        let key = attr.get("key").and_then(|v| v.as_str());
        let value_obj = attr.get("value");
        let value = value_obj
            .and_then(|v| v.get("stringValue"))
            .and_then(|v| v.as_str())
            .map(|v| v.to_string())
            .or_else(|| {
                value_obj
                    .and_then(|v| v.get("intValue"))
                    .and_then(|v| v.as_i64())
                    .map(|v| v.to_string())
            })
            .or_else(|| {
                value_obj
                    .and_then(|v| v.get("doubleValue"))
                    .and_then(|v| v.as_f64())
                    .map(|v| v.to_string())
            })
            .or_else(|| {
                value_obj
                    .and_then(|v| v.get("boolValue"))
                    .and_then(|v| v.as_bool())
                    .map(|v| v.to_string())
            });
        if let (Some(k), Some(v)) = (key, value) {
            attrs.insert(k.to_string(), v);
        }
    }

    attrs
}

fn filter_attributes(span: &serde_json::Value, patterns: &[String]) -> serde_json::Value {
    if patterns.is_empty() {
        return span.clone();
    }

    let Some(attr_list) = span.get("attributes").and_then(|v| v.as_array()) else {
        return span.clone();
    };

    let mut filtered = Vec::new();
    for attr in attr_list {
        let key = attr.get("key").and_then(|v| v.as_str()).unwrap_or("");
        if patterns.iter().any(|p| matches_pattern(key, p)) {
            filtered.push(attr.clone());
        }
    }

    let mut cloned = span.clone();
    if let Some(obj) = cloned.as_object_mut() {
        obj.insert("attributes".to_string(), serde_json::Value::Array(filtered));
    }
    cloned
}

fn read_recent_lines(
    path: &Path,
    max_bytes: u64,
    max_lines: usize,
) -> std::io::Result<Vec<String>> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start))?;

    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;

    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }

    let mut recent: Vec<String> = lines
        .into_iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.to_string())
        .collect();

    if recent.len() > max_lines {
        recent = recent[recent.len() - max_lines..].to_vec();
    }

    Ok(recent)
}

fn collect_traces(lines: Vec<String>, query: &TraceQuery) -> (Vec<TraceRecord>, Vec<String>) {
    let mut traces: HashMap<String, TraceRecord> = HashMap::new();
    let mut trace_order: Vec<String> = Vec::new();

    let now_nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let since_nanos = query
        .since_seconds
        .map(|s| now_nanos.saturating_sub((s as u128) * 1_000_000_000));

    for line in lines {
        let Ok(payload) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };

        let resource_spans = payload.get("resourceSpans").and_then(|v| v.as_array());
        let Some(resource_spans) = resource_spans else {
            continue;
        };

        for resource_span in resource_spans {
            let service_name = resource_span
                .get("resource")
                .and_then(|v| v.get("attributes"))
                .and_then(|v| v.as_array())
                .and_then(|attrs| {
                    attrs.iter().find(|attr| {
                        attr.get("key").and_then(|v| v.as_str()) == Some("service.name")
                    })
                })
                .and_then(|attr| attr.get("value"))
                .and_then(|v| v.get("stringValue"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            let scope_spans = resource_span
                .get("scopeSpans")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            for scope_span in scope_spans {
                let spans = scope_span
                    .get("spans")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();

                for span in spans {
                    if let Some(cutoff) = since_nanos {
                        let start = span
                            .get("startTimeUnixNano")
                            .and_then(|v| v.as_str())
                            .and_then(|v| v.parse::<u128>().ok())
                            .unwrap_or(0);
                        if start < cutoff {
                            continue;
                        }
                    }
                    let trace_id = span
                        .get("traceId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if trace_id.is_empty() {
                        continue;
                    }

                    let entry = traces.entry(trace_id.clone()).or_insert_with(|| {
                        trace_order.push(trace_id.clone());
                        TraceRecord {
                            trace_id: trace_id.clone(),
                            ..TraceRecord::default()
                        }
                    });

                    let mut span_obj = span.clone();
                    if let Some(obj) = span_obj.as_object_mut() {
                        obj.insert(
                            "service".to_string(),
                            serde_json::Value::String(service_name.to_string()),
                        );
                    }

                    let attrs = attribute_map(&span);
                    if let Some(request_id) = attrs
                        .get("x-request-id")
                        .or_else(|| attrs.get("guid:x-request-id"))
                    {
                        entry.request_ids.insert(request_id.to_string());
                    }

                    entry.spans.push(span_obj);
                }
            }
        }
    }

    let mut traces_vec: Vec<TraceRecord> = trace_order
        .into_iter()
        .filter_map(|trace_id| traces.remove(&trace_id))
        .collect();

    traces_vec.reverse(); // newest first

    // Apply where filters (AND semantics)
    if !query.r#where.is_empty() {
        traces_vec.retain(|trace| {
            query.r#where.iter().all(|(key, value)| {
                trace.spans.iter().any(|span| {
                    let attrs = attribute_map(span);
                    attrs.get(key).map(|v| v == value).unwrap_or(false)
                })
            })
        });
    }

    // Apply filter patterns to attributes
    if !query.filter.is_empty() {
        for trace in &mut traces_vec {
            trace.spans = trace
                .spans
                .iter()
                .map(|span| filter_attributes(span, &query.filter))
                .collect();
        }
    }

    let request_ids = traces_vec
        .iter()
        .flat_map(|trace| trace.request_ids.iter().cloned())
        .collect::<Vec<_>>();

    (traces_vec, request_ids)
}

pub fn handle_trace_api(
    req: &Request<hyper::body::Incoming>,
) -> Option<Response<BoxBody<Bytes, hyper::Error>>> {
    let path = req.uri().path().trim_end_matches('/');
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() < 2 || segments[0] != "debug" || segments[1] != "traces" {
        return None;
    }

    if req.method() != Method::GET {
        return Some(json_response(
            json!({"error": "method_not_allowed"}),
            StatusCode::METHOD_NOT_ALLOWED,
        ));
    }

    let log_path = std::env::var("PLANO_LOCAL_OTLP_LOG_PATH")
        .unwrap_or_else(|_| "/var/log/plano/otel.jsonl".to_string());
    let max_bytes = std::env::var("PLANO_LOCAL_OTLP_QUERY_MAX_BYTES")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(5_000_000);
    let max_lines = std::env::var("PLANO_LOCAL_OTLP_QUERY_MAX_LINES")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(2000);

    let query = parse_query(req.uri().query());
    let path_obj = Path::new(&log_path);
    let lines = match read_recent_lines(path_obj, max_bytes, max_lines) {
        Ok(lines) => lines,
        Err(_) => {
            return Some(json_response(
                json!({"error": "otlp_log_not_found", "path": log_path}),
                StatusCode::NOT_FOUND,
            ))
        }
    };

    let (mut traces, mut request_ids) = collect_traces(lines, &query);

    // Endpoint routing
    if segments.len() == 3 && segments[2] == "last" {
        traces.truncate(1);
        request_ids.truncate(1);
    } else if segments.len() == 3 && segments[2] != "last" && segments[2] != "any" {
        let trace_id = segments[2];
        traces.retain(|trace| trace.trace_id == trace_id);
        request_ids = traces
            .iter()
            .flat_map(|trace| trace.request_ids.iter().cloned())
            .collect();
    } else if segments.len() == 4 && segments[2] == "by-request" {
        let request_id = segments[3];
        traces.retain(|trace| trace.request_ids.contains(request_id));
        request_ids = traces
            .iter()
            .flat_map(|trace| trace.request_ids.iter().cloned())
            .collect();
    }

    if let Some(limit) = query.limit {
        if query.list {
            request_ids.truncate(limit);
        } else {
            traces.truncate(limit);
        }
    }

    if query.list {
        return Some(json_response(
            json!({ "request_ids": request_ids }),
            StatusCode::OK,
        ));
    }

    Some(json_response(
        json!({
            "traces": traces
        }),
        StatusCode::OK,
    ))
}
