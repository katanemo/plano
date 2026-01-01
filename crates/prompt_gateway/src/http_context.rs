use crate::stream_context::{ResponseHandlerType, StreamCallContext, StreamContext};
use common::{
    api::open_ai::{
        self, ArchState, ChatCompletionStreamResponse, ChatCompletionTool, ChatCompletionsRequest,
    },
    consts::{
        ARCH_FC_MODEL_NAME, ARCH_INTERNAL_CLUSTER_NAME, ARCH_ROUTING_HEADER,
        ARCH_UPSTREAM_HOST_HEADER, ASSISTANT_ROLE, CHAT_COMPLETIONS_PATH, HEALTHZ_PATH,
        MODEL_SERVER_NAME, MODEL_SERVER_REQUEST_TIMEOUT_MS, REQUEST_ID_HEADER, TOOL_ROLE,
        TRACE_PARENT_HEADER, USER_ROLE, X_ARCH_API_RESPONSE, X_ARCH_FC_MODEL_RESPONSE,
        X_ARCH_STATE_HEADER, X_ARCH_TOOL_CALL,
    },
    errors::ServerError,
    http::{CallArgs, Client},
    pii::obfuscate_auth_header,
};
use http::StatusCode;
use log::{debug, info, warn};
use proxy_wasm::{traits::HttpContext, types::Action};
use serde_json::Value;
use std::{
    collections::HashMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

// HttpContext is the trait that allows the Rust code to interact with HTTP objects.
impl HttpContext for StreamContext {
    // Envoy's HTTP model is event driven. The WASM ABI has given implementors events to hook onto
    // the lifecycle of the http request and response.
    fn on_http_request_headers(&mut self, _num_headers: usize, _end_of_stream: bool) -> Action {
        // Remove the Content-Length header because further body manipulations in the gateway logic will invalidate it.
        // Server's generally throw away requests whose body length do not match the Content-Length header.
        // However, a missing Content-Length header is not grounds for bad requests given that intermediary hops could
        // manipulate the body in benign ways e.g., compression.
        self.set_http_request_header("content-length", None);

        if let Some(overrides) = self.overrides.as_ref() {
            if overrides.use_agent_orchestrator.unwrap_or_default() {
                // get endpoint that has agent_orchestrator set to true
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
        // Let the client send the gateway all the data before sending to the LLM_provider.
        // TODO: consider a streaming API.

        if !end_of_stream {
            return Action::Pause;
        }

        if body_size == 0 {
            return Action::Continue;
        }

        self.request_body_size = body_size;

        debug!(
            "on_http_request_body S[{}] body_size={}",
            self.context_id, body_size
        );

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

        debug!("request body: {}", String::from_utf8_lossy(&body_bytes));

        // Deserialize body into spec.
        // Currently OpenAI API.
        let deserialized_body: ChatCompletionsRequest = match serde_json::from_slice(&body_bytes) {
            Ok(deserialized) => deserialized,
            Err(e) => {
                self.send_server_error(
                    ServerError::Deserialization(e),
                    Some(StatusCode::BAD_REQUEST),
                );
                return Action::Pause;
            }
        };

        self.arch_state = match deserialized_body.metadata {
            Some(ref metadata) => {
                if metadata.contains_key(X_ARCH_STATE_HEADER) {
                    let arch_state_str = metadata[X_ARCH_STATE_HEADER].clone();
                    let arch_state: Vec<ArchState> = serde_json::from_str(&arch_state_str).unwrap();
                    Some(arch_state)
                } else {
                    None
                }
            }
            None => None,
        };

        self.streaming_response = deserialized_body.stream;

        let last_user_prompt = match deserialized_body
            .messages
            .iter()
            .rfind(|msg| msg.role == USER_ROLE)
        {
            Some(content) => content,
            None => {
                warn!("No messages in the request body");
                return Action::Continue;
            }
        };

        self.user_prompt = Some(last_user_prompt.clone());

        // convert prompt targets to ChatCompletionTool
        let tool_calls: Vec<ChatCompletionTool> =
            self.prompt_targets.values().map(|pt| pt.into()).collect();

        let mut metadata = deserialized_body.metadata.clone();

        if let Some(overrides) = self.overrides.as_ref() {
            if overrides.optimize_context_window.unwrap_or_default() {
                if metadata.is_none() {
                    metadata = Some(HashMap::new());
                }
                metadata
                    .as_mut()
                    .unwrap()
                    .insert("optimize_context_window".to_string(), "true".to_string());
            }
        }

        if let Some(overrides) = self.overrides.as_ref() {
            if overrides.use_agent_orchestrator.unwrap_or_default() {
                if metadata.is_none() {
                    metadata = Some(HashMap::new());
                }
                metadata
                    .as_mut()
                    .unwrap()
                    .insert("use_agent_orchestrator".to_string(), "true".to_string());
            }
        }

        let arch_fc_chat_completion_request = ChatCompletionsRequest {
            messages: deserialized_body.messages.clone(),
            metadata,
            stream: deserialized_body.stream,
            model: deserialized_body.model.clone(),
            stream_options: deserialized_body.stream_options.clone(),
            tools: Some(tool_calls),
        };

        self.chat_completions_request = Some(deserialized_body);

        let json_data = match serde_json::to_string(&arch_fc_chat_completion_request) {
            Ok(json_data) => json_data,
            Err(error) => {
                self.send_server_error(ServerError::Serialization(error), None);
                return Action::Pause;
            }
        };

        info!("on_http_request_body: sending request to model server");
        debug!("request body: {}", json_data);

        let timeout_str = MODEL_SERVER_REQUEST_TIMEOUT_MS.to_string();

        let mut headers = vec![
            (ARCH_UPSTREAM_HOST_HEADER, MODEL_SERVER_NAME),
            (":method", "POST"),
            (":path", "/function_calling"),
            ("content-type", "application/json"),
            (":authority", MODEL_SERVER_NAME),
            ("x-envoy-upstream-rq-timeout-ms", timeout_str.as_str()),
        ];

        if self.request_id.is_some() {
            headers.push((REQUEST_ID_HEADER, self.request_id.as_ref().unwrap()));
        }

        if self.traceparent.is_some() {
            headers.push((TRACE_PARENT_HEADER, self.traceparent.as_ref().unwrap()));
        }

        let call_args = CallArgs::new(
            ARCH_INTERNAL_CLUSTER_NAME,
            "/function_calling",
            headers,
            Some(json_data.as_bytes()),
            vec![],
            Duration::from_secs(5),
        );

        if let Some(content) = self.user_prompt.as_ref().unwrap().content.as_ref() {
            let call_context = StreamCallContext {
                response_handler_type: ResponseHandlerType::ArchFC,
                user_message: Some(content.to_string()),
                prompt_target_name: None,
                request_body: self.chat_completions_request.as_ref().unwrap().clone(),
                similarity_scores: None,
                upstream_cluster: Some(ARCH_INTERNAL_CLUSTER_NAME.to_string()),
                upstream_cluster_path: Some("/function_calling".to_string()),
            };

            if let Err(e) = self.http_call(call_args, call_context) {
                warn!("http_call failed: {:?}", e);
                self.send_server_error(ServerError::HttpDispatch(e), None);
            }
        } else {
            warn!("No content in the last user prompt");
            self.send_server_error(
                ServerError::LogicError("No content in the last user prompt".to_string()),
                None,
            );
        }
        Action::Pause
    }

    fn on_http_response_headers(&mut self, _num_headers: usize, _end_of_stream: bool) -> Action {
        debug!(
            "on_http_response_headers recv [S={}] headers={:?}",
            self.context_id,
            self.get_http_response_headers()
        );
        // delete content-lenght header let envoy calculate it, because we modify the response body
        // that would result in a different content-length
        self.set_http_response_header("content-length", None);
        Action::Continue
    }

    fn on_http_response_body(&mut self, body_size: usize, end_of_stream: bool) -> Action {
        debug!(
            "on_http_response_body: recv [S={}] bytes={} end_stream={}",
            self.context_id, body_size, end_of_stream
        );

        if !self.is_chat_completions_request {
            info!("non-gpt request");
            return Action::Continue;
        }

        if self.time_to_first_token.is_none() {
            self.time_to_first_token = Some(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
            );
        }

        if end_of_stream && body_size == 0 {
            return Action::Continue;
        }

        let body = if self.streaming_response {
            let streaming_chunk = match self.get_http_response_body(0, body_size) {
                Some(chunk) => chunk,
                None => {
                    warn!(
                        "response body empty, chunk_start: {}, chunk_size: {}",
                        0, body_size
                    );
                    return Action::Continue;
                }
            };

            if streaming_chunk.len() != body_size {
                warn!(
                    "chunk size mismatch: read: {} != requested: {}",
                    streaming_chunk.len(),
                    body_size
                );
            }

            streaming_chunk
        } else {
            info!("non streaming response bytes read: 0:{}", body_size);
            match self.get_http_response_body(0, body_size) {
                Some(body) => body,
                None => {
                    warn!("non streaming response body empty");
                    return Action::Continue;
                }
            }
        };

        let body_utf8 = match String::from_utf8(body) {
            Ok(body_utf8) => body_utf8,
            Err(e) => {
                info!("could not convert to utf8: {}", e);
                return Action::Continue;
            }
        };

        if self.streaming_response {
            debug!("streaming response");

            if self.tool_calls.is_some() && !self.tool_calls.as_ref().unwrap().is_empty() {
                let chunks = vec![
                    ChatCompletionStreamResponse::new(
                        self.arch_fc_response.clone(),
                        Some(ASSISTANT_ROLE.to_string()),
                        Some(ARCH_FC_MODEL_NAME.to_string()),
                        None,
                    ),
                    ChatCompletionStreamResponse::new(
                        self.tool_call_response.clone(),
                        Some(TOOL_ROLE.to_string()),
                        Some(ARCH_FC_MODEL_NAME.to_string()),
                        None,
                    ),
                ];

                let mut response_str = open_ai::to_server_events(chunks);
                // append the original response from the model to the stream
                response_str.push_str(&body_utf8);
                self.set_http_response_body(0, body_size, response_str.as_bytes());
                self.tool_calls = None;
            }
        } else if let Some(tool_calls) = self.tool_calls.as_ref() {
            if !tool_calls.is_empty() {
                if self.arch_state.is_none() {
                    self.arch_state = Some(Vec::new());
                }

                let mut data = match serde_json::from_str(&body_utf8) {
                    Ok(data) => data,
                    Err(e) => {
                        warn!(
                            "could not deserialize response, sending data as it is: {}",
                            e
                        );
                        return Action::Continue;
                    }
                };
                // use serde::Value to manipulate the json object and ensure that we don't lose any data
                if let Value::Object(ref mut map) = data {
                    // serialize arch state and add to metadata
                    let metadata = map
                        .entry("metadata")
                        .or_insert(Value::Object(serde_json::Map::new()));
                    if metadata == &Value::Null {
                        *metadata = Value::Object(serde_json::Map::new());
                    }

                    let tool_call_message = self.generate_tool_call_message();
                    let tool_call_message_str = serde_json::to_string(&tool_call_message).unwrap();
                    metadata.as_object_mut().unwrap().insert(
                        X_ARCH_TOOL_CALL.to_string(),
                        serde_json::Value::String(tool_call_message_str),
                    );

                    let api_response_message = self.generate_api_response_message();
                    let api_response_message_str =
                        serde_json::to_string(&api_response_message).unwrap();
                    metadata.as_object_mut().unwrap().insert(
                        X_ARCH_API_RESPONSE.to_string(),
                        serde_json::Value::String(api_response_message_str),
                    );

                    let fc_messages = vec![tool_call_message, api_response_message];

                    let fc_messages_str = serde_json::to_string(&fc_messages).unwrap();
                    let arch_state = HashMap::from([("messages".to_string(), fc_messages_str)]);
                    let arch_state_str = serde_json::to_string(&arch_state).unwrap();
                    metadata.as_object_mut().unwrap().insert(
                        X_ARCH_STATE_HEADER.to_string(),
                        serde_json::Value::String(arch_state_str),
                    );

                    if let Some(arch_fc_response) = self.arch_fc_response.as_ref() {
                        metadata.as_object_mut().unwrap().insert(
                            X_ARCH_FC_MODEL_RESPONSE.to_string(),
                            serde_json::Value::String(
                                serde_json::to_string(arch_fc_response).unwrap(),
                            ),
                        );
                    }
                    let data_serialized = serde_json::to_string(&data).unwrap();
                    info!("archgw <= developer: {}", data_serialized);
                    self.set_http_response_body(0, body_size, data_serialized.as_bytes());
                };
            }
        }

        debug!("recv [S={}] end_stream={}", self.context_id, end_of_stream);

        Action::Continue
    }
}
