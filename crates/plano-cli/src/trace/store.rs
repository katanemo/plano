use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

const MAX_TRACES: usize = 50;
const MAX_SPANS_PER_TRACE: usize = 500;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Span {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    pub start_time_unix_nano: u64,
    pub end_time_unix_nano: u64,
    pub status: Option<SpanStatus>,
    pub attributes: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpanStatus {
    pub code: i32,
    pub message: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Trace {
    pub trace_id: String,
    pub spans: Vec<Span>,
    pub root_span: Option<String>,
    pub start_time: u64,
}

pub type SharedTraceStore = Arc<RwLock<TraceStore>>;

#[derive(Debug, Default)]
pub struct TraceStore {
    traces: HashMap<String, Trace>,
    trace_order: Vec<String>,
    /// Maps span_id → group_trace_id (for merging traces via parent_span_id links)
    span_to_group: HashMap<String, String>,
}

impl TraceStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn shared() -> SharedTraceStore {
        Arc::new(RwLock::new(Self::new()))
    }

    pub fn add_spans(&mut self, spans: Vec<Span>) {
        for span in spans {
            let trace_id = span.trace_id.clone();

            // Check if this span belongs to an existing group via parent_span_id
            let group_id = span
                .parent_span_id
                .as_ref()
                .and_then(|pid| self.span_to_group.get(pid).cloned())
                .unwrap_or_else(|| trace_id.clone());

            // Register this span's ID in the group
            self.span_to_group
                .insert(span.span_id.clone(), group_id.clone());

            let trace = self.traces.entry(group_id.clone()).or_insert_with(|| {
                self.trace_order.push(group_id.clone());
                Trace {
                    trace_id: group_id.clone(),
                    spans: Vec::new(),
                    root_span: None,
                    start_time: span.start_time_unix_nano,
                }
            });

            // Dedup by span_id
            if trace.spans.iter().any(|s| s.span_id == span.span_id) {
                continue;
            }

            // Track root span
            if span.parent_span_id.is_none() || span.parent_span_id.as_deref() == Some("") {
                trace.root_span = Some(span.span_id.clone());
            }

            if trace.spans.len() < MAX_SPANS_PER_TRACE {
                trace.spans.push(span);
            }
        }

        // Evict oldest traces
        while self.trace_order.len() > MAX_TRACES {
            if let Some(oldest) = self.trace_order.first().cloned() {
                self.trace_order.remove(0);
                if let Some(trace) = self.traces.remove(&oldest) {
                    for span in &trace.spans {
                        self.span_to_group.remove(&span.span_id);
                    }
                }
            }
        }
    }

    pub fn get_traces(&self) -> Vec<&Trace> {
        self.trace_order
            .iter()
            .rev()
            .filter_map(|id| self.traces.get(id))
            .collect()
    }

    pub fn get_trace(&self, trace_id: &str) -> Option<&Trace> {
        self.traces.get(trace_id)
    }
}
