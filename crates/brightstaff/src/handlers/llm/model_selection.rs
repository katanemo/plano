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
                // Skills-only decision (no route, no models) -> fall through
                // to the "none" sentinel so the original / aliased model is
                // used, but propagate activated_skills so they still get
                // injected. Documented at docs/source/resources/skills.rst.
                if decision.route_name.is_none() && decision.models.is_empty() {
                    current_span.record("route.selected_model", "none");
                    info!(
                        skills = ?decision
                            .activated_skills
                            .iter()
                            .map(|s| s.name.as_str())
                            .collect::<Vec<_>>(),
                        "no route determined; activating skills against default model"
                    );
                    bs_metrics::record_router_decision(
                        route_label,
                        "none",
                        true,
                        determination_elapsed,
                    );
                    return Ok(RoutingResult {
                        model_name: "none".to_string(),
                        models: vec!["none".to_string()],
                        route_name: None,
                        activated_skills: decision.activated_skills,
                    });
                }

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
                    route_name: decision.route_name,
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
/// shape (`get_messages` / `set_messages`).
///
/// # Behavior contract
///
/// * **No-op** when `activated_skills` is empty.
/// * **Augments the first system message in place** when one is present at
///   any position in `messages` (typically index 0, but we look for the
///   first `Role::System` rather than assuming). Subsequent system messages
///   (rare but legal for some providers) are left untouched. We pick "first"
///   so the skill content appears as early in the prompt as possible —
///   models weight earlier system content more heavily and an Anthropic
///   tools+system combo is conventionally a single leading block.
/// * **Inserts a new leading system message** at index 0 when no system
///   message exists in the request.
/// * **Flattens `MessageContent::Parts` system content to a single
///   `MessageContent::Text`** when extracting the base prompt. This is
///   intentional: every supported upstream API accepts text in system
///   messages, and the alternative — preserving each `ContentPart` and
///   appending a new text part — fails on providers that disallow
///   multi-part system content. The trade-off is that non-text system parts
///   (e.g. images attached to a system message, which no production
///   provider supports anyway) are dropped on the floor. Verified by
///   `flattens_parts_system_content` below.
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

#[cfg(test)]
mod tests {
    use super::*;
    use hermesllm::apis::openai::{ChatCompletionsRequest, ContentPart};

    fn req_with_messages(msgs: Vec<Message>) -> ProviderRequestType {
        ProviderRequestType::ChatCompletionsRequest(ChatCompletionsRequest {
            model: "test".to_string(),
            messages: msgs,
            ..Default::default()
        })
    }

    fn user_msg(text: &str) -> Message {
        Message {
            role: Role::User,
            content: Some(MessageContent::Text(text.to_string())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn system_msg(text: &str) -> Message {
        Message {
            role: Role::System,
            content: Some(MessageContent::Text(text.to_string())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn skill(name: &str, body: &str) -> SkillRef {
        SkillRef {
            name: name.to_string(),
            description: format!("desc for {name}"),
            path: None,
            base_dir: None,
            body: Some(body.to_string()),
            scope: Some("project".to_string()),
            compatibility: None,
            license: None,
            metadata: None,
            allowed_tools: None,
        }
    }

    fn first_system_text(req: &ProviderRequestType) -> String {
        req.get_messages()
            .iter()
            .find(|m| m.role == Role::System)
            .and_then(|m| m.content.as_ref())
            .map(|c| c.extract_text())
            .unwrap_or_default()
    }

    #[test]
    fn injects_new_system_message_when_none_present() {
        let mut req = req_with_messages(vec![user_msg("hi")]);
        inject_activated_skills_into_request(&mut req, &[skill("pdf", "process pdfs")]);
        let messages = req.get_messages();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::System);
        let txt = first_system_text(&req);
        assert!(txt.contains("<skill_content name=\"pdf\""));
        assert!(txt.contains("process pdfs"));
    }

    #[test]
    fn augments_existing_system_message_in_place_preserving_user_message() {
        let mut req = req_with_messages(vec![system_msg("you are helpful"), user_msg("hi")]);
        inject_activated_skills_into_request(&mut req, &[skill("pdf", "process pdfs")]);
        let messages = req.get_messages();
        assert_eq!(messages.len(), 2);
        let txt = first_system_text(&req);
        assert!(txt.starts_with("you are helpful"));
        assert!(txt.contains("<skill_content name=\"pdf\""));
        assert_eq!(messages[1].role, Role::User);
    }

    #[test]
    fn noop_when_no_skills_activated() {
        let mut req = req_with_messages(vec![system_msg("base"), user_msg("hi")]);
        let before = req.get_messages();
        inject_activated_skills_into_request(&mut req, &[]);
        let after = req.get_messages();
        assert_eq!(after.len(), before.len());
        assert_eq!(first_system_text(&req), "base");
    }

    #[test]
    fn augments_only_the_first_system_message() {
        // Two system messages (rare in practice). Only the first one is
        // augmented; the trailing one is left untouched. Documented contract.
        let mut req = req_with_messages(vec![
            system_msg("primary"),
            system_msg("secondary"),
            user_msg("hi"),
        ]);
        inject_activated_skills_into_request(&mut req, &[skill("pdf", "process pdfs")]);
        let messages = req.get_messages();
        assert!(messages[0]
            .content
            .as_ref()
            .unwrap()
            .extract_text()
            .contains("primary"));
        assert!(messages[0]
            .content
            .as_ref()
            .unwrap()
            .extract_text()
            .contains("<skill_content"));
        assert_eq!(
            messages[1].content.as_ref().unwrap().extract_text(),
            "secondary"
        );
    }

    #[test]
    fn flattens_parts_system_content() {
        // Documented behavior: `MessageContent::Parts` system content is
        // extracted to plain text via ExtractText and re-emitted as
        // `MessageContent::Text`. Non-text parts (e.g. images) are dropped
        // — no production provider ships images in a system message.
        let parts_system = Message {
            role: Role::System,
            content: Some(MessageContent::Parts(vec![
                ContentPart::Text {
                    text: "be brief".to_string(),
                },
                ContentPart::Text {
                    text: " and polite".to_string(),
                },
            ])),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        };
        let mut req = req_with_messages(vec![parts_system, user_msg("hi")]);
        inject_activated_skills_into_request(&mut req, &[skill("pdf", "body")]);
        let messages = req.get_messages();
        let system = &messages[0];
        match system.content.as_ref().unwrap() {
            MessageContent::Text(t) => {
                assert!(t.contains("be brief"));
                assert!(t.contains(" and polite"));
                assert!(t.contains("<skill_content name=\"pdf\""));
            }
            MessageContent::Parts(_) => panic!("expected flattened text, got Parts"),
        }
    }

    #[test]
    fn injects_in_orchestrator_order_for_multiple_skills() {
        let mut req = req_with_messages(vec![user_msg("hi")]);
        inject_activated_skills_into_request(
            &mut req,
            &[skill("first", "alpha-body"), skill("second", "beta-body")],
        );
        let txt = first_system_text(&req);
        let first_pos = txt.find("alpha-body").expect("first skill body present");
        let second_pos = txt.find("beta-body").expect("second skill body present");
        assert!(
            first_pos < second_pos,
            "skills should appear in the order they were activated"
        );
    }
}
