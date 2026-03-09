use bytes::Bytes;
use common::configuration::{LlmProvider, ModelAlias, SpanAttributes};
use common::consts::{
    ARCH_IS_STREAMING_HEADER, ARCH_PROVIDER_HINT_HEADER, REQUEST_ID_HEADER, TRACE_PARENT_HEADER,
};
use common::llm_providers::LlmProviders;
use common::retry::error_response::build_error_response;
use common::retry::orchestrator::RetryOrchestrator;
use common::retry::{rebuild_request_for_provider, RequestContext, RequestSignature};
use hermesllm::apis::openai_responses::InputParam;
use hermesllm::clients::{SupportedAPIsFromClient, SupportedUpstreamAPIs};
use hermesllm::{ProviderRequest, ProviderRequestType};
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use hyper::header::{self};
use hyper::{Request, Response};
use opentelemetry::global;
use opentelemetry::trace::get_active_span;
use opentelemetry_http::HeaderInjector;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, info_span, warn, Instrument};

use crate::handlers::router_chat::router_chat_get_upstream_model;
use crate::handlers::utils::{
    create_streaming_response, truncate_message, ObservableStreamProcessor,
};
use crate::router::llm_router::RouterService;
use crate::state::response_state_processor::ResponsesStateProcessor;
use crate::state::{
    extract_input_items, retrieve_and_combine_input, StateStorage, StateStorageError,
};
use crate::tracing::{
    collect_custom_trace_attributes, llm as tracing_llm, operation_component, set_service_name,
};

use common::errors::BrightStaffError;

pub async fn llm_chat(
    request: Request<hyper::body::Incoming>,
    router_service: Arc<RouterService>,
    full_qualified_llm_provider_url: String,
    model_aliases: Arc<Option<HashMap<String, ModelAlias>>>,
    llm_providers: Arc<RwLock<LlmProviders>>,
    span_attributes: Arc<Option<SpanAttributes>>,
    state_storage: Option<Arc<dyn StateStorage>>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let request_path = request.uri().path().to_string();
    let request_headers = request.headers().clone();
    let request_id: String = match request_headers
        .get(REQUEST_ID_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
    {
        Some(id) => id,
        None => uuid::Uuid::new_v4().to_string(),
    };
    let custom_attrs =
        collect_custom_trace_attributes(&request_headers, span_attributes.as_ref().as_ref());

    // Create a span with request_id that will be included in all log lines
    let request_span = info_span!(
        "llm",
        component = "llm",
        request_id = %request_id,
        http.method = %request.method(),
        http.path = %request_path,
        llm.model = tracing::field::Empty,
        llm.tools = tracing::field::Empty,
        llm.user_message_preview = tracing::field::Empty,
        llm.temperature = tracing::field::Empty,
    );

    // Execute the rest of the handler inside the span
    llm_chat_inner(
        request,
        router_service,
        full_qualified_llm_provider_url,
        model_aliases,
        llm_providers,
        custom_attrs,
        state_storage,
        request_id,
        request_path,
        request_headers,
    )
    .instrument(request_span)
    .await
}

#[allow(clippy::too_many_arguments)]
async fn llm_chat_inner(
    request: Request<hyper::body::Incoming>,
    router_service: Arc<RouterService>,
    full_qualified_llm_provider_url: String,
    model_aliases: Arc<Option<HashMap<String, ModelAlias>>>,
    llm_providers: Arc<RwLock<LlmProviders>>,
    custom_attrs: HashMap<String, String>,
    state_storage: Option<Arc<dyn StateStorage>>,
    request_id: String,
    request_path: String,
    mut request_headers: hyper::HeaderMap,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    // Set service name for LLM operations
    set_service_name(operation_component::LLM);
    get_active_span(|span| {
        for (key, value) in &custom_attrs {
            span.set_attribute(opentelemetry::KeyValue::new(key.clone(), value.clone()));
        }
    });

    // Extract or generate traceparent - this establishes the trace context for all spans
    let traceparent: String = match request_headers
        .get(TRACE_PARENT_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
    {
        Some(tp) => tp,
        None => {
            use uuid::Uuid;
            let trace_id = Uuid::new_v4().to_string().replace("-", "");
            let generated_tp = format!("00-{}-0000000000000000-01", trace_id);
            warn!(
                generated_traceparent = %generated_tp,
                "TRACE_PARENT header missing, generated new traceparent"
            );
            generated_tp
        }
    };

    let chat_request_bytes = request.collect().await?.to_bytes();

    debug!(
        body = %String::from_utf8_lossy(&chat_request_bytes),
        "request body received"
    );

    let mut client_request = match ProviderRequestType::try_from((
        &chat_request_bytes[..],
        &SupportedAPIsFromClient::from_endpoint(request_path.as_str()).unwrap(),
    )) {
        Ok(request) => request,
        Err(err) => {
            warn!(
                error = %err,
                "failed to parse request as ProviderRequestType"
            );
            return Ok(BrightStaffError::InvalidRequest(format!(
                "Failed to parse request: {}",
                err
            ))
            .into_response());
        }
    };

    // === v1/responses state management: Extract input items early ===
    let mut original_input_items = Vec::new();
    let client_api = SupportedAPIsFromClient::from_endpoint(request_path.as_str());
    let is_responses_api_client = matches!(
        client_api,
        Some(SupportedAPIsFromClient::OpenAIResponsesAPI(_))
    );

    // If model is not specified in the request, resolve from default provider
    let model_from_request = client_request.model().to_string();
    let model_from_request = if model_from_request.is_empty() {
        match llm_providers.read().await.default() {
            Some(default_provider) => {
                let default_model = default_provider.name.clone();
                info!(default_model = %default_model, "no model specified in request, using default provider");
                client_request.set_model(default_model.clone());
                default_model
            }
            None => {
                let err_msg = "No model specified in request and no default provider configured";
                warn!("{}", err_msg);
                return Ok(BrightStaffError::NoModelSpecified.into_response());
            }
        }
    } else {
        model_from_request
    };

    // Model alias resolution: update model field in client_request immediately
    // This ensures all downstream objects use the resolved model
    let temperature = client_request.get_temperature();
    let is_streaming_request = client_request.is_streaming();
    let alias_resolved_model = resolve_model_alias(&model_from_request, &model_aliases);

    // Validate that the requested model exists in configuration
    // This matches the validation in llm_gateway routing.rs
    if llm_providers
        .read()
        .await
        .get(&alias_resolved_model)
        .is_none()
    {
        warn!(model = %alias_resolved_model, "model not found in configured providers");
        return Ok(BrightStaffError::ModelNotFound(alias_resolved_model).into_response());
    }

    // Handle provider/model slug format (e.g., "openai/gpt-4")
    // Extract just the model name for upstream (providers don't understand the slug)
    let model_name_only = if let Some((_, model)) = alias_resolved_model.split_once('/') {
        model.to_string()
    } else {
        alias_resolved_model.clone()
    };

    // Extract tool names and user message preview for span attributes
    let tool_names = client_request.get_tool_names();
    let user_message_preview = client_request
        .get_recent_user_message()
        .map(|msg| truncate_message(&msg, 50));
    let span = tracing::Span::current();
    if let Some(temp) = temperature {
        span.record(tracing_llm::TEMPERATURE, tracing::field::display(temp));
    }
    if let Some(tools) = &tool_names {
        let formatted_tools = tools
            .iter()
            .map(|name| format!("{}(...)", name))
            .collect::<Vec<_>>()
            .join("\n");
        span.record(tracing_llm::TOOLS, formatted_tools.as_str());
    }
    if let Some(preview) = &user_message_preview {
        span.record(tracing_llm::USER_MESSAGE_PREVIEW, preview.as_str());
    }

    // Extract messages for signal analysis (clone before moving client_request)
    let messages_for_signals = Some(client_request.get_messages());

    // Set the model to just the model name (without provider prefix)
    // This ensures upstream receives "gpt-4" not "openai/gpt-4"
    client_request.set_model(model_name_only.clone());
    if client_request.remove_metadata_key("plano_preference_config") {
        debug!("removed plano_preference_config from metadata");
    }

    // === v1/responses state management: Determine upstream API and combine input if needed ===
    // Do this BEFORE routing since routing consumes the request
    // Only process state if state_storage is configured
    let mut should_manage_state = false;
    if is_responses_api_client {
        if let (
            ProviderRequestType::ResponsesAPIRequest(ref mut responses_req),
            Some(ref state_store),
        ) = (&mut client_request, &state_storage)
        {
            // Extract original input once
            original_input_items = extract_input_items(&responses_req.input);

            // Get the upstream path and check if it's ResponsesAPI
            let upstream_path = get_upstream_path(
                &llm_providers,
                &alias_resolved_model,
                &request_path,
                &alias_resolved_model,
                is_streaming_request,
            )
            .await;

            let upstream_api = SupportedUpstreamAPIs::from_endpoint(&upstream_path);

            // Only manage state if upstream is NOT OpenAIResponsesAPI (needs translation)
            should_manage_state = !matches!(
                upstream_api,
                Some(SupportedUpstreamAPIs::OpenAIResponsesAPI(_))
            );

            if should_manage_state {
                // Retrieve and combine conversation history if previous_response_id exists
                if let Some(ref prev_resp_id) = responses_req.previous_response_id {
                    match retrieve_and_combine_input(
                        state_store.clone(),
                        prev_resp_id,
                        original_input_items, // Pass ownership instead of cloning
                    )
                    .await
                    {
                        Ok(combined_input) => {
                            // Update both the request and original_input_items
                            responses_req.input = InputParam::Items(combined_input.clone());
                            original_input_items = combined_input;
                            info!(
                                items = original_input_items.len(),
                                "updated request with conversation history"
                            );
                        }
                        Err(StateStorageError::NotFound(_)) => {
                            // Return 409 Conflict when previous_response_id not found
                            warn!(previous_response_id = %prev_resp_id, "previous response_id not found");
                            return Ok(BrightStaffError::ConversationStateNotFound(
                                prev_resp_id.to_string(),
                            )
                            .into_response());
                        }
                        Err(e) => {
                            // Log warning but continue on other storage errors
                            warn!(
                                previous_response_id = %prev_resp_id,
                                error = %e,
                                "failed to retrieve conversation state"
                            );
                            // Restore original_input_items since we passed ownership
                            original_input_items = extract_input_items(&responses_req.input);
                        }
                    }
                }
            } else {
                debug!("upstream supports ResponsesAPI natively");
            }
        }
    }

    // Serialize request for upstream BEFORE router consumes it
    let client_request_bytes_for_upstream = ProviderRequestType::to_bytes(&client_request).unwrap();

    // Determine routing using the dedicated router_chat module
    // This gets its own span for latency and error tracking
    let routing_span = info_span!(
        "routing",
        component = "routing",
        http.method = "POST",
        http.target = %request_path,
        model.requested = %model_from_request,
        model.alias_resolved = %alias_resolved_model,
        route.selected_model = tracing::field::Empty,
        routing.determination_ms = tracing::field::Empty,
    );
    let routing_result = match async {
        set_service_name(operation_component::ROUTING);
        router_chat_get_upstream_model(
            router_service,
            client_request, // Pass the original request - router_chat will convert it
            &traceparent,
            &request_path,
            &request_id,
        )
        .await
    }
    .instrument(routing_span)
    .await
    {
        Ok(result) => result,
        Err(err) => {
            return Ok(BrightStaffError::ForwardedError {
                status_code: err.status_code,
                message: err.message,
            }
            .into_response());
        }
    };

    // Determine final model to use
    // Router returns "none" as a sentinel value when it doesn't select a specific model
    let router_selected_model = routing_result.model_name;
    let resolved_model = if router_selected_model != "none" {
        // Router selected a specific model via routing preferences
        router_selected_model
    } else {
        // Router returned "none" sentinel, use validated resolved_model from request
        alias_resolved_model.clone()
    };
    tracing::Span::current().record(tracing_llm::MODEL_NAME, resolved_model.as_str());

    let span_name = if model_from_request == resolved_model {
        format!("POST {} {}", request_path, resolved_model)
    } else {
        format!(
            "POST {} {} -> {}",
            request_path, model_from_request, resolved_model
        )
    };
    get_active_span(|span| {
        span.update_name(span_name.clone());
    });

    debug!(
        url = %full_qualified_llm_provider_url,
        provider_hint = %resolved_model,
        upstream_model = %model_name_only,
        "Routing to upstream"
    );

    request_headers.insert(
        ARCH_PROVIDER_HINT_HEADER,
        header::HeaderValue::from_str(&resolved_model).unwrap(),
    );

    request_headers.insert(
        header::HeaderName::from_static(ARCH_IS_STREAMING_HEADER),
        header::HeaderValue::from_str(&is_streaming_request.to_string()).unwrap(),
    );
    // remove content-length header if it exists
    request_headers.remove(header::CONTENT_LENGTH);

    // Inject current LLM span's trace context so upstream spans are children of plano(llm)
    global::get_text_map_propagator(|propagator| {
        let cx = tracing_opentelemetry::OpenTelemetrySpanExt::context(&tracing::Span::current());
        propagator.inject_context(&cx, &mut HeaderInjector(&mut request_headers));
    });

    // Capture start time right before sending request to upstream
    let request_start_time = std::time::Instant::now();
    let _request_start_system_time = std::time::SystemTime::now();

    // === Retry orchestrator integration ===
    // Check if the resolved provider has a retry_policy configured.
    // If so, use the RetryOrchestrator to wrap the upstream call with retry logic.
    // Otherwise, preserve the existing direct-call behavior unchanged.
    let resolved_provider: Option<Arc<LlmProvider>> =
        llm_providers.read().await.get(&resolved_model);

    let has_retry_policy = resolved_provider
        .as_ref()
        .and_then(|p| p.retry_policy.as_ref())
        .is_some();

    if has_retry_policy {
        let provider = resolved_provider.as_ref().unwrap();
        let retry_policy = provider.retry_policy.as_ref().unwrap();

        // Build the list of all providers for the retry orchestrator
        let all_providers: Vec<LlmProvider> = llm_providers
            .read()
            .await
            .iter()
            .map(|(_, p)| (**p).clone())
            .collect();

        // Create RequestSignature from the original request bytes (computes body hash, does not clone body)
        let request_signature = RequestSignature::new(
            &chat_request_bytes,
            &request_headers,
            is_streaming_request,
            alias_resolved_model.clone(),
        );

        // Create RequestContext with the handler's request_id
        let mut request_context = RequestContext {
            request_id: request_id.clone(),
            attempted_providers: std::collections::HashSet::new(),
            retry_start_time: None,
            attempt_number: 0,
            request_retry_after_state: HashMap::new(),
            request_latency_block_state: HashMap::new(),
            request_signature: request_signature.clone(),
            errors: vec![],
        };

        // Create the retry orchestrator with default state managers (P0)
        let orchestrator = RetryOrchestrator::new_default();

        debug!(
            model = %alias_resolved_model,
            fallback_models = ?retry_policy.fallback_models,
            default_strategy = ?retry_policy.default_strategy,
            default_max_attempts = retry_policy.default_max_attempts,
            "Retry orchestrator initialized for request"
        );

        // Capture references needed by the forward_fn closure
        let base_url = full_qualified_llm_provider_url.clone();
        let original_headers = request_headers.clone();
        let request_path_clone = request_path.clone();

        // The forward_fn closure handles the actual HTTP call to upstream.
        // For each attempt, it rebuilds the request for the target provider
        // (updating model field and auth credentials), then sends the request.
        let forward_fn = |body: &Bytes, target_provider: &LlmProvider| {
            let body = body.clone();
            let target_provider = target_provider.clone();
            let base_url = base_url.clone();
            let original_headers = original_headers.clone();
            let request_path_clone = request_path_clone.clone();
            let primary_model = alias_resolved_model.clone();

            async move {
                // Determine if we're retrying to a different provider or the same one
                let target_model = target_provider
                    .model
                    .as_deref()
                    .unwrap_or(&target_provider.name);

                let (request_body, mut headers) = if target_model == primary_model {
                    // Same provider: use original request bytes and headers
                    (body.clone(), original_headers.clone())
                } else {
                    // Different provider: rebuild request with updated model and auth
                    match rebuild_request_for_provider(&body, &target_provider, &original_headers) {
                        Ok((new_body, new_headers)) => (new_body, new_headers),
                        Err(e) => {
                            warn!(error = %e, "Failed to rebuild request for provider");
                            return Err(common::retry::error_detector::TimeoutError {
                                duration_ms: 0,
                            });
                        }
                    }
                };

                // Resolve the upstream URL for the target provider
                let upstream_url = {
                    let provider_id = target_provider.provider_interface.to_provider_id();
                    let prefix = target_provider.base_url_path_prefix.clone();
                    let target_model_name = target_model
                        .split_once('/')
                        .map(|(_, m)| m)
                        .unwrap_or(target_model);

                    let client_api =
                        SupportedAPIsFromClient::from_endpoint(request_path_clone.as_str());
                    if let Some(api) = client_api {
                        let upstream_path = api.target_endpoint_for_provider(
                            &provider_id,
                            &request_path_clone,
                            target_model_name,
                            target_provider.stream == Some(true),
                            prefix.as_deref(),
                        );
                        // Build the full URL from the target provider's endpoint
                        if let (Some(endpoint), Some(port)) =
                            (&target_provider.endpoint, target_provider.port)
                        {
                            format!("{}:{}{}", endpoint, port, upstream_path)
                        } else if let Some(endpoint) = &target_provider.endpoint {
                            format!("{}{}", endpoint, upstream_path)
                        } else {
                            // Fallback: use the original base URL (same host)
                            base_url.clone()
                        }
                    } else {
                        base_url.clone()
                    }
                };

                // Set provider hint header for the target
                headers.insert(
                    ARCH_PROVIDER_HINT_HEADER,
                    header::HeaderValue::from_str(target_model).unwrap_or_else(|_| {
                        header::HeaderValue::from_static("unknown")
                    }),
                );

                // Respect passthrough_auth per provider
                if target_provider.passthrough_auth != Some(true) {
                    // Auth headers are already set by rebuild_request_for_provider
                    // For same-provider retries, ensure the original auth is used
                }

                // Remove content-length as body may have changed
                headers.remove(header::CONTENT_LENGTH);

                // Send the request
                let result = reqwest::Client::new()
                    .post(&upstream_url)
                    .headers(headers)
                    .body(request_body.to_vec())
                    .send()
                    .await;

                match result {
                    Ok(res) => {
                        // Convert reqwest::Response to HttpResponse (hyper Response<BoxBody>)
                        let status = res.status().as_u16();
                        let resp_headers = res.headers().clone();
                        let body_bytes = res.bytes().await.unwrap_or_default();

                        let full_body = http_body_util::Full::new(body_bytes)
                            .map_err(|never| match never {})
                            .boxed();

                        let mut builder = Response::builder().status(status);
                        if let Some(hdrs) = builder.headers_mut() {
                            for (name, value) in resp_headers.iter() {
                                if let Ok(hyper_name) =
                                    hyper::header::HeaderName::from_bytes(name.as_str().as_bytes())
                                {
                                    if let Ok(hyper_value) =
                                        hyper::header::HeaderValue::from_bytes(value.as_bytes())
                                    {
                                        hdrs.insert(hyper_name, hyper_value);
                                    }
                                }
                            }
                        }

                        Ok(builder.body(full_body).unwrap())
                    }
                    Err(err) => {
                        warn!(error = %err, "Upstream request failed");
                        Err(common::retry::error_detector::TimeoutError {
                            duration_ms: 0,
                        })
                    }
                }
            }
        };

        // Execute the retry orchestrator
        let retry_result = orchestrator
            .execute(
                &chat_request_bytes,
                &request_signature,
                provider,
                retry_policy,
                &all_providers,
                &mut request_context,
                forward_fn,
            )
            .await;

        match retry_result {
            Ok(http_response) => {
                // Success (possibly after retries) — convert HttpResponse back to client response.
                // The retry orchestrator collected the full response body for classification,
                // so we reconstruct the response for the client.
                let upstream_status = http_response.status();
                let response_headers = http_response.headers().clone();

                let mut response = Response::builder().status(upstream_status);
                let headers = response.headers_mut().unwrap();
                for (header_name, header_value) in response_headers.iter() {
                    headers.insert(header_name, header_value.clone());
                }

                // Collect the body from the HttpResponse
                let body_bytes = http_response
                    .into_body()
                    .collect()
                    .await
                    .map(|collected| collected.to_bytes())
                    .unwrap_or_default();

                // Convert to a reqwest-compatible byte stream for create_streaming_response
                let byte_stream = futures::stream::iter(
                    vec![Ok::<Bytes, reqwest::Error>(body_bytes)]
                );

                // Create base processor for metrics and tracing
                let base_processor = ObservableStreamProcessor::new(
                    operation_component::LLM,
                    span_name,
                    request_start_time,
                    messages_for_signals,
                );

                // === v1/responses state management ===
                let streaming_response = if let (true, false, Some(state_store)) = (
                    should_manage_state,
                    original_input_items.is_empty(),
                    &state_storage,
                ) {
                    let content_encoding = response_headers
                        .get("content-encoding")
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.to_string());

                    let state_processor = ResponsesStateProcessor::new(
                        base_processor,
                        state_store.clone(),
                        original_input_items,
                        alias_resolved_model.clone(),
                        resolved_model.clone(),
                        is_streaming_request,
                        false,
                        content_encoding,
                        request_id,
                    );
                    create_streaming_response(byte_stream, state_processor, 16)
                } else {
                    create_streaming_response(byte_stream, base_processor, 16)
                };

                match response.body(streaming_response.body) {
                    Ok(response) => Ok(response),
                    Err(err) => Ok(BrightStaffError::InternalServerError(format!(
                        "Failed to create response: {}",
                        err
                    ))
                    .into_response()),
                }
            }
            Err(retry_exhausted_error) => {
                // All retries exhausted — build error response using the error_response module
                info!(
                    request_id = %request_id,
                    total_attempts = retry_exhausted_error.attempts.len(),
                    budget_exhausted = retry_exhausted_error.retry_budget_exhausted,
                    "All retries exhausted"
                );

                let error_resp = build_error_response(&retry_exhausted_error, &request_id);

                // Convert Full<Bytes> body to BoxBody<Bytes, hyper::Error>
                let (parts, full_body) = error_resp.into_parts();
                let boxed_body = full_body
                    .map_err(|never| match never {})
                    .boxed();

                Ok(Response::from_parts(parts, boxed_body))
            }
        }
    } else {
        // === No retry_policy: preserve existing direct-call behavior unchanged ===
        let llm_response = match reqwest::Client::new()
            .post(&full_qualified_llm_provider_url)
            .headers(request_headers)
            .body(client_request_bytes_for_upstream)
            .send()
            .await
        {
            Ok(res) => res,
            Err(err) => {
                return Ok(BrightStaffError::InternalServerError(format!(
                    "Failed to send request: {}",
                    err
                ))
                .into_response());
            }
        };

        // copy over the headers and status code from the original response
        let response_headers = llm_response.headers().clone();
        let upstream_status = llm_response.status();
        let mut response = Response::builder().status(upstream_status);
        let headers = response.headers_mut().unwrap();
        for (header_name, header_value) in response_headers.iter() {
            headers.insert(header_name, header_value.clone());
        }

        // Build LLM span with actual status code using constants
        let byte_stream = llm_response.bytes_stream();

        // Create base processor for metrics and tracing
        let base_processor = ObservableStreamProcessor::new(
            operation_component::LLM,
            span_name,
            request_start_time,
            messages_for_signals,
        );

        // === v1/responses state management: Wrap with ResponsesStateProcessor ===
        let streaming_response = if let (true, false, Some(state_store)) = (
            should_manage_state,
            original_input_items.is_empty(),
            state_storage,
        ) {
            let content_encoding = response_headers
                .get("content-encoding")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            let state_processor = ResponsesStateProcessor::new(
                base_processor,
                state_store,
                original_input_items,
                alias_resolved_model.clone(),
                resolved_model.clone(),
                is_streaming_request,
                false,
                content_encoding,
                request_id,
            );
            create_streaming_response(byte_stream, state_processor, 16)
        } else {
            create_streaming_response(byte_stream, base_processor, 16)
        };

        match response.body(streaming_response.body) {
            Ok(response) => Ok(response),
            Err(err) => Ok(BrightStaffError::InternalServerError(format!(
                "Failed to create response: {}",
                err
            ))
            .into_response()),
        }
    }
}

/// Resolves model aliases by looking up the requested model in the model_aliases map.
/// Returns the target model if an alias is found, otherwise returns the original model.
fn resolve_model_alias(
    model_from_request: &str,
    model_aliases: &Arc<Option<HashMap<String, ModelAlias>>>,
) -> String {
    if let Some(aliases) = model_aliases.as_ref() {
        if let Some(model_alias) = aliases.get(model_from_request) {
            debug!(
                "Model Alias: 'From {}' -> 'To {}'",
                model_from_request, model_alias.target
            );
            return model_alias.target.clone();
        }
    }
    model_from_request.to_string()
}

/// Calculates the upstream path for the provider based on the model name.
/// Looks up provider configuration, gets the ProviderId and base_url_path_prefix,
/// then uses target_endpoint_for_provider to calculate the correct upstream path.
async fn get_upstream_path(
    llm_providers: &Arc<RwLock<LlmProviders>>,
    model_name: &str,
    request_path: &str,
    resolved_model: &str,
    is_streaming: bool,
) -> String {
    let (provider_id, base_url_path_prefix) = get_provider_info(llm_providers, model_name).await;

    // Calculate the upstream path using the proper API
    let client_api = SupportedAPIsFromClient::from_endpoint(request_path)
        .expect("Should have valid API endpoint");

    client_api.target_endpoint_for_provider(
        &provider_id,
        request_path,
        resolved_model,
        is_streaming,
        base_url_path_prefix.as_deref(),
    )
}

/// Helper function to get provider info (ProviderId and base_url_path_prefix)
async fn get_provider_info(
    llm_providers: &Arc<RwLock<LlmProviders>>,
    model_name: &str,
) -> (hermesllm::ProviderId, Option<String>) {
    let providers_lock = llm_providers.read().await;

    // Try to find by model name or provider name using LlmProviders::get
    // This handles both "gpt-4" and "openai/gpt-4" formats
    if let Some(provider) = providers_lock.get(model_name) {
        let provider_id = provider.provider_interface.to_provider_id();
        let prefix = provider.base_url_path_prefix.clone();
        return (provider_id, prefix);
    }

    // Fall back to default provider
    if let Some(provider) = providers_lock.default() {
        let provider_id = provider.provider_interface.to_provider_id();
        let prefix = provider.base_url_path_prefix.clone();
        (provider_id, prefix)
    } else {
        // Last resort: use OpenAI as hardcoded fallback
        warn!("No default provider found, falling back to OpenAI");
        (hermesllm::ProviderId::OpenAI, None)
    }
}
