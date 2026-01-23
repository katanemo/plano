use common::configuration::ModelUsagePreference;
use common::traces::{parse_traceparent, SpanBuilder, SpanKind, TraceCollector};
use hermesllm::clients::endpoints::SupportedUpstreamAPIs;
use hermesllm::{ProviderRequest, ProviderRequestType};
use hyper::StatusCode;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::router::llm_router::RouterService;
use crate::tracing::{http, operation_component, routing, OperationNameBuilder};

pub struct RoutingResult {
    pub model_name: String,
}

pub struct RoutingError {
    pub message: String,
    pub status_code: StatusCode,
}

impl RoutingError {
    pub fn internal_error(message: String) -> Self {
        Self {
            message,
            status_code: StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Determines the routing decision if
///
/// # Returns
/// * `Ok(RoutingResult)` - Contains the selected model name and span ID
/// * `Err(RoutingError)` - Contains error details and optional span ID
pub async fn router_chat_get_upstream_model(
    router_service: Arc<RouterService>,
    client_request: ProviderRequestType,
    trace_collector: Arc<TraceCollector>,
    traceparent: &str,
    request_path: &str,
    request_id: &str,
) -> Result<RoutingResult, RoutingError> {
    // Clone metadata for routing before converting (which consumes client_request)
    let routing_metadata = client_request.metadata().clone();

    // Convert to ChatCompletionsRequest for routing (regardless of input type)
    let chat_request = match ProviderRequestType::try_from((
        client_request,
        &SupportedUpstreamAPIs::OpenAIChatCompletions(hermesllm::apis::OpenAIApi::ChatCompletions),
    )) {
        Ok(ProviderRequestType::ChatCompletionsRequest(req)) => req,
        Ok(
            ProviderRequestType::MessagesRequest(_)
            | ProviderRequestType::BedrockConverse(_)
            | ProviderRequestType::BedrockConverseStream(_)
            | ProviderRequestType::ResponsesAPIRequest(_),
        ) => {
            warn!("Unexpected: got non-ChatCompletions request after converting to OpenAI format");
            return Err(RoutingError::internal_error(
                "Request conversion failed".to_string(),
            ));
        }
        Err(err) => {
            warn!(
                "Failed to convert request to ChatCompletionsRequest: {}",
                err
            );
            return Err(RoutingError::internal_error(format!(
                "Failed to convert request: {}",
                err
            )));
        }
    };

    debug!(
        "[PLANO_REQ_ID: {:?}]: ROUTER_REQ: {}",
        request_id,
        &serde_json::to_string(&chat_request).unwrap()
    );

    // Extract usage preferences from metadata
    let usage_preferences_str: Option<String> = routing_metadata.as_ref().and_then(|metadata| {
        metadata
            .get("archgw_preference_config")
            .map(|value| value.to_string())
    });

    let usage_preferences: Option<Vec<ModelUsagePreference>> = usage_preferences_str
        .as_ref()
        .and_then(|s| serde_yaml::from_str(s).ok());

    // Prepare log message with latest message from chat request
    let latest_message_for_log = chat_request
        .messages
        .last()
        .map_or("None".to_string(), |msg| {
            msg.content
                .as_ref()
                .map_or("None".to_string(), |c| c.to_string().replace('\n', "\\n"))
        });

    const MAX_MESSAGE_LENGTH: usize = 50;
    let latest_message_for_log = if latest_message_for_log.chars().count() > MAX_MESSAGE_LENGTH {
        let truncated: String = latest_message_for_log
            .chars()
            .take(MAX_MESSAGE_LENGTH)
            .collect();
        format!("{}...", truncated)
    } else {
        latest_message_for_log
    };

    info!(
        "[PLANO_REQ_ID: {:?}] | ROUTER_REQ | Usage preferences from request: {}, request_path: {}, latest message: {}",
        request_id,
        usage_preferences.is_some(),
        request_path,
        latest_message_for_log
    );

    // Capture start time for routing span
    let routing_start_time = std::time::Instant::now();
    let routing_start_system_time = std::time::SystemTime::now();

    // Attempt to determine route using the router service
    let routing_result = router_service
        .determine_route(
            &chat_request.messages,
            traceparent,
            usage_preferences,
            request_id,
        )
        .await;

    match routing_result {
        Ok(route) => match route {
            Some((_, model_name)) => {
                // Record successful routing span
                let mut attrs: HashMap<String, String> = HashMap::new();
                attrs.insert("route.selected_model".to_string(), model_name.clone());
                record_routing_span(
                    trace_collector,
                    traceparent,
                    routing_start_time,
                    routing_start_system_time,
                    attrs,
                )
                .await;

                Ok(RoutingResult { model_name })
            }
            None => {
                // No route determined, return sentinel value "none"
                // This signals to llm.rs to use the original validated request model
                info!(
                    "[PLANO_REQ_ID: {}] | ROUTER_REQ | No route determined, returning sentinel 'none'",
                    request_id
                );

                let mut attrs = HashMap::new();
                attrs.insert("route.selected_model".to_string(), "none".to_string());
                record_routing_span(
                    trace_collector,
                    traceparent,
                    routing_start_time,
                    routing_start_system_time,
                    attrs,
                )
                .await;

                Ok(RoutingResult {
                    model_name: "none".to_string(),
                })
            }
        },
        Err(err) => {
            // Record failed routing span
            let mut attrs = HashMap::new();
            attrs.insert("route.selected_model".to_string(), "unknown".to_string());
            attrs.insert("error.message".to_string(), err.to_string());
            record_routing_span(
                trace_collector,
                traceparent,
                routing_start_time,
                routing_start_system_time,
                attrs,
            )
            .await;

            Err(RoutingError::internal_error(format!(
                "Failed to determine route: {}",
                err
            )))
        }
    }
}

/// Helper function to record a routing span with the given attributes.
/// Reduces code duplication across different routing outcomes.
async fn record_routing_span(
    trace_collector: Arc<TraceCollector>,
    traceparent: &str,
    start_time: std::time::Instant,
    start_system_time: std::time::SystemTime,
    attrs: HashMap<String, String>,
) {
    // The routing always uses OpenAI Chat Completions format internally,
    // so we log that as the actual API being used for routing
    let routing_api_path = "/v1/chat/completions";

    let routing_operation_name = OperationNameBuilder::new()
        .with_method("POST")
        .with_path(routing_api_path)
        .with_target("Arch-Router-1.5B")
        .build();

    let (trace_id, parent_span_id) = parse_traceparent(traceparent);

    // Build the routing span directly using constants
    let mut span_builder = SpanBuilder::new(&routing_operation_name)
        .with_trace_id(&trace_id)
        .with_kind(SpanKind::Client)
        .with_start_time(start_system_time)
        .with_end_time(std::time::SystemTime::now())
        .with_attribute(http::METHOD, "POST")
        .with_attribute(http::TARGET, routing_api_path.to_string())
        .with_attribute(
            routing::ROUTE_DETERMINATION_MS,
            start_time.elapsed().as_millis().to_string(),
        );

    // Only set parent span ID if it exists (not a root span)
    if let Some(parent) = parent_span_id {
        span_builder = span_builder.with_parent_span_id(&parent);
    }

    // Add all custom attributes
    for (key, value) in attrs {
        span_builder = span_builder.with_attribute(key, value);
    }

    let span = span_builder.build();

    // Record the span directly to the collector
    trace_collector.record_span(operation_component::ROUTING, span);
}
