use rand::RngCore;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct ResourceSpan {
    pub resource: Resource,
    #[serde(rename = "scopeSpans")]
    pub scope_spans: Vec<ScopeSpan>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Resource {
    pub attributes: Vec<Attribute>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ScopeSpan {
    scope: Scope,
    spans: Vec<Span>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Scope {
    name: String,
    version: String,
    attributes: Vec<Attribute>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Span {
    #[serde(rename = "traceId")]
    pub trace_id: String,
    #[serde(rename = "spanId")]
    pub span_id: String,
    #[serde(rename = "parentSpanId")]
    pub parent_span_id: Option<String>, // Optional in case there’s no parent span
    pub name: String,
    #[serde(rename = "startTimeUnixNano")]
    pub start_time_unix_nano: String,
    #[serde(rename = "endTimeUnixNano")]
    pub end_time_unix_nano: String,
    pub kind: u32,
    pub attributes: Vec<Attribute>,
    pub events: Option<Vec<Event>>,
}

impl Span {
    pub fn new(
        name: String,
        trace_id: Option<String>,
        parent_span_id: Option<String>,
        start_time_unix_nano: u128,
        end_time_unix_nano: u128,
    ) -> Self {
        let trace_id = match trace_id {
            Some(trace_id) => trace_id,
            None => Span::get_random_trace_id(),
        };
        Span {
            trace_id,
            span_id: Span::get_random_span_id(),
            parent_span_id,
            name,
            start_time_unix_nano: format!("{}", start_time_unix_nano),
            end_time_unix_nano: format!("{}", end_time_unix_nano),
            kind: 0,
            attributes: Vec::new(),
            events: None,
        }
    }

    pub fn add_attribute(&mut self, key: String, value: String) {
        self.attributes.push(Attribute {
            key,
            value: AttributeValue {
                string_value: Some(value),
            },
        });
    }

    pub fn add_event(&mut self, event: Event) {
        if self.events.is_none() {
            self.events = Some(Vec::new());
        }
        self.events.as_mut().unwrap().push(event);
    }

    fn get_random_span_id() -> String {
        let mut rng = rand::thread_rng();
        let mut random_bytes = [0u8; 8];
        rng.fill_bytes(&mut random_bytes);

        hex::encode(random_bytes)
    }

    fn get_random_trace_id() -> String {
        let mut rng = rand::thread_rng();
        let mut random_bytes = [0u8; 16];
        rng.fill_bytes(&mut random_bytes);

        hex::encode(random_bytes)
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Event {
    #[serde(rename = "timeUnixNano")]
    pub time_unix_nano: String,
    pub name: String,
    pub attributes: Vec<Attribute>,
}

impl Event {
    pub fn new(name: String, time_unix_nano: u128) -> Self {
        Event {
            time_unix_nano: format!("{}", time_unix_nano),
            name,
            attributes: Vec::new(),
        }
    }

    pub fn add_attribute(&mut self, key: String, value: String) {
        self.attributes.push(Attribute {
            key,
            value: AttributeValue {
                string_value: Some(value),
            },
        });
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Attribute {
    key: String,
    value: AttributeValue,
}

#[derive(Serialize, Deserialize, Debug)]
struct AttributeValue {
    #[serde(rename = "stringValue")]
    string_value: Option<String>, // Use Option to handle different value types
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraceData {
    #[serde(rename = "resourceSpans")]
    resource_spans: Vec<ResourceSpan>,
}

impl Default for TraceData {
    fn default() -> Self {
        Self::new()
    }
}

impl TraceData {
    pub fn new() -> Self {
        TraceData {
            resource_spans: Vec::new(),
        }
    }

    pub fn new_with_service_name(service_name: String) -> Self {
        let resource_attributes = vec![Attribute {
            key: "service.name".to_string(),
            value: AttributeValue {
                string_value: Some(service_name),
            },
        }];

        let resource = Resource {
            attributes: resource_attributes,
        };

        let scope_span = ScopeSpan {
            scope: Scope {
                name: "default".to_string(),
                version: "1.0".to_string(),
                attributes: Vec::new(),
            },
            spans: Vec::new(),
        };

        let resource_span = ResourceSpan {
            resource,
            scope_spans: vec![scope_span],
        };

        TraceData {
            resource_spans: vec![resource_span],
        }
    }

    pub fn add_span(&mut self, span: Span) {
        if self.resource_spans.is_empty() {
            let resource = Resource {
                attributes: Vec::new(),
            };
            let scope_span = ScopeSpan {
                scope: Scope {
                    name: "default".to_string(),
                    version: "1.0".to_string(),
                    attributes: Vec::new(),
                },
                spans: Vec::new(),
            };
            let resource_span = ResourceSpan {
                resource,
                scope_spans: vec![scope_span],
            };
            self.resource_spans.push(resource_span);
        }
        self.resource_spans[0].scope_spans[0].spans.push(span);
    }
}

pub struct Traceparent {
    pub version: String,
    pub trace_id: String,
    pub parent_id: String,
    pub flags: String,
}

impl std::fmt::Display for Traceparent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}-{}-{}-{}",
            self.version, self.trace_id, self.parent_id, self.flags
        )
    }
}

#[derive(thiserror::Error, Debug)]
pub enum TraceparentNewError {
    #[error("Invalid traceparent: \'{0}\'")]
    InvalidTraceparent(String),
}

impl TryFrom<String> for Traceparent {
    type Error = TraceparentNewError;

    fn try_from(traceparent: String) -> Result<Self, Self::Error> {
        let traceparent_tokens: Vec<&str> = traceparent.split("-").collect::<Vec<&str>>();
        if traceparent_tokens.len() != 4 {
            return Err(TraceparentNewError::InvalidTraceparent(traceparent));
        }
        Ok(Traceparent {
            version: traceparent_tokens[0].to_string(),
            trace_id: traceparent_tokens[1].to_string(),
            parent_id: traceparent_tokens[2].to_string(),
            flags: traceparent_tokens[3].to_string(),
        })
    }
}
