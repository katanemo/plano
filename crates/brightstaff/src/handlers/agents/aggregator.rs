use hermesllm::apis::openai::{MessageContent, Role};
use hermesllm::apis::OpenAIMessage;
use hermesllm::transforms::ExtractText;
use hyper::header;
use tracing::{debug, warn};

use crate::router::http::{post_and_extract_content, HttpError};

/// Instruction for the model that merges agent replies. Kept narrow on purpose:
/// the aggregator composes, it does not reason about the task or add facts.
const AGGREGATOR_SYSTEM_PROMPT: &str = "You merge answers from specialist agents into a single reply for the user.

Rules:
- Cover every part of the user's request that an agent answered.
- Use only facts present in the agent answers. Never add, guess, or correct them.
- If agents disagree, state both claims and say which agent gave each.
- A response marked status=\"unavailable\" means that part could not be answered. Say so briefly instead of omitting it or inventing an answer.
- Return one coherent reply. Do not mention agents, routing, or that answers were merged.";

/// One agent's contribution to the final answer. A branch that produced nothing
/// still appears, so the merge can tell the user what is missing rather than
/// returning a silently incomplete answer.
#[derive(Debug, Clone)]
pub struct AgentReply {
    pub agent: String,
    /// `Ok` is the agent's answer, `Err` is why the branch produced none.
    pub result: Result<String, String>,
}

impl AgentReply {
    pub fn answered(agent: &str, text: String) -> Self {
        Self {
            agent: agent.to_string(),
            result: Ok(text),
        }
    }

    pub fn failed(agent: &str, reason: String) -> Self {
        Self {
            agent: agent.to_string(),
            result: Err(reason),
        }
    }

    pub fn text(&self) -> Option<&str> {
        self.result.as_deref().ok()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AggregationError {
    #[error("aggregator '{0}' is not a configured model provider or agent id")]
    Unresolved(String),
    #[error("aggregator model call failed: {0}")]
    Http(#[from] HttpError),
    #[error("aggregator returned no content")]
    EmptyResponse,
    #[error("failed to build aggregator request: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("invalid provider hint header: {0}")]
    InvalidHeader(String),
}

/// Text of the last user turn, which is what the agents were asked about.
pub fn last_user_text(messages: &[OpenAIMessage]) -> String {
    messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .and_then(|m| m.content.as_ref())
        .map(|c| c.extract_text())
        .unwrap_or_default()
}

/// Render replies as a labeled block so the model can attribute each answer.
/// Unavailable branches are listed too, so a partial answer is reported as
/// partial instead of looking complete.
pub fn render_replies(replies: &[AgentReply]) -> String {
    let mut out = String::from("<agent_responses>\n");
    for reply in replies {
        match &reply.result {
            Ok(text) => out.push_str(&format!(
                "<response agent=\"{}\">\n{}\n</response>\n",
                reply.agent,
                text.trim()
            )),
            Err(reason) => out.push_str(&format!(
                "<response agent=\"{}\" status=\"unavailable\" reason=\"{}\"/>\n",
                reply.agent,
                reason.trim()
            )),
        }
    }
    out.push_str("</agent_responses>");
    out
}

/// Deterministic join for complementary specialists. No model call — each
/// successful reply is labeled; unavailable branches stay visible.
pub fn concat_replies(replies: &[AgentReply]) -> String {
    let mut parts = Vec::new();
    for reply in replies {
        match &reply.result {
            Ok(text) if !text.trim().is_empty() => {
                parts.push(format!("[{}]\n{}", reply.agent, text.trim()));
            }
            Ok(_) => {}
            Err(reason) => {
                parts.push(format!("[{}] unavailable: {}", reply.agent, reason.trim()));
            }
        }
    }
    parts.join("\n\n")
}

/// Build the aggregator conversation. Only the user's request and the agent
/// replies are passed; prior turns are deliberately left out to keep the
/// merge grounded in what the agents actually said.
pub fn build_aggregation_messages(
    user_request: &str,
    replies: &[AgentReply],
) -> Vec<OpenAIMessage> {
    let prompt = format!(
        "<user_request>\n{}\n</user_request>\n\n{}",
        user_request.trim(),
        render_replies(replies)
    );

    vec![
        message(Role::System, AGGREGATOR_SYSTEM_PROMPT),
        message(Role::User, &prompt),
    ]
}

fn message(role: Role, text: &str) -> OpenAIMessage {
    OpenAIMessage {
        role,
        content: Some(MessageContent::Text(text.to_string())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }
}

/// Call the aggregator model the same way agent selection calls its model:
/// a plain chat completion against the internal LLM endpoint with a provider hint.
pub async fn aggregate_with_model(
    client: &reqwest::Client,
    url: &str,
    provider: &str,
    model: &str,
    user_request: &str,
    replies: &[AgentReply],
) -> Result<String, AggregationError> {
    let body = serde_json::json!({
        "model": model,
        "stream": false,
        "messages": build_aggregation_messages(user_request, replies),
    });

    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    headers.insert(
        common::consts::ARCH_PROVIDER_HINT_HEADER,
        header::HeaderValue::from_str(provider)
            .map_err(|err| AggregationError::InvalidHeader(err.to_string()))?,
    );

    debug!(provider = %provider, model = %model, replies = replies.len(), "merging agent replies");

    let content = post_and_extract_content(client, url, headers, serde_json::to_string(&body)?)
        .await?
        .map(|(text, _)| text);

    match content {
        Some(text) if !text.trim().is_empty() => Ok(text),
        _ => {
            warn!(provider = %provider, "aggregator returned no content");
            Err(AggregationError::EmptyResponse)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    fn reply(agent: &str, text: &str) -> AgentReply {
        AgentReply::answered(agent, text.to_string())
    }

    fn user(text: &str) -> OpenAIMessage {
        message(Role::User, text)
    }

    #[test]
    fn concat_keeps_every_answer_and_marks_failures() {
        let text = concat_replies(&[
            reply("alpha", "TOKEN_A"),
            AgentReply::failed("beta", "timeout".into()),
            reply("gamma", "TOKEN_C"),
        ]);
        assert!(text.contains("[alpha]\nTOKEN_A"));
        assert!(text.contains("[beta] unavailable: timeout"));
        assert!(text.contains("[gamma]\nTOKEN_C"));
    }

    /// Every agent's answer has to survive into the prompt, labeled by agent.
    #[test]
    fn rendered_replies_label_each_agent() {
        let rendered = render_replies(&[reply("weather", "15C"), reply("currency", "1.08")]);
        assert!(rendered.contains("<response agent=\"weather\">\n15C"));
        assert!(rendered.contains("<response agent=\"currency\">\n1.08"));
        assert!(rendered.starts_with("<agent_responses>"));
        assert!(rendered.ends_with("</agent_responses>"));
    }

    /// An unavailable branch is reported, not silently dropped, so the user
    /// learns which part of their question went unanswered.
    #[test]
    fn unavailable_branches_are_rendered_with_a_reason() {
        let rendered = render_replies(&[
            reply("weather", "15C"),
            AgentReply::failed("currency", "timeout".to_string()),
        ]);
        assert!(rendered.contains("<response agent=\"weather\">\n15C"));
        assert!(rendered
            .contains("<response agent=\"currency\" status=\"unavailable\" reason=\"timeout\"/>"));
    }

    /// The merge prompt carries the user's request plus every reply.
    #[test]
    fn aggregation_messages_carry_request_and_replies() {
        let messages = build_aggregation_messages("weather and rate?", &[reply("weather", "15C")]);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::System);
        let prompt = messages[1].content.as_ref().unwrap().extract_text();
        assert!(prompt.contains("weather and rate?"));
        assert!(prompt.contains("15C"));
    }

    /// Prior turns stay out of the merge; only the latest request is used.
    #[test]
    fn last_user_text_picks_the_latest_user_turn() {
        let messages = vec![
            user("first question"),
            message(Role::Assistant, "an answer"),
            user("second question"),
        ];
        assert_eq!(last_user_text(&messages), "second question");
    }

    #[test]
    fn last_user_text_is_empty_without_user_turns() {
        assert_eq!(last_user_text(&[]), "");
    }

    #[tokio::test]
    async fn aggregate_returns_merged_content() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/chat/completions")
            .match_header("x-arch-llm-provider-hint", "openai")
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "id": "c1", "object": "chat.completion", "created": 0, "model": "gpt-4o",
                    "choices": [{"index": 0, "message": {"role": "assistant", "content": "merged"}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
                })
                .to_string(),
            )
            .create_async()
            .await;

        let merged = aggregate_with_model(
            &reqwest::Client::new(),
            &format!("{}/v1/chat/completions", server.url()),
            "openai",
            "gpt-4o",
            "question",
            &[reply("weather", "15C")],
        )
        .await
        .unwrap();

        assert_eq!(merged, "merged");
        mock.assert_async().await;
    }

    /// A blank merge is treated as failure so callers can fall back.
    #[tokio::test]
    async fn blank_aggregator_content_is_an_error() {
        let mut server = Server::new_async().await;
        server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "id": "c1", "object": "chat.completion", "created": 0, "model": "gpt-4o",
                    "choices": [{"index": 0, "message": {"role": "assistant", "content": "   "}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
                })
                .to_string(),
            )
            .create_async()
            .await;

        let err = aggregate_with_model(
            &reqwest::Client::new(),
            &format!("{}/v1/chat/completions", server.url()),
            "openai",
            "gpt-4o",
            "question",
            &[reply("weather", "15C")],
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AggregationError::EmptyResponse));
    }

    #[tokio::test]
    async fn upstream_failure_surfaces_as_http_error() {
        let mut server = Server::new_async().await;
        server
            .mock("POST", "/v1/chat/completions")
            .with_status(500)
            .with_body(r#"{"error":{"message":"boom"}}"#)
            .create_async()
            .await;

        let err = aggregate_with_model(
            &reqwest::Client::new(),
            &format!("{}/v1/chat/completions", server.url()),
            "openai",
            "gpt-4o",
            "question",
            &[reply("weather", "15C")],
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AggregationError::Http(_)));
    }
}
