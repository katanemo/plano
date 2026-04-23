use std::collections::HashMap;

use common::configuration::{AgentUsagePreference, OrchestrationPreference};
use hermesllm::apis::openai::{ChatCompletionsRequest, Message, MessageContent, Role};
use hermesllm::transforms::lib::ExtractText;
use serde::{ser::Serialize as SerializeTrait, Deserialize, Serialize};
use tracing::{debug, warn};

use super::orchestrator_model::{OrchestratorModel, OrchestratorModelError};

pub const MAX_TOKEN_LEN: usize = 8192; // Default max token length for the orchestration model

/// Hard cap on the number of recent messages considered when building the
/// routing prompt. Bounds prompt growth for long-running conversations and
/// acts as an outer guardrail before the token-budget loop runs. The most
/// recent `MAX_ROUTING_TURNS` filtered messages are kept; older turns are
/// dropped entirely.
pub const MAX_ROUTING_TURNS: usize = 16;

/// Unicode ellipsis used to mark where content was trimmed out of a long
/// message. Helps signal to the downstream router model that the message was
/// truncated.
const TRIM_MARKER: &str = "…";

/// Custom JSON formatter that produces spaced JSON (space after colons and commas), same as JSON in python
struct SpacedJsonFormatter;

impl serde_json::ser::Formatter for SpacedJsonFormatter {
    fn begin_array<W>(&mut self, writer: &mut W) -> std::io::Result<()>
    where
        W: ?Sized + std::io::Write,
    {
        writer.write_all(b"[")
    }

    fn end_array<W>(&mut self, writer: &mut W) -> std::io::Result<()>
    where
        W: ?Sized + std::io::Write,
    {
        writer.write_all(b"]")
    }

    fn begin_array_value<W>(&mut self, writer: &mut W, first: bool) -> std::io::Result<()>
    where
        W: ?Sized + std::io::Write,
    {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    fn end_array_value<W>(&mut self, _writer: &mut W) -> std::io::Result<()>
    where
        W: ?Sized + std::io::Write,
    {
        Ok(())
    }

    fn begin_object<W>(&mut self, writer: &mut W) -> std::io::Result<()>
    where
        W: ?Sized + std::io::Write,
    {
        writer.write_all(b"{")
    }

    fn end_object<W>(&mut self, writer: &mut W) -> std::io::Result<()>
    where
        W: ?Sized + std::io::Write,
    {
        writer.write_all(b"}")
    }

    fn begin_object_key<W>(&mut self, writer: &mut W, first: bool) -> std::io::Result<()>
    where
        W: ?Sized + std::io::Write,
    {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    fn end_object_key<W>(&mut self, _writer: &mut W) -> std::io::Result<()>
    where
        W: ?Sized + std::io::Write,
    {
        Ok(())
    }

    fn begin_object_value<W>(&mut self, writer: &mut W) -> std::io::Result<()>
    where
        W: ?Sized + std::io::Write,
    {
        writer.write_all(b": ")
    }

    fn end_object_value<W>(&mut self, _writer: &mut W) -> std::io::Result<()>
    where
        W: ?Sized + std::io::Write,
    {
        Ok(())
    }
}

/// Serialize a value to JSON with standard spacing (space after colons and commas)
/// e.g. {"name": "foo", "key": "value"} instead of {"name":"foo","key":"value"}
fn to_spaced_json<T: serde::Serialize>(value: &T) -> String {
    let mut buf = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut buf, SpacedJsonFormatter);
    value.serialize(&mut serializer).unwrap();
    String::from_utf8(buf).unwrap_or_default()
}

pub const ARCH_ORCHESTRATOR_V1_SYSTEM_PROMPT: &str = r#"
You are a helpful assistant that selects the most suitable routes based on user intent.
You are provided with a list of available routes enclosed within <routes></routes> XML tags:
<routes>
{routes}
</routes>

You are also given the conversation context enclosed within <conversation></conversation> XML tags:
<conversation>
{conversation}
</conversation>

## Instructions
1. Analyze the latest user intent from the conversation.
2. Compare it against the available routes to find which routes can help fulfill the request.
3. Respond only with the exact route names from <routes>.
4. If no routes can help or the intent is already fulfilled, return an empty list.

## Response Format
Return your answer strictly in JSON as follows:
{{"route": ["route_name_1", "route_name_2", "..."]}}
If no routes are needed, return an empty list for `route`.
"#;

pub type Result<T> = std::result::Result<T, OrchestratorModelError>;
pub struct OrchestratorModelV1 {
    agent_orchestration_json_str: String,
    agent_orchestration_to_model_map: HashMap<String, String>,
    orchestration_model: String,
    max_token_length: usize,
}

impl OrchestratorModelV1 {
    pub fn new(
        agent_orchestrations: HashMap<String, Vec<OrchestrationPreference>>,
        orchestration_model: String,
        max_token_length: usize,
    ) -> Self {
        let agent_orchestration_values: Vec<OrchestrationPreference> =
            agent_orchestrations.values().flatten().cloned().collect();
        // Format routes: each route as JSON on its own line with standard spacing
        let agent_orchestration_json_str = agent_orchestration_values
            .iter()
            .map(to_spaced_json)
            .collect::<Vec<String>>()
            .join("\n");
        let agent_orchestration_to_model_map: HashMap<String, String> = agent_orchestrations
            .iter()
            .flat_map(|(model, prefs)| prefs.iter().map(|pref| (pref.name.clone(), model.clone())))
            .collect();

        OrchestratorModelV1 {
            orchestration_model,
            max_token_length,
            agent_orchestration_json_str,
            agent_orchestration_to_model_map,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentOrchestratorResponse {
    /// The route field now expects an array of route names: ["route_name_1", "route_name_2", ...]
    pub route: Option<Vec<String>>,
}

const TOKEN_LENGTH_DIVISOR: usize = 4; // Approximate token length divisor for UTF-8 characters

impl OrchestratorModel for OrchestratorModelV1 {
    fn generate_request(
        &self,
        messages: &[Message],
        usage_preferences_from_request: &Option<Vec<AgentUsagePreference>>,
    ) -> ChatCompletionsRequest {
        // Remove system/developer/tool messages and messages without extractable
        // text (tool calls have no text content we can classify against).
        let filtered: Vec<&Message> = messages
            .iter()
            .filter(|m| {
                m.role != Role::System
                    && m.role != Role::Developer
                    && m.role != Role::Tool
                    && !m.content.extract_text().is_empty()
            })
            .collect();

        // Outer guardrail: only consider the last `MAX_ROUTING_TURNS` filtered
        // messages when building the routing prompt. Keeps prompt growth
        // predictable for long conversations regardless of per-message size.
        let start = filtered.len().saturating_sub(MAX_ROUTING_TURNS);
        let messages_vec: &[&Message] = &filtered[start..];

        // Ensure the conversation does not exceed the configured token budget.
        // We use `len() / TOKEN_LENGTH_DIVISOR` as a cheap token estimate to
        // avoid running a real tokenizer on the hot path.
        let mut token_count = ARCH_ORCHESTRATOR_V1_SYSTEM_PROMPT.len() / TOKEN_LENGTH_DIVISOR;
        let mut selected_messages_list_reversed: Vec<Message> = vec![];
        for (selected_messsage_count, message) in messages_vec.iter().rev().enumerate() {
            let message_text = message.content.extract_text();
            let message_token_count = message_text.len() / TOKEN_LENGTH_DIVISOR;
            if token_count + message_token_count > self.max_token_length {
                let remaining_tokens = self.max_token_length.saturating_sub(token_count);
                debug!(
                    attempted_total_tokens = token_count + message_token_count,
                    max_tokens = self.max_token_length,
                    remaining_tokens,
                    selected = selected_messsage_count,
                    total = messages_vec.len(),
                    "token count exceeds max, truncating conversation"
                );
                // If the overflow message is from the user we need to keep
                // some of it so the orchestrator still sees the latest user
                // intent. Use a middle-trim (head + ellipsis + tail): users
                // often frame the task at the start AND put the actual ask
                // at the end of a long pasted block, so preserving both is
                // better than a head-only cut. The ellipsis also signals to
                // the router model that content was dropped.
                if message.role == Role::User && remaining_tokens > 0 {
                    let max_bytes = remaining_tokens.saturating_mul(TOKEN_LENGTH_DIVISOR);
                    let truncated = trim_middle_utf8(&message_text, max_bytes);
                    selected_messages_list_reversed.push(Message {
                        role: Role::User,
                        content: Some(MessageContent::Text(truncated)),
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
                break;
            }
            token_count += message_token_count;
            selected_messages_list_reversed.push(Message {
                role: message.role.clone(),
                content: Some(MessageContent::Text(message_text)),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
        }

        if selected_messages_list_reversed.is_empty() {
            debug!("no messages selected, using last message");
            if let Some(last_message) = messages_vec.last() {
                selected_messages_list_reversed.push(Message {
                    role: last_message.role.clone(),
                    content: Some(MessageContent::Text(last_message.content.extract_text())),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
        }

        // ensure that first and last selected message is from user
        // Note: selected_messages_list_reversed is in reverse order, so:
        // - first() is the last message in the original conversation
        // - last() is the first message in the original conversation
        if let Some(first_message) = selected_messages_list_reversed.first() {
            if first_message.role != Role::User {
                warn!("last message is not from user, may lead to incorrect orchestration");
            }
        }
        if let Some(last_message) = selected_messages_list_reversed.last() {
            if last_message.role != Role::User {
                warn!("first message is not from user, may lead to incorrect orchestration");
            }
        }

        // Reverse the selected messages to maintain the conversation order
        let selected_conversation_list: Vec<Message> =
            selected_messages_list_reversed.into_iter().rev().collect();

        // Generate the orchestrator request message based on the usage preferences.
        // If preferences are passed in request then we use them;
        // Otherwise, we use the default orchestration modelpreferences.
        let orchestrator_message =
            match convert_to_orchestrator_preferences(usage_preferences_from_request) {
                Some(prefs) => generate_orchestrator_message(&prefs, &selected_conversation_list),
                None => generate_orchestrator_message(
                    &self.agent_orchestration_json_str,
                    &selected_conversation_list,
                ),
            };

        ChatCompletionsRequest {
            model: self.orchestration_model.clone(),
            messages: vec![Message {
                content: Some(MessageContent::Text(orchestrator_message)),
                role: Role::User,
                name: None,
                tool_calls: None,
                tool_call_id: None,
            }],
            temperature: Some(0.01),
            ..Default::default()
        }
    }

    fn parse_response(
        &self,
        content: &str,
        usage_preferences: &Option<Vec<AgentUsagePreference>>,
    ) -> Result<Option<Vec<(String, String)>>> {
        if content.is_empty() {
            return Ok(None);
        }
        let orchestrator_resp_fixed = fix_json_response(content);
        let orchestrator_response: AgentOrchestratorResponse =
            serde_json::from_str(orchestrator_resp_fixed.as_str())?;

        let selected_routes = orchestrator_response.route.unwrap_or_default();

        // Filter out empty routes
        let valid_routes: Vec<String> = selected_routes
            .into_iter()
            .filter(|route| !route.is_empty())
            .collect();

        if valid_routes.is_empty() {
            return Ok(None);
        }

        let mut result: Vec<(String, String)> = Vec::new();

        if let Some(usage_preferences) = usage_preferences {
            // If usage preferences are defined, we need to find the model that matches each selected route
            for selected_route in valid_routes {
                let model_name: Option<String> = usage_preferences
                    .iter()
                    .find(|pref| {
                        pref.orchestration_preferences
                            .iter()
                            .any(|orchestration_pref| orchestration_pref.name == selected_route)
                    })
                    .map(|pref| pref.model.clone());

                if let Some(model_name) = model_name {
                    result.push((selected_route, model_name));
                } else {
                    warn!(
                        route = %selected_route,
                        preferences = ?usage_preferences,
                        "no matching model found for route"
                    );
                }
            }
        } else {
            // If no usage preferences are passed in request then use the default orchestration model preferences
            for selected_route in valid_routes {
                if let Some(model) = self
                    .agent_orchestration_to_model_map
                    .get(&selected_route)
                    .cloned()
                {
                    result.push((selected_route, model));
                } else {
                    warn!(
                        route = %selected_route,
                        preferences = ?self.agent_orchestration_to_model_map,
                        "no model found for route"
                    );
                }
            }
        }

        if result.is_empty() {
            return Ok(None);
        }

        Ok(Some(result))
    }

    fn get_model_name(&self) -> String {
        self.orchestration_model.clone()
    }
}

fn generate_orchestrator_message(prefs: &str, selected_conversation_list: &Vec<Message>) -> String {
    // Format conversation with 4-space indentation (equivalent to Python's json.dumps(obj, indent=4))
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut conversation_buf = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut conversation_buf, formatter);
    SerializeTrait::serialize(&selected_conversation_list, &mut serializer).unwrap();
    let conversation_json = String::from_utf8(conversation_buf).unwrap_or_default();

    ARCH_ORCHESTRATOR_V1_SYSTEM_PROMPT
        .replace("{routes}", prefs)
        .replace("{conversation}", &conversation_json)
}

fn convert_to_orchestrator_preferences(
    prefs_from_request: &Option<Vec<AgentUsagePreference>>,
) -> Option<String> {
    if let Some(usage_preferences) = prefs_from_request {
        let orchestration_preferences: Vec<OrchestrationPreference> = usage_preferences
            .iter()
            .flat_map(|pref| {
                pref.orchestration_preferences
                    .iter()
                    .map(|orchestration_pref| OrchestrationPreference {
                        name: orchestration_pref.name.clone(),
                        description: orchestration_pref.description.clone(),
                    })
            })
            .collect();

        // Format routes: each route as JSON on its own line with standard spacing
        let routes_str = orchestration_preferences
            .iter()
            .map(to_spaced_json)
            .collect::<Vec<String>>()
            .join("\n");

        return Some(routes_str);
    }

    None
}

fn fix_json_response(body: &str) -> String {
    body.replace("'", "\"").replace("\\n", "")
}

/// Truncate `s` so the result is at most `max_bytes` bytes long, keeping
/// roughly 60% from the start and 40% from the end, with a Unicode ellipsis
/// separating the two. All splits respect UTF-8 character boundaries. When
/// `max_bytes` is too small to fit the marker at all, falls back to a
/// head-only truncation.
fn trim_middle_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    if max_bytes <= TRIM_MARKER.len() {
        // Not enough room even for the marker — just keep the start.
        let mut end = max_bytes;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        return s[..end].to_string();
    }

    let available = max_bytes - TRIM_MARKER.len();
    // Bias toward the start (60%) where task framing typically lives, while
    // still preserving ~40% of the tail where the user's actual ask often
    // appears after a long paste.
    let mut start_len = available * 3 / 5;
    while start_len > 0 && !s.is_char_boundary(start_len) {
        start_len -= 1;
    }
    let end_len = available - start_len;
    let mut end_start = s.len().saturating_sub(end_len);
    while end_start < s.len() && !s.is_char_boundary(end_start) {
        end_start += 1;
    }

    let mut out = String::with_capacity(start_len + TRIM_MARKER.len() + (s.len() - end_start));
    out.push_str(&s[..start_len]);
    out.push_str(TRIM_MARKER);
    out.push_str(&s[end_start..]);
    out
}

impl std::fmt::Debug for dyn OrchestratorModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OrchestratorModel")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_spaced_json_formatter() {
        // Test basic object
        let obj = serde_json::json!({"name": "foo", "value": 123});
        let result = to_spaced_json(&obj);
        assert_eq!(result, r#"{"name": "foo", "value": 123}"#);

        // Test nested object
        let nested = serde_json::json!({"outer": {"inner": "value"}});
        let result = to_spaced_json(&nested);
        assert_eq!(result, r#"{"outer": {"inner": "value"}}"#);

        // Test array
        let arr = serde_json::json!(["a", "b", "c"]);
        let result = to_spaced_json(&arr);
        assert_eq!(result, r#"["a", "b", "c"]"#);

        // Test object with array
        let obj_arr = serde_json::json!({"items": [1, 2, 3]});
        let result = to_spaced_json(&obj_arr);
        assert_eq!(result, r#"{"items": [1, 2, 3]}"#);

        // CRITICAL: Test that colons inside string values are NOT modified
        let with_colon = serde_json::json!({"name": "foo:bar", "url": "http://example.com"});
        let result = to_spaced_json(&with_colon);
        assert_eq!(
            result,
            r#"{"name": "foo:bar", "url": "http://example.com"}"#
        );

        // Test empty object and array
        let empty_obj = serde_json::json!({});
        let result = to_spaced_json(&empty_obj);
        assert_eq!(result, r#"{}"#);

        let empty_arr = serde_json::json!([]);
        let result = to_spaced_json(&empty_arr);
        assert_eq!(result, r#"[]"#);

        // Test complex nested structure with special characters in values
        // Note: serde_json doesn't guarantee field order, so we verify the formatting is correct
        // by checking key properties of the output
        let complex = serde_json::json!({
            "type": "object",
            "properties": {},
            "urls": ["https://api.example.com:8080/path", "file:///local/path"]
        });
        let result = to_spaced_json(&complex);
        // Verify URLs with colons are preserved correctly
        assert!(result
            .contains(r#""urls": ["https://api.example.com:8080/path", "file:///local/path"]"#));
        // Verify spacing format
        assert!(result.contains(r#""type": "object""#));
        assert!(result.contains(r#""properties": {}"#));
    }

    #[test]
    fn test_system_prompt_format() {
        let expected_prompt = r#"
You are a helpful assistant that selects the most suitable routes based on user intent.
You are provided with a list of available routes enclosed within <routes></routes> XML tags:
<routes>
{"name": "Image generation", "description": "generating image", "parameters": {"type": "object", "properties": {}, "required": []}}
</routes>

You are also given the conversation context enclosed within <conversation></conversation> XML tags:
<conversation>
[
    {
        "role": "user",
        "content": "hi"
    },
    {
        "role": "assistant",
        "content": "Hello! How can I assist you today?"
    },
    {
        "role": "user",
        "content": "given the image In style of Andy Warhol, portrait of Bart and Lisa Simpson"
    }
]
</conversation>

## Instructions
1. Analyze the latest user intent from the conversation.
2. Compare it against the available routes to find which routes can help fulfill the request.
3. Respond only with the exact route names from <routes>.
4. If no routes can help or the intent is already fulfilled, return an empty list.

## Response Format
Return your answer strictly in JSON as follows:
{{"route": ["route_name_1", "route_name_2", "..."]}}
If no routes are needed, return an empty list for `route`.
"#;
        let orchestrations_str = r#"
          {
            "gpt-4o": [
              {"name": "Image generation", "description": "generating image"}
            ]
        }
        "#;
        let agent_orchestrations = serde_json::from_str::<
            HashMap<String, Vec<OrchestrationPreference>>,
        >(orchestrations_str)
        .unwrap();
        let orchestration_model = "test-model".to_string();
        let orchestrator = OrchestratorModelV1::new(
            agent_orchestrations,
            orchestration_model.clone(),
            usize::MAX,
        );

        let conversation_str = r#"
                    [
                        {
                            "role": "user",
                            "content": "hi"
                        },
                        {
                            "role": "assistant",
                            "content": "Hello! How can I assist you today?"
                        },
                        {
                            "role": "user",
                            "content": "given the image In style of Andy Warhol, portrait of Bart and Lisa Simpson"
                        }
                    ]
        "#;
        let conversation: Vec<Message> = serde_json::from_str(conversation_str).unwrap();

        let req = orchestrator.generate_request(&conversation, &None);

        let prompt = req.messages[0].content.extract_text();

        assert_eq!(expected_prompt, prompt);
    }

    #[test]
    fn test_system_prompt_format_usage_preferences() {
        let expected_prompt = r#"
You are a helpful assistant that selects the most suitable routes based on user intent.
You are provided with a list of available routes enclosed within <routes></routes> XML tags:
<routes>
{"name": "code-generation", "description": "generating new code snippets, functions, or boilerplate based on user prompts or requirements", "parameters": {"type": "object", "properties": {}, "required": []}}
</routes>

You are also given the conversation context enclosed within <conversation></conversation> XML tags:
<conversation>
[
    {
        "role": "user",
        "content": "hi"
    },
    {
        "role": "assistant",
        "content": "Hello! How can I assist you today?"
    },
    {
        "role": "user",
        "content": "given the image In style of Andy Warhol, portrait of Bart and Lisa Simpson"
    }
]
</conversation>

## Instructions
1. Analyze the latest user intent from the conversation.
2. Compare it against the available routes to find which routes can help fulfill the request.
3. Respond only with the exact route names from <routes>.
4. If no routes can help or the intent is already fulfilled, return an empty list.

## Response Format
Return your answer strictly in JSON as follows:
{{"route": ["route_name_1", "route_name_2", "..."]}}
If no routes are needed, return an empty list for `route`.
"#;
        // Empty orchestrations map - not used when usage_preferences are provided
        let agent_orchestrations: HashMap<String, Vec<OrchestrationPreference>> = HashMap::new();
        let orchestration_model = "test-model".to_string();
        let orchestrator = OrchestratorModelV1::new(
            agent_orchestrations,
            orchestration_model.clone(),
            usize::MAX,
        );

        let conversation_str = r#"
                    [
                        {
                            "role": "user",
                            "content": "hi"
                        },
                        {
                            "role": "assistant",
                            "content": "Hello! How can I assist you today?"
                        },
                        {
                            "role": "user",
                            "content": "given the image In style of Andy Warhol, portrait of Bart and Lisa Simpson"
                        }
                    ]
        "#;
        let conversation: Vec<Message> = serde_json::from_str(conversation_str).unwrap();

        let usage_preferences = Some(vec![AgentUsagePreference {
            model: "claude/claude-3-7-sonnet".to_string(),
            orchestration_preferences: vec![OrchestrationPreference {
                name: "code-generation".to_string(),
                description: "generating new code snippets, functions, or boilerplate based on user prompts or requirements".to_string(),
            }],
        }]);
        let req = orchestrator.generate_request(&conversation, &usage_preferences);

        let prompt = req.messages[0].content.extract_text();

        assert_eq!(expected_prompt, prompt);
    }

    #[test]
    fn test_conversation_exceed_token_count() {
        let expected_prompt = r#"
You are a helpful assistant that selects the most suitable routes based on user intent.
You are provided with a list of available routes enclosed within <routes></routes> XML tags:
<routes>
{"name": "Image generation", "description": "generating image", "parameters": {"type": "object", "properties": {}, "required": []}}
</routes>

You are also given the conversation context enclosed within <conversation></conversation> XML tags:
<conversation>
[
    {
        "role": "user",
        "content": "given the image In style of Andy Warhol, portrait of Bart and Lisa Simpson"
    }
]
</conversation>

## Instructions
1. Analyze the latest user intent from the conversation.
2. Compare it against the available routes to find which routes can help fulfill the request.
3. Respond only with the exact route names from <routes>.
4. If no routes can help or the intent is already fulfilled, return an empty list.

## Response Format
Return your answer strictly in JSON as follows:
{{"route": ["route_name_1", "route_name_2", "..."]}}
If no routes are needed, return an empty list for `route`.
"#;

        let orchestrations_str = r#"
          {
            "gpt-4o": [
              {"name": "Image generation", "description": "generating image"}
            ]
        }
        "#;
        let agent_orchestrations = serde_json::from_str::<
            HashMap<String, Vec<OrchestrationPreference>>,
        >(orchestrations_str)
        .unwrap();
        let orchestration_model = "test-model".to_string();
        let orchestrator = OrchestratorModelV1::new(agent_orchestrations, orchestration_model, 235);

        let conversation_str = r#"
                    [
                        {
                            "role": "user",
                            "content": "hi"
                        },
                        {
                            "role": "assistant",
                            "content": "Hello! How can I assist you today?"
                        },
                        {
                            "role": "user",
                            "content": "given the image In style of Andy Warhol, portrait of Bart and Lisa Simpson"
                        }
                    ]
        "#;

        let conversation: Vec<Message> = serde_json::from_str(conversation_str).unwrap();

        let req = orchestrator.generate_request(&conversation, &None);

        let prompt = req.messages[0].content.extract_text();

        assert_eq!(expected_prompt, prompt);
    }

    #[test]
    fn test_conversation_exceed_token_count_large_single_message() {
        let expected_prompt = r#"
You are a helpful assistant that selects the most suitable routes based on user intent.
You are provided with a list of available routes enclosed within <routes></routes> XML tags:
<routes>
{"name": "Image generation", "description": "generating image", "parameters": {"type": "object", "properties": {}, "required": []}}
</routes>

You are also given the conversation context enclosed within <conversation></conversation> XML tags:
<conversation>
[
    {
        "role": "user",
        "content": "given the image In style of Andy Warhol, portrait of Bart and Lisa Simpson and this is a very long message that exceeds the max token length of the routing model, so it should be truncated and only the last user message should be included in the conversation for routing."
    }
]
</conversation>

## Instructions
1. Analyze the latest user intent from the conversation.
2. Compare it against the available routes to find which routes can help fulfill the request.
3. Respond only with the exact route names from <routes>.
4. If no routes can help or the intent is already fulfilled, return an empty list.

## Response Format
Return your answer strictly in JSON as follows:
{{"route": ["route_name_1", "route_name_2", "..."]}}
If no routes are needed, return an empty list for `route`.
"#;

        let orchestrations_str = r#"
          {
            "gpt-4o": [
              {"name": "Image generation", "description": "generating image"}
            ]
        }
        "#;
        let agent_orchestrations = serde_json::from_str::<
            HashMap<String, Vec<OrchestrationPreference>>,
        >(orchestrations_str)
        .unwrap();

        let orchestration_model = "test-model".to_string();
        let orchestrator = OrchestratorModelV1::new(agent_orchestrations, orchestration_model, 200);

        let conversation_str = r#"
                    [
                        {
                            "role": "user",
                            "content": "hi"
                        },
                        {
                            "role": "assistant",
                            "content": "Hello! How can I assist you today?"
                        },
                        {
                            "role": "user",
                            "content": "given the image In style of Andy Warhol, portrait of Bart and Lisa Simpson and this is a very long message that exceeds the max token length of the routing model, so it should be truncated and only the last user message should be included in the conversation for routing."
                        }
                    ]
        "#;

        let conversation: Vec<Message> = serde_json::from_str(conversation_str).unwrap();

        let req = orchestrator.generate_request(&conversation, &None);

        let prompt = req.messages[0].content.extract_text();

        assert_eq!(expected_prompt, prompt);
    }

    #[test]
    fn test_conversation_trim_upto_user_message() {
        // With max_token_length=230, the older user message "given the image
        // In style of Andy Warhol" overflows the remaining budget and gets
        // middle-trimmed (head + ellipsis + tail) until it fits. Newer turns
        // are kept in full.
        let expected_prompt = r#"
You are a helpful assistant that selects the most suitable routes based on user intent.
You are provided with a list of available routes enclosed within <routes></routes> XML tags:
<routes>
{"name": "Image generation", "description": "generating image", "parameters": {"type": "object", "properties": {}, "required": []}}
</routes>

You are also given the conversation context enclosed within <conversation></conversation> XML tags:
<conversation>
[
    {
        "role": "user",
        "content": "given…rhol"
    },
    {
        "role": "assistant",
        "content": "ok here is the image"
    },
    {
        "role": "user",
        "content": "pls give me another image about Bart and Lisa"
    }
]
</conversation>

## Instructions
1. Analyze the latest user intent from the conversation.
2. Compare it against the available routes to find which routes can help fulfill the request.
3. Respond only with the exact route names from <routes>.
4. If no routes can help or the intent is already fulfilled, return an empty list.

## Response Format
Return your answer strictly in JSON as follows:
{{"route": ["route_name_1", "route_name_2", "..."]}}
If no routes are needed, return an empty list for `route`.
"#;

        let orchestrations_str = r#"
          {
            "gpt-4o": [
              {"name": "Image generation", "description": "generating image"}
            ]
        }
        "#;
        let agent_orchestrations = serde_json::from_str::<
            HashMap<String, Vec<OrchestrationPreference>>,
        >(orchestrations_str)
        .unwrap();
        let orchestration_model = "test-model".to_string();
        let orchestrator = OrchestratorModelV1::new(agent_orchestrations, orchestration_model, 230);

        let conversation_str = r#"
                    [
                        {
                            "role": "user",
                            "content": "hi"
                        },
                        {
                            "role": "assistant",
                            "content": "Hello! How can I assist you today?"
                        },
                        {
                            "role": "user",
                            "content": "given the image In style of Andy Warhol"
                        },
                        {
                            "role": "assistant",
                            "content": "ok here is the image"
                        },
                        {
                            "role": "user",
                            "content": "pls give me another image about Bart and Lisa"
                        }
                    ]
        "#;

        let conversation: Vec<Message> = serde_json::from_str(conversation_str).unwrap();

        let req = orchestrator.generate_request(&conversation, &None);

        let prompt = req.messages[0].content.extract_text();

        assert_eq!(expected_prompt, prompt);
    }

    #[test]
    fn test_huge_single_user_message_is_middle_trimmed() {
        // Regression test for the case where a single, extremely large user
        // message was being passed to the orchestrator verbatim and blowing
        // past the upstream model's context window. The trimmer must now
        // middle-trim (head + ellipsis + tail) the oversized message so the
        // resulting request stays within the configured budget, and the
        // trim marker must be present so the router model knows content
        // was dropped.
        let orchestrations_str = r#"
          {
            "gpt-4o": [
              {"name": "Image generation", "description": "generating image"}
            ]
        }
        "#;
        let agent_orchestrations = serde_json::from_str::<
            HashMap<String, Vec<OrchestrationPreference>>,
        >(orchestrations_str)
        .unwrap();

        let max_token_length = 2048;
        let orchestrator = OrchestratorModelV1::new(
            agent_orchestrations,
            "test-model".to_string(),
            max_token_length,
        );

        // ~500KB of content — same scale as the real payload that triggered
        // the production upstream 400.
        let head = "HEAD_MARKER_START ";
        let tail = " TAIL_MARKER_END";
        let filler = "A".repeat(500_000);
        let huge_user_content = format!("{head}{filler}{tail}");

        let conversation = vec![Message {
            role: Role::User,
            content: Some(MessageContent::Text(huge_user_content.clone())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }];

        let req = orchestrator.generate_request(&conversation, &None);
        let prompt = req.messages[0].content.extract_text();

        // Prompt must stay bounded. Generous ceiling = budget-in-bytes +
        // scaffolding + slack. Real result should be well under this.
        let byte_ceiling = max_token_length * TOKEN_LENGTH_DIVISOR
            + ARCH_ORCHESTRATOR_V1_SYSTEM_PROMPT.len()
            + 1024;
        assert!(
            prompt.len() < byte_ceiling,
            "prompt length {} exceeded ceiling {} — truncation did not apply",
            prompt.len(),
            byte_ceiling,
        );

        // Not all 500k filler chars survive.
        let a_count = prompt.chars().filter(|c| *c == 'A').count();
        assert!(
            a_count < filler.len(),
            "expected user message to be truncated; all {} 'A's survived",
            a_count
        );
        assert!(
            a_count > 0,
            "expected some of the user message to survive truncation"
        );

        // Head and tail of the message must both be preserved (that's the
        // whole point of middle-trim over head-only).
        assert!(
            prompt.contains(head),
            "head marker missing — head was not preserved"
        );
        assert!(
            prompt.contains(tail),
            "tail marker missing — tail was not preserved"
        );

        // Trim marker must be present so the router model can see that
        // content was omitted.
        assert!(
            prompt.contains(TRIM_MARKER),
            "ellipsis trim marker missing from truncated prompt"
        );

        // Routing prompt scaffolding remains intact.
        assert!(prompt.contains("<conversation>"));
        assert!(prompt.contains("<routes>"));
    }

    #[test]
    fn test_turn_cap_limits_routing_history() {
        // The outer turn-cap guardrail should keep only the last
        // `MAX_ROUTING_TURNS` filtered messages regardless of how long the
        // conversation is. We build a conversation with alternating
        // user/assistant turns tagged with their index and verify that only
        // the tail of the conversation makes it into the prompt.
        let orchestrations_str = r#"
          {
            "gpt-4o": [
              {"name": "Image generation", "description": "generating image"}
            ]
        }
        "#;
        let agent_orchestrations = serde_json::from_str::<
            HashMap<String, Vec<OrchestrationPreference>>,
        >(orchestrations_str)
        .unwrap();

        let orchestrator =
            OrchestratorModelV1::new(agent_orchestrations, "test-model".to_string(), usize::MAX);

        let mut conversation: Vec<Message> = Vec::new();
        let total_turns = MAX_ROUTING_TURNS * 2; // well past the cap
        for i in 0..total_turns {
            let role = if i % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            conversation.push(Message {
                role,
                content: Some(MessageContent::Text(format!("turn-{i:03}"))),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
        }

        let req = orchestrator.generate_request(&conversation, &None);
        let prompt = req.messages[0].content.extract_text();

        // The last MAX_ROUTING_TURNS messages (indexes total-cap..total)
        // must all appear.
        for i in (total_turns - MAX_ROUTING_TURNS)..total_turns {
            let tag = format!("turn-{i:03}");
            assert!(
                prompt.contains(&tag),
                "expected recent turn tag {tag} to be present"
            );
        }

        // And earlier turns (indexes 0..total-cap) must all be dropped.
        for i in 0..(total_turns - MAX_ROUTING_TURNS) {
            let tag = format!("turn-{i:03}");
            assert!(
                !prompt.contains(&tag),
                "old turn tag {tag} leaked past turn cap into the prompt"
            );
        }
    }

    #[test]
    fn test_trim_middle_utf8_helper() {
        // No-op when already small enough.
        assert_eq!(trim_middle_utf8("hello", 100), "hello");
        assert_eq!(trim_middle_utf8("hello", 5), "hello");

        // 60/40 split with ellipsis when too long.
        let long = "a".repeat(20);
        let out = trim_middle_utf8(&long, 10);
        assert!(out.len() <= 10);
        assert!(out.contains(TRIM_MARKER));
        // Exactly one ellipsis, rest are 'a's.
        assert_eq!(out.matches(TRIM_MARKER).count(), 1);
        assert!(out.chars().filter(|c| *c == 'a').count() > 0);

        // When max_bytes is smaller than the marker, falls back to
        // head-only truncation (no marker).
        let out = trim_middle_utf8("abcdefgh", 2);
        assert_eq!(out, "ab");

        // UTF-8 boundary safety: 2-byte chars.
        let s = "é".repeat(50); // 100 bytes
        let out = trim_middle_utf8(&s, 25);
        assert!(out.len() <= 25);
        // Must still be valid UTF-8 that only contains 'é' and the marker.
        let ok = out.chars().all(|c| c == 'é' || c == '…');
        assert!(ok, "unexpected char in trimmed output: {out:?}");
    }

    #[test]
    fn test_non_text_input() {
        let expected_prompt = r#"
You are a helpful assistant that selects the most suitable routes based on user intent.
You are provided with a list of available routes enclosed within <routes></routes> XML tags:
<routes>
{"name": "Image generation", "description": "generating image", "parameters": {"type": "object", "properties": {}, "required": []}}
</routes>

You are also given the conversation context enclosed within <conversation></conversation> XML tags:
<conversation>
[
    {
        "role": "user",
        "content": "hi"
    },
    {
        "role": "assistant",
        "content": "Hello! How can I assist you today?"
    },
    {
        "role": "user",
        "content": "given the image In style of Andy Warhol, portrait of Bart and Lisa Simpson"
    }
]
</conversation>

## Instructions
1. Analyze the latest user intent from the conversation.
2. Compare it against the available routes to find which routes can help fulfill the request.
3. Respond only with the exact route names from <routes>.
4. If no routes can help or the intent is already fulfilled, return an empty list.

## Response Format
Return your answer strictly in JSON as follows:
{{"route": ["route_name_1", "route_name_2", "..."]}}
If no routes are needed, return an empty list for `route`.
"#;
        let orchestrations_str = r#"
          {
            "gpt-4o": [
              {"name": "Image generation", "description": "generating image"}
            ]
        }
        "#;
        let agent_orchestrations = serde_json::from_str::<
            HashMap<String, Vec<OrchestrationPreference>>,
        >(orchestrations_str)
        .unwrap();
        let orchestration_model = "test-model".to_string();
        let orchestrator = OrchestratorModelV1::new(
            agent_orchestrations,
            orchestration_model.clone(),
            usize::MAX,
        );

        let conversation_str = r#"
                    [
                        {
                            "role": "user",
                            "content": [
                              {
                                "type": "text",
                                "text": "hi"
                              },
                              {
                                "type": "image_url",
                                "image_url": {
                                  "url": "https://example.com/image.png"
                                }
                              }
                            ]
                        },
                        {
                            "role": "assistant",
                            "content": "Hello! How can I assist you today?"
                        },
                        {
                            "role": "user",
                            "content": "given the image In style of Andy Warhol, portrait of Bart and Lisa Simpson"
                        }
                    ]
        "#;
        let conversation: Vec<Message> = serde_json::from_str(conversation_str).unwrap();

        let req = orchestrator.generate_request(&conversation, &None);

        let prompt = req.messages[0].content.extract_text();

        assert_eq!(expected_prompt, prompt);
    }

    #[test]
    fn test_skip_tool_call() {
        let expected_prompt = r#"
You are a helpful assistant that selects the most suitable routes based on user intent.
You are provided with a list of available routes enclosed within <routes></routes> XML tags:
<routes>
{"name": "Image generation", "description": "generating image", "parameters": {"type": "object", "properties": {}, "required": []}}
</routes>

You are also given the conversation context enclosed within <conversation></conversation> XML tags:
<conversation>
[
    {
        "role": "user",
        "content": "What's the weather like in Tokyo?"
    },
    {
        "role": "assistant",
        "content": "The current weather in Tokyo is 22°C and sunny."
    },
    {
        "role": "user",
        "content": "What about in New York?"
    }
]
</conversation>

## Instructions
1. Analyze the latest user intent from the conversation.
2. Compare it against the available routes to find which routes can help fulfill the request.
3. Respond only with the exact route names from <routes>.
4. If no routes can help or the intent is already fulfilled, return an empty list.

## Response Format
Return your answer strictly in JSON as follows:
{{"route": ["route_name_1", "route_name_2", "..."]}}
If no routes are needed, return an empty list for `route`.
"#;
        let orchestrations_str = r#"
          {
            "gpt-4o": [
              {"name": "Image generation", "description": "generating image"}
            ]
        }
        "#;
        let agent_orchestrations = serde_json::from_str::<
            HashMap<String, Vec<OrchestrationPreference>>,
        >(orchestrations_str)
        .unwrap();
        let orchestration_model = "test-model".to_string();
        let orchestrator = OrchestratorModelV1::new(
            agent_orchestrations,
            orchestration_model.clone(),
            usize::MAX,
        );

        let conversation_str = r#"
                                                [
                                                  {
                                                    "role": "user",
                                                    "content": "What's the weather like in Tokyo?"
                                                  },
                                                  {
                                                    "role": "assistant",
                                                    "content": "",
                                                    "tool_calls": [
                                                      {
                                                        "id": "toolcall-abc123",
                                                        "type": "function",
                                                        "function": {
                                                          "name": "get_weather",
                                                          "arguments": "{ \"location\": \"Tokyo\" }"
                                                        }
                                                      }
                                                    ]
                                                  },
                                                  {
                                                    "role": "tool",
                                                    "tool_call_id": "toolcall-abc123",
                                                    "content": "{ \"temperature\": \"22°C\", \"condition\": \"Sunny\" }"
                                                  },
                                                  {
                                                    "role": "assistant",
                                                    "content": "The current weather in Tokyo is 22°C and sunny."
                                                  },
                                                  {
                                                    "role": "user",
                                                    "content": "What about in New York?"
                                                  }
                                                ]
        "#;

        // expects conversation to look like this

        // [
        //   {
        //     "role": "user",
        //     "content": "What's the weather like in Tokyo?"
        //   },
        //   {
        //     "role": "assistant",
        //     "content": "The current weather in Tokyo is 22°C and sunny."
        //   },
        //   {
        //     "role": "user",
        //     "content": "What about in New York?"
        //   }
        // ]

        let conversation: Vec<Message> = serde_json::from_str(conversation_str).unwrap();

        let req: ChatCompletionsRequest = orchestrator.generate_request(&conversation, &None);

        let prompt = req.messages[0].content.extract_text();

        assert_eq!(expected_prompt, prompt);
    }

    #[test]
    fn test_parse_response() {
        let orchestrations_str = r#"
          {
            "gpt-4o": [
              {"name": "Image generation", "description": "generating image"},
              {"name": "Code generation", "description": "generating code"}
            ]
        }
        "#;
        let agent_orchestrations = serde_json::from_str::<
            HashMap<String, Vec<OrchestrationPreference>>,
        >(orchestrations_str)
        .unwrap();

        let orchestrator =
            OrchestratorModelV1::new(agent_orchestrations, "test-model".to_string(), 2000);

        // Case 1: Valid JSON with single route in array
        let input = r#"{"route": ["Image generation"]}"#;
        let result = orchestrator.parse_response(input, &None).unwrap();
        assert_eq!(
            result,
            Some(vec![("Image generation".to_string(), "gpt-4o".to_string())])
        );

        // Case 2: Valid JSON with multiple routes in array
        let input = r#"{"route": ["Image generation", "Code generation"]}"#;
        let result = orchestrator.parse_response(input, &None).unwrap();
        assert_eq!(
            result,
            Some(vec![
                ("Image generation".to_string(), "gpt-4o".to_string()),
                ("Code generation".to_string(), "gpt-4o".to_string())
            ])
        );

        // Case 3: Valid JSON with empty array
        let input = r#"{"route": []}"#;
        let result = orchestrator.parse_response(input, &None).unwrap();
        assert_eq!(result, None);

        // Case 4: Valid JSON with null route
        let input = r#"{"route": null}"#;
        let result = orchestrator.parse_response(input, &None).unwrap();
        assert_eq!(result, None);

        // Case 5: JSON missing route field
        let input = r#"{}"#;
        let result = orchestrator.parse_response(input, &None).unwrap();
        assert_eq!(result, None);

        // Case 5.1: empty string
        let input = r#""#;
        let result = orchestrator.parse_response(input, &None).unwrap();
        assert_eq!(result, None);

        // Case 6: Malformed JSON
        let input = r#"{"route": ["route1""#; // missing closing ]
        let result = orchestrator.parse_response(input, &None);
        assert!(result.is_err());

        // Case 7: Single quotes and \n in JSON
        let input = "{'route': ['Image generation']}\\n";
        let result = orchestrator.parse_response(input, &None).unwrap();
        assert_eq!(
            result,
            Some(vec![("Image generation".to_string(), "gpt-4o".to_string())])
        );

        // Case 8: Array with unknown route (not in orchestrations map)
        let input = r#"{"route": ["Unknown route"]}"#;
        let result = orchestrator.parse_response(input, &None).unwrap();
        assert_eq!(result, None);
    }
}
