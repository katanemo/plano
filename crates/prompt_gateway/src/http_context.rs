use crate::stream_context::StreamContext;
use common::{
    api::open_ai::ChatCompletionsRequest,
    consts::{
        ARCH_ROUTING_HEADER, CHAT_COMPLETIONS_PATH, HEALTHZ_PATH, REQUEST_ID_HEADER,
        TRACE_PARENT_HEADER, USER_ROLE,
    },
    errors::ServerError,
    pii::obfuscate_auth_header,
};
use log::{debug, info, warn};
use proxy_wasm::{traits::HttpContext, types::Action};

impl HttpContext for StreamContext {
    fn on_http_request_headers(&mut self, _num_headers: usize, _end_of_stream: bool) -> Action {
        // Remove Content-Length; body may be rewritten below.
        self.set_http_request_header("content-length", None);

        if let Some(overrides) = self.overrides.as_ref() {
            if overrides.use_agent_orchestrator.unwrap_or_default() {
                if let Some(endpoints) = self.endpoints.as_ref() {
                    if endpoints.len() == 1 {
                        let (name, _) = endpoints.iter().next().unwrap();
                        info!("Setting ARCH_PROVIDER_HINT_HEADER to {}", name);
                        self.set_http_request_header(ARCH_ROUTING_HEADER, Some(name));
                    } else {
                        warn!("Need single endpoint when use_agent_orchestrator is set");
                        self.send_server_error(
                            ServerError::LogicError(
                                "Need single endpoint when use_agent_orchestrator is set"
                                    .to_string(),
                            ),
                            None,
                        );
                    }
                }
            }
        }

        let request_path = self.get_http_request_header(":path").unwrap_or_default();
        if request_path == HEALTHZ_PATH {
            self.send_http_response(200, vec![], None);
            return Action::Continue;
        }

        self.is_chat_completions_request = CHAT_COMPLETIONS_PATH.contains(request_path.as_str());

        debug!(
            "on_http_request_headers S[{}] req_headers={:?}",
            self.context_id,
            obfuscate_auth_header(&mut self.get_http_request_headers())
        );

        self.request_id = self.get_http_request_header(REQUEST_ID_HEADER);
        self.traceparent = self.get_http_request_header(TRACE_PARENT_HEADER);

        Action::Continue
    }

    fn on_http_request_body(&mut self, body_size: usize, end_of_stream: bool) -> Action {
        if !end_of_stream {
            return Action::Pause;
        }

        if body_size == 0 {
            return Action::Continue;
        }

        self.request_body_size = body_size;

        // Only rewrite body when metadata must be injected.
        let needs_metadata_injection = self
            .overrides
            .as_ref()
            .as_ref()
            .and_then(|o| o.use_agent_orchestrator)
            .unwrap_or_default()
            || self
                .overrides
                .as_ref()
                .as_ref()
                .and_then(|o| o.optimize_context_window)
                .unwrap_or_default();

        if !needs_metadata_injection {
            return Action::Continue;
        }

        let body_bytes = match self.get_http_request_body(0, body_size) {
            Some(body_bytes) => body_bytes,
            None => {
                self.send_server_error(
                    ServerError::LogicError(format!(
                        "Failed to obtain body bytes even though body_size is {}",
                        body_size
                    )),
                    None,
                );
                return Action::Pause;
            }
        };

        let mut deserialized_body: ChatCompletionsRequest =
            match serde_json::from_slice(&body_bytes) {
                Ok(deserialized) => deserialized,
                Err(e) => {
                    warn!("Failed to deserialize request body for metadata injection: {e}");
                    return Action::Continue;
                }
            };

        self.streaming_response = deserialized_body.stream;
        self.user_prompt = deserialized_body
            .messages
            .iter()
            .rfind(|msg| msg.role == USER_ROLE)
            .cloned();

        let mut metadata = deserialized_body.metadata.take().unwrap_or_default();

        if let Some(overrides) = self.overrides.as_ref() {
            if overrides.optimize_context_window.unwrap_or_default() {
                metadata.insert("optimize_context_window".to_string(), "true".to_string());
            }
            if overrides.use_agent_orchestrator.unwrap_or_default() {
                metadata.insert("use_agent_orchestrator".to_string(), "true".to_string());
            }
        }

        deserialized_body.metadata = Some(metadata);
        self.chat_completions_request = Some(deserialized_body.clone());

        match serde_json::to_vec(&deserialized_body) {
            Ok(json_data) => {
                self.set_http_request_body(0, body_size, &json_data);
            }
            Err(error) => {
                self.send_server_error(ServerError::Serialization(error), None);
                return Action::Pause;
            }
        }

        Action::Continue
    }

    fn on_http_response_headers(&mut self, _num_headers: usize, _end_of_stream: bool) -> Action {
        debug!(
            "on_http_response_headers recv [S={}] headers={:?}",
            self.context_id,
            self.get_http_response_headers()
        );
        Action::Continue
    }

    fn on_http_response_body(&mut self, body_size: usize, end_of_stream: bool) -> Action {
        debug!(
            "on_http_response_body: recv [S={}] bytes={} end_stream={}",
            self.context_id, body_size, end_of_stream
        );
        Action::Continue
    }
}
