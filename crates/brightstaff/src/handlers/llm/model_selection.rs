use common::configuration::{SkillRef, TopLevelRoutingPreference};
use common::skills_runtime::augment_system_prompt_with_skills;
use hermesllm::apis::openai::{Message, MessageContent, Role};
use hermesllm::clients::endpoints::SupportedUpstreamAPIs;
use hermesllm::providers::request::ProviderRequest;
use hermesllm::transforms::lib::ExtractText;
use hermesllm::ProviderRequestType;
use hyper::StatusCode;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::metrics as bs_metrics;
use crate::metrics::labels as metric_labels;
use crate::router::orchestrator::OrchestratorService;
use crate::streaming::truncate_message;
use crate::tracing::routing;

/// Classify a request path (already stripped of `/agents` or `/routing` by
/// the caller) into the fixed `route` label used on routing metrics.
fn route_label_for_path(request_path: &str) -> &'static str {
    if request_path.starts_with("/agents") {
        metric_labels::ROUTE_AGENT
    } else if request_path.starts_with("/routing") {
        metric_labels::ROUTE_ROUTING
    } else {
        metric_labels::ROUTE_LLM
    }
}

pub struct RoutingResult {
    /// Primary model to use (first in the ranked list).
    pub model_name: String,
    /// Full ranked list — use subsequent entries as fallbacks on 429/5xx.
    pub models: Vec<String>,
    pub route_name: Option<String>,
    /// Agent Skills activated by Plano-Orchestrator for this request.
    /// Their `body` field (the SKILL.md content) is prepended to the
    /// upstream system prompt by the caller in `send_upstream`.
    pub activated_skills: Vec<SkillRef>,
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
    orchestrator_service: Arc<OrchestratorService>,
    client_request: ProviderRequestType,
    request_path: &str,
    request_id: &str,
    inline_routing_preferences: Option<Vec<TopLevelRoutingPreference>>,
) -> Result<RoutingResult, RoutingError> {
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
            warn!("unexpected: got non-ChatCompletions request after converting to OpenAI format");
            return Err(RoutingError::internal_error(
                "Request conversion failed".to_string(),
            ));
        }
        Err(err) => {
            warn!(
                "failed to convert request to ChatCompletionsRequest: {}",
                err
            );
            return Err(RoutingError::internal_error(format!(
                "Failed to convert request: {}",
                err
            )));
        }
    };

    debug!(
        request = %serde_json::to_string(&chat_request).unwrap(),
        "router request"
    );

    // Prepare log message with latest message from chat request
    let latest_message_for_log = chat_request
        .messages
        .last()
        .map_or("None".to_string(), |msg| {
            msg.content
                .as_ref()
                .map_or("None".to_string(), |c| c.to_string().replace('\n', "\\n"))
        });

    let latest_message_for_log = truncate_message(&latest_message_for_log, 50);

    info!(
        path = %request_path,
        latest_message = %latest_message_for_log,
        "processing router request"
    );

    // Capture start time for routing span
    let routing_start_time = std::time::Instant::now();

    let routing_result = orchestrator_service
        .determine_route(
            &chat_request.messages,
            inline_routing_preferences,
            request_id,
        )
        .await;

    let determination_elapsed = routing_start_time.elapsed();
    let determination_ms = determination_elapsed.as_millis() as i64;
    let current_span = tracing::Span::current();
    current_span.record(routing::ROUTE_DETERMINATION_MS, determination_ms);
    let route_label = route_label_for_path(request_path);

    match routing_result {
        Ok(route) => match route {
            Some(decision) => {
                let model_name = decision.models.first().cloned().unwrap_or_default();
                current_span.record("route.selected_model", model_name.as_str());
                bs_metrics::record_router_decision(
                    route_label,
                    &model_name,
                    false,
                    determination_elapsed,
                );
                Ok(RoutingResult {
                    model_name,
                    models: decision.models,
                    route_name: Some(decision.route_name),
                    activated_skills: decision.activated_skills,
                })
            }
            None => {
                // No route determined, return sentinel value "none"
                // This signals to llm.rs to use the original validated request model
                current_span.record("route.selected_model", "none");
                info!("no route determined, using default model");
                bs_metrics::record_router_decision(
                    route_label,
                    "none",
                    true,
                    determination_elapsed,
                );

                Ok(RoutingResult {
                    model_name: "none".to_string(),
                    models: vec!["none".to_string()],
                    route_name: None,
                    activated_skills: Vec::new(),
                })
            }
        },
        Err(err) => {
            current_span.record("route.selected_model", "unknown");
            bs_metrics::record_router_decision(route_label, "unknown", true, determination_elapsed);
            Err(RoutingError::internal_error(format!(
                "Failed to determine route: {}",
                err
            )))
        }
    }
}

/// Prepend the bodies of `activated_skills` to the system prompt of the
/// upstream request so the chosen LLM has access to each skill's instructions.
/// Works across every provider variant by going through the OpenAI message
/// shape (`get_messages`/`set_messages`).
///
/// When there is already a leading system message we augment it in place;
/// otherwise a new system message is inserted at position 0. No-op when
/// `activated_skills` is empty.
pub fn inject_activated_skills_into_request(
    client_request: &mut ProviderRequestType,
    activated_skills: &[SkillRef],
) {
    if activated_skills.is_empty() {
        return;
    }

    let skill_refs: Vec<&SkillRef> = activated_skills.iter().collect();

    let mut messages = client_request.get_messages();

    let (system_idx, base_text) = match messages.iter().position(|m| m.role == Role::System) {
        Some(idx) => {
            let text = messages[idx]
                .content
                .as_ref()
                .map(|c| c.extract_text())
                .unwrap_or_default();
            (Some(idx), Some(text))
        }
        None => (None, None),
    };

    let augmented = augment_system_prompt_with_skills(base_text, &skill_refs);
    let Some(augmented_text) = augmented else {
        return;
    };

    match system_idx {
        Some(idx) => {
            messages[idx].content = Some(MessageContent::Text(augmented_text));
        }
        None => {
            messages.insert(
                0,
                Message {
                    role: Role::System,
                    content: Some(MessageContent::Text(augmented_text)),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            );
        }
    }

    client_request.set_messages(&messages);
}
