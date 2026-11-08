use crate::metrics::Metrics;
use common::api::open_ai::{ChatCompletionsRequest, Message};
use common::configuration::{Endpoint, Overrides, Tracing};
use common::errors::ServerError;
use common::http::Client;
use common::stats::Gauge;
use http::StatusCode;
use proxy_wasm::traits::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Context for in-flight HTTP callouts. Prompt gateway is currently a passthrough
/// filter and does not issue callouts; this remains to satisfy the [`Client`] trait.
#[derive(Clone, Debug)]
pub struct StreamCallContext {}

pub struct StreamContext {
    pub endpoints: Rc<Option<HashMap<String, Endpoint>>>,
    pub overrides: Rc<Option<Overrides>>,
    pub metrics: Rc<Metrics>,
    pub callouts: RefCell<HashMap<u32, StreamCallContext>>,
    pub context_id: u32,
    pub request_body_size: usize,
    pub user_prompt: Option<Message>,
    pub streaming_response: bool,
    pub is_chat_completions_request: bool,
    pub chat_completions_request: Option<ChatCompletionsRequest>,
    pub request_id: Option<String>,
    pub traceparent: Option<String>,
    pub _tracing: Rc<Option<Tracing>>,
}

impl StreamContext {
    pub fn new(
        context_id: u32,
        metrics: Rc<Metrics>,
        endpoints: Rc<Option<HashMap<String, Endpoint>>>,
        overrides: Rc<Option<Overrides>>,
        tracing: Rc<Option<Tracing>>,
    ) -> Self {
        StreamContext {
            context_id,
            metrics,
            endpoints,
            callouts: RefCell::new(HashMap::new()),
            chat_completions_request: None,
            request_body_size: 0,
            streaming_response: false,
            user_prompt: None,
            is_chat_completions_request: false,
            overrides,
            request_id: None,
            traceparent: None,
            _tracing: tracing,
        }
    }

    pub fn send_server_error(&self, error: ServerError, override_status_code: Option<StatusCode>) {
        self.send_http_response(
            override_status_code
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
                .as_u16()
                .into(),
            vec![],
            Some(format!("{error}").as_bytes()),
        );
    }
}

impl Client for StreamContext {
    type CallContext = StreamCallContext;

    fn callouts(&self) -> &RefCell<HashMap<u32, Self::CallContext>> {
        &self.callouts
    }

    fn active_http_calls(&self) -> &Gauge {
        &self.metrics.active_http_calls
    }
}
