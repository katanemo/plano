use crate::metrics::Metrics;
use crate::tools::compute_request_path_body;
use common::api::open_ai::{
    to_server_events, ArchState, ChatCompletionStreamResponse, ChatCompletionsRequest,
    ChatCompletionsResponse, ContentType, Message, ToolCall,
};
use common::configuration::{Endpoint, Overrides, PromptTarget, Tracing};
use common::consts::{
    API_REQUEST_TIMEOUT_MS, ARCH_FC_MODEL_NAME, ARCH_INTERNAL_CLUSTER_NAME,
    ARCH_UPSTREAM_HOST_HEADER, ASSISTANT_ROLE, DEFAULT_TARGET_REQUEST_TIMEOUT_MS, MESSAGES_KEY,
    REQUEST_ID_HEADER, SYSTEM_ROLE, TOOL_ROLE, TRACE_PARENT_HEADER, USER_ROLE,
    X_ARCH_FC_MODEL_RESPONSE,
};
use common::errors::ServerError;
use common::http::{CallArgs, Client};
use common::stats::Gauge;
use derivative::Derivative;
use http::StatusCode;
use log::{debug, info, warn};
use proxy_wasm::traits::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub enum ResponseHandlerType {
    ArchFC,
    FunctionCall,
    DefaultTarget,
}

#[derive(Clone, Derivative)]
#[derivative(Debug)]
pub struct StreamCallContext {
    pub response_handler_type: ResponseHandlerType,
    pub user_message: Option<String>,
    pub prompt_target_name: Option<String>,
    #[derivative(Debug = "ignore")]
    pub request_body: ChatCompletionsRequest,
    pub similarity_scores: Option<Vec<(String, f64)>>,
    pub upstream_cluster: Option<String>,
    pub upstream_cluster_path: Option<String>,
}

pub struct StreamContext {
    system_prompt: Rc<Option<String>>,
    pub prompt_targets: Rc<HashMap<String, PromptTarget>>,
    pub endpoints: Rc<Option<HashMap<String, Endpoint>>>,
    pub overrides: Rc<Option<Overrides>>,
    pub metrics: Rc<Metrics>,
    pub callouts: RefCell<HashMap<u32, StreamCallContext>>,
    pub context_id: u32,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_call_response: Option<String>,
    pub arch_state: Option<Vec<ArchState>>,
    pub request_body_size: usize,
    pub user_prompt: Option<Message>,
    pub streaming_response: bool,
    pub is_chat_completions_request: bool,
    pub chat_completions_request: Option<ChatCompletionsRequest>,
    pub request_id: Option<String>,
    pub start_upstream_llm_request_time: u128,
    pub time_to_first_token: Option<u128>,
    pub traceparent: Option<String>,
    pub _tracing: Rc<Option<Tracing>>,
    pub arch_fc_response: Option<String>,
}

impl StreamContext {
    pub fn new(
        context_id: u32,
        metrics: Rc<Metrics>,
        system_prompt: Rc<Option<String>>,
        prompt_targets: Rc<HashMap<String, PromptTarget>>,
        endpoints: Rc<Option<HashMap<String, Endpoint>>>,
        overrides: Rc<Option<Overrides>>,
        tracing: Rc<Option<Tracing>>,
    ) -> Self {
        StreamContext {
            context_id,
            metrics,
            system_prompt,
            prompt_targets,
            endpoints,
            callouts: RefCell::new(HashMap::new()),
            chat_completions_request: None,
            tool_calls: None,
            tool_call_response: None,
            arch_state: None,
            request_body_size: 0,
            streaming_response: false,
            user_prompt: None,
            is_chat_completions_request: false,
            overrides,
            request_id: None,
            traceparent: None,
            _tracing: tracing,
            start_upstream_llm_request_time: 0,
            time_to_first_token: None,
            arch_fc_response: None,
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

    fn _trace_arch_internal(&self) -> bool {
        match self._tracing.as_ref() {
            Some(tracing) => match tracing.trace_arch_internal.as_ref() {
                Some(trace_arch_internal) => *trace_arch_internal,
                None => false,
            },
            None => false,
        }
    }

    pub fn arch_fc_response_handler(
        &mut self,
        body: Vec<u8>,
        mut callout_context: StreamCallContext,
    ) {
        let body_str = String::from_utf8(body).unwrap();
        info!("on_http_call_response: model server response received");
        debug!("response body: {}", body_str);

        let model_server_response: ChatCompletionsResponse = match serde_json::from_str(&body_str) {
            Ok(arch_fc_response) => arch_fc_response,
            Err(e) => {
                warn!(
                    "error deserializing modelserver response: {}, body: {}",
                    e, body_str
                );
                return self.send_server_error(ServerError::Deserialization(e), None);
            }
        };

        let intent_matched = check_intent_matched(&model_server_response);
        info!("intent matched: {}", intent_matched);

        self.arch_fc_response = model_server_response
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get(X_ARCH_FC_MODEL_RESPONSE))
            .cloned();

        if !intent_matched {
            // check if we have a default prompt target
            if let Some(default_prompt_target) = self
                .prompt_targets
                .values()
                .find(|pt| pt.default.unwrap_or(false))
            {
                info!("default prompt target found, forwarding request to default prompt target");
                let endpoint = default_prompt_target.endpoint.clone().unwrap();
                let upstream_path: String = endpoint.path.unwrap_or(String::from("/"));

                let upstream_endpoint = endpoint.name;
                let mut params = HashMap::new();
                params.insert(
                    MESSAGES_KEY.to_string(),
                    callout_context.request_body.messages.clone(),
                );
                let arch_messages_json = serde_json::to_string(&params).unwrap();
                let timeout_str = DEFAULT_TARGET_REQUEST_TIMEOUT_MS.to_string();

                let mut headers = vec![
                    (":method", "POST"),
                    (ARCH_UPSTREAM_HOST_HEADER, &upstream_endpoint),
                    (":path", &upstream_path),
                    (":authority", &upstream_endpoint),
                    ("content-type", "application/json"),
                    ("x-envoy-max-retries", "3"),
                    ("x-envoy-upstream-rq-timeout-ms", timeout_str.as_str()),
                ];

                if let Some(request_id) = &self.request_id {
                    headers.push((REQUEST_ID_HEADER, request_id));
                }

                let call_args = CallArgs::new(
                    ARCH_INTERNAL_CLUSTER_NAME,
                    &upstream_path,
                    headers,
                    Some(arch_messages_json.as_bytes()),
                    vec![],
                    Duration::from_secs(5),
                );
                callout_context.response_handler_type = ResponseHandlerType::DefaultTarget;
                callout_context.prompt_target_name = Some(default_prompt_target.name.clone());

                if let Err(e) = self.http_call(call_args, callout_context) {
                    warn!("error dispatching default prompt target request: {}", e);
                    return self.send_server_error(
                        ServerError::HttpDispatch(e),
                        Some(StatusCode::BAD_REQUEST),
                    );
                }
                return;
            } else {
                info!("no default prompt target found, forwarding request to upstream llm");
                let mut messages = Vec::new();
                // add system prompt
                match self.system_prompt.as_ref() {
                    None => {}
                    Some(system_prompt) => {
                        let system_prompt_message = Message {
                            role: SYSTEM_ROLE.to_string(),
                            content: Some(ContentType::Text(system_prompt.clone())),
                            model: None,
                            tool_calls: None,
                            tool_call_id: None,
                        };
                        messages.push(system_prompt_message);
                    }
                }

                messages.append(
                    &mut self
                        .filter_out_arch_messages(callout_context.request_body.messages.as_ref()),
                );

                let chat_completion_request = ChatCompletionsRequest {
                    model: self
                        .chat_completions_request
                        .as_ref()
                        .unwrap()
                        .model
                        .clone(),
                    messages,
                    tools: None,
                    stream: callout_context.request_body.stream,
                    stream_options: callout_context.request_body.stream_options,
                    metadata: None,
                };

                let chat_completion_request_json =
                    serde_json::to_string(&chat_completion_request).unwrap();
                info!(
                    "plano => upstream llm request: {}",
                    chat_completion_request_json
                );
                self.set_http_request_body(
                    0,
                    self.request_body_size,
                    chat_completion_request_json.as_bytes(),
                );
                self.resume_http_request();
                return;
            }
        }

        model_server_response.choices[0]
            .message
            .tool_calls
            .clone_into(&mut self.tool_calls);

        if self.tool_calls.is_none() || self.tool_calls.as_ref().unwrap().is_empty() {
            // This means that Arch FC did not have enough information to resolve the function call
            // Arch FC probably responded with a message asking for more information.
            // Let's send the response back to the user to initialize lightweight dialog for parameter collection

            //TODO: add resolver name to the response so the client can send the response back to the correct resolver

            let direct_response_str = if self.streaming_response {
                let content = model_server_response.choices[0]
                    .message
                    .content
                    .as_ref()
                    .unwrap()
                    .clone();

                let chunks = vec![
                    ChatCompletionStreamResponse::new(
                        self.arch_fc_response.clone(),
                        Some(ASSISTANT_ROLE.to_string()),
                        Some(ARCH_FC_MODEL_NAME.to_string()),
                        None,
                    ),
                    ChatCompletionStreamResponse::new(
                        Some(content.to_string()),
                        None,
                        Some(format!("{}-Chat", ARCH_FC_MODEL_NAME.to_owned())),
                        None,
                    ),
                ];

                to_server_events(chunks)
            } else {
                body_str
            };

            self.tool_calls = None;
            return self.send_http_response(
                StatusCode::OK.as_u16().into(),
                vec![],
                Some(direct_response_str.as_bytes()),
            );
        }

        // At this point, we know tool_calls is not None and not empty
        if self.tool_calls.as_ref().unwrap().len() > 1 {
            warn!(
                "multiple tool calls not supported yet, tool_calls count found: {}",
                self.tool_calls.as_ref().unwrap().len()
            );
        }

        // update prompt target name from the tool call response
        callout_context.prompt_target_name =
            Some(self.tool_calls.as_ref().unwrap()[0].function.name.clone());

        if let Some(overrides) = self.overrides.as_ref() {
            if overrides.use_agent_orchestrator.unwrap_or_default() {
                let mut metadata = HashMap::new();
                metadata.insert("use_agent_orchestrator".to_string(), "true".to_string());

                metadata.insert(
                    "agent-name".to_string(),
                    callout_context
                        .prompt_target_name
                        .as_ref()
                        .unwrap()
                        .to_string(),
                );

                if let Some(overrides) = self.overrides.as_ref() {
                    if overrides.optimize_context_window.unwrap_or_default() {
                        metadata.insert("optimize_context_window".to_string(), "true".to_string());
                    }
                }

                if let Some(overrides) = self.overrides.as_ref() {
                    if overrides.use_agent_orchestrator.unwrap_or_default() {
                        metadata.insert("use_agent_orchestrator".to_string(), "true".to_string());
                    }
                }

                let messages = self.construct_llm_messages(&callout_context);

                let chat_completion_request = ChatCompletionsRequest {
                    model: callout_context.request_body.model.clone(),
                    messages,
                    tools: None,
                    stream: callout_context.request_body.stream,
                    stream_options: callout_context.request_body.stream_options.clone(),
                    metadata: Some(metadata),
                };

                let body_str = serde_json::to_string(&chat_completion_request).unwrap();
                info!("sending request to llm agent: {}", body_str);
                self.set_http_request_body(0, self.request_body_size, body_str.as_bytes());
                self.resume_http_request();
                return;
            }
        }

        self.schedule_api_call_request(callout_context);
    }

    fn schedule_api_call_request(&mut self, mut callout_context: StreamCallContext) {
        // Construct messages early to avoid mutable borrow conflicts

        let tools_call_name = self.tool_calls.as_ref().unwrap()[0].function.name.clone();
        let prompt_target = self.prompt_targets.get(&tools_call_name).unwrap().clone();
        let tool_params_str = &self.tool_calls.as_ref().unwrap()[0].function.arguments;

        // Parse arguments JSON string into HashMap
        // Note: convert from serde_json::Value to serde_yaml::Value for compatibility
        let tool_params: Option<HashMap<String, serde_yaml::Value>> =
            match serde_json::from_str::<HashMap<String, serde_json::Value>>(tool_params_str) {
                Ok(json_params) => {
                    let yaml_params: HashMap<String, serde_yaml::Value> = json_params
                        .into_iter()
                        .filter_map(|(k, v)| {
                            serde_yaml::to_value(&v).ok().map(|yaml_v| (k, yaml_v))
                        })
                        .collect();
                    Some(yaml_params)
                }
                Err(e) => {
                    warn!("Failed to parse tool call arguments: {}", e);
                    None
                }
            };

        let endpoint_details = prompt_target.endpoint.as_ref().unwrap();
        let endpoint_path: String = endpoint_details
            .path
            .as_ref()
            .unwrap_or(&String::from("/"))
            .to_string();

        let http_method = endpoint_details.method.clone().unwrap_or_default();
        let prompt_target_params = prompt_target.parameters.clone().unwrap_or_default();

        let (path, api_call_body) = match compute_request_path_body(
            &endpoint_path,
            &tool_params,
            &prompt_target_params,
            &http_method,
        ) {
            Ok((path, body)) => (path, body),
            Err(e) => {
                return self.send_server_error(
                    ServerError::BadRequest {
                        why: format!("error computing api request path or body: {}", e),
                    },
                    Some(StatusCode::BAD_REQUEST),
                );
            }
        };

        debug!("on_http_call_response: api call body {:?}", api_call_body);

        let timeout_str = API_REQUEST_TIMEOUT_MS.to_string();

        let http_method_str = http_method.to_string();
        let mut headers: HashMap<_, _> = [
            (ARCH_UPSTREAM_HOST_HEADER, endpoint_details.name.as_str()),
            (":method", &http_method_str),
            (":path", &path),
            (":authority", endpoint_details.name.as_str()),
            ("content-type", "application/json"),
            ("x-envoy-max-retries", "3"),
            ("x-envoy-upstream-rq-timeout-ms", timeout_str.as_str()),
        ]
        .into_iter()
        .collect();

        if let Some(request_id) = &self.request_id {
            headers.insert(REQUEST_ID_HEADER, request_id);
        }

        if let Some(traceparent) = &self.traceparent {
            headers.insert(TRACE_PARENT_HEADER, traceparent);
        }

        // override http headers that are set in the prompt target
        let http_headers = endpoint_details.http_headers.clone().unwrap_or_default();
        for (key, value) in http_headers.iter() {
            headers.insert(key.as_str(), value.as_str());
        }

        let call_args = CallArgs::new(
            ARCH_INTERNAL_CLUSTER_NAME,
            &path,
            headers.into_iter().collect(),
            api_call_body.as_deref().map(|s| s.as_bytes()),
            vec![],
            Duration::from_secs(5),
        );

        info!(
            "on_http_call_response: dispatching api call to developer endpoint: {}, path: {}, method: {}",
            endpoint_details.name, path, http_method_str
        );

        callout_context.upstream_cluster = Some(endpoint_details.name.to_owned());
        callout_context.upstream_cluster_path = Some(path.to_owned());
        callout_context.response_handler_type = ResponseHandlerType::FunctionCall;

        if let Err(e) = self.http_call(call_args, callout_context) {
            self.send_server_error(ServerError::HttpDispatch(e), Some(StatusCode::BAD_REQUEST));
        }
    }

    pub fn api_call_response_handler(&mut self, body: Vec<u8>, callout_context: StreamCallContext) {
        let http_status = self
            .get_http_call_response_header(":status")
            .unwrap_or(StatusCode::OK.as_str().to_string());
        info!(
            "on_http_call_response: developer api call response received: status code: {}",
            http_status
        );
        let prompt_target = self
            .prompt_targets
            .get(callout_context.prompt_target_name.as_ref().unwrap())
            .unwrap()
            .clone();
        if http_status != StatusCode::OK.as_str() {
            warn!(
                "api server responded with non 2xx status code: {}",
                http_status
            );
            return self.send_server_error(
                ServerError::Upstream {
                    host: callout_context.upstream_cluster.unwrap(),
                    path: callout_context.upstream_cluster_path.unwrap(),
                    status: http_status.clone(),
                    body: String::from_utf8(body).unwrap(),
                },
                Some(StatusCode::from_str(http_status.as_str()).unwrap()),
            );
        }
        self.tool_call_response = Some(String::from_utf8(body).unwrap());
        debug!(
            "response body: {}",
            self.tool_call_response.as_ref().unwrap()
        );

        let mut messages = self.construct_llm_messages(&callout_context);

        let user_message = match messages.pop() {
            Some(user_message) => user_message,
            None => {
                return self.send_server_error(
                    ServerError::NoMessagesFound {
                        why: "no user messages found".to_string(),
                    },
                    None,
                );
            }
        };

        if !prompt_target.auto_llm_dispatch_on_response.unwrap_or(true) {
            let tool_call_response = self.tool_call_response.as_ref().unwrap().clone();

            let direct_response_str = if self.streaming_response {
                let chunks = vec![
                    ChatCompletionStreamResponse::new(
                        None,
                        Some(ASSISTANT_ROLE.to_string()),
                        Some(ARCH_FC_MODEL_NAME.to_owned()),
                        None,
                    ),
                    ChatCompletionStreamResponse::new(
                        Some(tool_call_response.clone()),
                        None,
                        Some(ARCH_FC_MODEL_NAME.to_owned()),
                        None,
                    ),
                ];

                to_server_events(chunks)
            } else {
                tool_call_response
            };

            return self.send_http_response(
                StatusCode::OK.as_u16().into(),
                vec![],
                Some(direct_response_str.as_bytes()),
            );
        }

        let final_prompt = format!(
            "{}\ncontext: {}",
            user_message.content.unwrap(),
            self.tool_call_response.as_ref().unwrap()
        );

        // add original user prompt
        messages.push({
            Message {
                role: USER_ROLE.to_string(),
                content: Some(ContentType::Text(final_prompt)),
                model: None,
                tool_calls: None,
                tool_call_id: None,
            }
        });

        let chat_completions_request: ChatCompletionsRequest = ChatCompletionsRequest {
            model: callout_context.request_body.model,
            messages,
            tools: None,
            stream: callout_context.request_body.stream,
            stream_options: callout_context.request_body.stream_options,
            metadata: None,
        };

        let llm_request_str = match serde_json::to_string(&chat_completions_request) {
            Ok(json_string) => json_string,
            Err(e) => {
                return self.send_server_error(ServerError::Serialization(e), None);
            }
        };
        info!("on_http_call_response: sending request to upstream llm");
        debug!("request body: {}", llm_request_str);

        self.start_upstream_llm_request_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        self.set_http_request_body(0, self.request_body_size, &llm_request_str.into_bytes());
        self.resume_http_request();
    }

    fn get_system_prompt(&self, prompt_target: Option<PromptTarget>) -> Option<String> {
        match prompt_target {
            None => self.system_prompt.as_ref().clone(),
            Some(prompt_target) => match prompt_target.system_prompt {
                None => self.system_prompt.as_ref().clone(),
                Some(system_prompt) => Some(system_prompt),
            },
        }
    }

    fn filter_out_arch_messages(&self, messages: &[Message]) -> Vec<Message> {
        messages
            .iter()
            .filter(|m| {
                !(m.role == TOOL_ROLE
                    || m.content.is_none()
                    || (m.tool_calls.is_some() && !m.tool_calls.as_ref().unwrap().is_empty()))
            })
            .cloned()
            .collect()
    }

    fn construct_llm_messages(&mut self, callout_context: &StreamCallContext) -> Vec<Message> {
        let mut messages: Vec<Message> = Vec::new();

        // add system prompt
        let system_prompt = match callout_context.prompt_target_name.as_ref() {
            None => self.system_prompt.as_ref().clone(),
            Some(prompt_target_name) => {
                self.get_system_prompt(self.prompt_targets.get(prompt_target_name).cloned())
            }
        };

        if let Some(system_prompt_text) = system_prompt {
            let system_prompt_message = Message {
                role: SYSTEM_ROLE.to_string(),
                content: Some(ContentType::Text(system_prompt_text)),
                model: None,
                tool_calls: None,
                tool_call_id: None,
            };
            messages.push(system_prompt_message);
        }

        messages.append(
            &mut self.filter_out_arch_messages(callout_context.request_body.messages.as_ref()),
        );
        messages
    }

    pub fn generate_tool_call_message(&mut self) -> Message {
        if let Some(arch_fc_response) = &self.arch_fc_response {
            Message {
                role: ASSISTANT_ROLE.to_string(),
                content: Some(ContentType::Text(arch_fc_response.clone())),
                model: Some(ARCH_FC_MODEL_NAME.to_string()),
                tool_calls: None,
                tool_call_id: None,
            }
        } else {
            info!("arch_fc_response is none, generating tool call message");
            Message {
                role: ASSISTANT_ROLE.to_string(),
                content: None,
                model: Some(ARCH_FC_MODEL_NAME.to_string()),
                tool_calls: self.tool_calls.clone(),
                tool_call_id: None,
            }
        }
    }

    pub fn generate_api_response_message(&mut self) -> Message {
        Message {
            role: TOOL_ROLE.to_string(),
            content: Some(ContentType::Text(
                self.tool_call_response.as_ref().unwrap().clone(),
            )),
            model: None,
            tool_calls: None,
            tool_call_id: Some(self.tool_calls.as_ref().unwrap()[0].id.clone()),
        }
    }

    pub fn default_target_handler(&self, body: Vec<u8>, mut callout_context: StreamCallContext) {
        let prompt_target = self
            .prompt_targets
            .get(callout_context.prompt_target_name.as_ref().unwrap())
            .unwrap()
            .clone();

        // check if the default target should be dispatched to the LLM provider
        if !prompt_target.auto_llm_dispatch_on_response.unwrap_or(true) {
            let default_target_response_str = if self.streaming_response {
                let chat_completion_response =
                    match serde_json::from_slice::<ChatCompletionsResponse>(&body) {
                        Ok(chat_completion_response) => chat_completion_response,
                        Err(e) => {
                            warn!(
                                "error deserializing default target response: {}, body str: {}",
                                e,
                                String::from_utf8(body).unwrap()
                            );
                            return self.send_server_error(ServerError::Deserialization(e), None);
                        }
                    };

                let chunks = vec![
                    ChatCompletionStreamResponse::new(
                        None,
                        Some(ASSISTANT_ROLE.to_string()),
                        Some(chat_completion_response.model.clone()),
                        None,
                    ),
                    ChatCompletionStreamResponse::new(
                        Some(
                            chat_completion_response.choices[0]
                                .message
                                .content
                                .as_ref()
                                .unwrap()
                                .to_string(),
                        ),
                        None,
                        Some(chat_completion_response.model.clone()),
                        None,
                    ),
                ];

                to_server_events(chunks)
            } else {
                String::from_utf8(body).unwrap()
            };

            self.send_http_response(
                StatusCode::OK.as_u16().into(),
                vec![],
                Some(default_target_response_str.as_bytes()),
            );
            return;
        }

        let chat_completions_resp: ChatCompletionsResponse = match serde_json::from_slice(&body) {
            Ok(chat_completions_resp) => chat_completions_resp,
            Err(e) => {
                warn!(
                    "error deserializing default target response: {}, body str: {}",
                    e,
                    String::from_utf8(body).unwrap()
                );
                return self.send_server_error(ServerError::Deserialization(e), None);
            }
        };

        let mut messages = Vec::new();
        // add system prompt
        match prompt_target.system_prompt.as_ref() {
            None => {}
            Some(system_prompt) => {
                let system_prompt_message = Message {
                    role: SYSTEM_ROLE.to_string(),
                    content: Some(ContentType::Text(system_prompt.clone())),
                    model: None,
                    tool_calls: None,
                    tool_call_id: None,
                };
                messages.push(system_prompt_message);
            }
        }

        messages.append(&mut callout_context.request_body.messages);

        let api_resp = chat_completions_resp.choices[0]
            .message
            .content
            .as_ref()
            .unwrap();

        let user_message = messages.pop().unwrap();
        let message = format!("{}\ncontext: {}", user_message.content.unwrap(), api_resp);
        messages.push(Message {
            role: USER_ROLE.to_string(),
            content: Some(ContentType::Text(message)),
            model: None,
            tool_calls: None,
            tool_call_id: None,
        });

        let chat_completion_request = ChatCompletionsRequest {
            model: self
                .chat_completions_request
                .as_ref()
                .unwrap()
                .model
                .clone(),
            messages,
            tools: None,
            stream: callout_context.request_body.stream,
            stream_options: callout_context.request_body.stream_options,
            metadata: None,
        };

        let json_resp = serde_json::to_string(&chat_completion_request).unwrap();
        info!("plano => (default target) llm request: {}", json_resp);
        self.set_http_request_body(0, self.request_body_size, json_resp.as_bytes());
        self.resume_http_request();
    }
}

fn check_intent_matched(model_server_response: &ChatCompletionsResponse) -> bool {
    let content = model_server_response
        .choices
        .first()
        .and_then(|choice| choice.message.content.as_ref());

    let content_has_value = content.is_some() && !content.unwrap().to_string().is_empty();

    let tool_calls = model_server_response
        .choices
        .first()
        .and_then(|choice| choice.message.tool_calls.as_ref());

    // intent was matched if content has some value or tool_calls is empty

    content_has_value || (tool_calls.is_some() && !tool_calls.unwrap().is_empty())
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

#[cfg(test)]
mod test {
    use common::api::open_ai::{ChatCompletionsResponse, Choice, ContentType, Message, ToolCall};

    use crate::stream_context::check_intent_matched;

    #[test]
    fn test_intent_matched() {
        let model_server_response = ChatCompletionsResponse {
            choices: vec![Choice {
                message: Message {
                    content: Some(ContentType::Text("".to_string())),
                    tool_calls: Some(vec![]),
                    role: "assistant".to_string(),
                    model: None,
                    tool_call_id: None,
                },
                finish_reason: None,
                index: None,
            }],
            usage: None,
            model: "arch-fc".to_string(),
            metadata: None,
        };

        assert!(!check_intent_matched(&model_server_response));

        let model_server_response = ChatCompletionsResponse {
            choices: vec![Choice {
                message: Message {
                    content: Some(ContentType::Text("hello".to_string())),
                    tool_calls: Some(vec![]),
                    role: "assistant".to_string(),
                    model: None,
                    tool_call_id: None,
                },
                finish_reason: None,
                index: None,
            }],
            usage: None,
            model: "arch-fc".to_string(),
            metadata: None,
        };

        assert!(check_intent_matched(&model_server_response));

        let model_server_response = ChatCompletionsResponse {
            choices: vec![Choice {
                message: Message {
                    content: Some(ContentType::Text("".to_string())),
                    tool_calls: Some(vec![ToolCall {
                        id: "1".to_string(),
                        function: common::api::open_ai::FunctionCallDetail {
                            name: "test".to_string(),
                            arguments: "{}".to_string(),
                        },
                        tool_type: common::api::open_ai::ToolType::Function,
                    }]),
                    role: "assistant".to_string(),
                    model: None,
                    tool_call_id: None,
                },
                finish_reason: None,
                index: None,
            }],
            usage: None,
            model: "arch-fc".to_string(),
            metadata: None,
        };

        assert!(check_intent_matched(&model_server_response));
    }
}
