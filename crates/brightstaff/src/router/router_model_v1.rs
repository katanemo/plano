use std::collections::HashMap;

use common::configuration::{ModelUsagePreference, RoutingPreference};
use hermesllm::apis::openai::{ChatCompletionsRequest, Message, MessageContent, Role};
use hermesllm::transforms::lib::ExtractText;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use super::router_model::{RouterModel, RoutingModelError};

pub const MAX_TOKEN_LEN: usize = 2048; // Default max token length for the routing model
pub const ARCH_ROUTER_V1_SYSTEM_PROMPT: &str = r#"
You are a helpful assistant designed to find the best suited route.
You are provided with route description within <routes></routes> XML tags:
<routes>
{routes}
</routes>

<conversation>
{conversation}
</conversation>

Your task is to decide which route is best suit with user intent on the conversation in <conversation></conversation> XML tags.  Follow the instruction:
1. If the latest intent from user is irrelevant or user intent is full filled, response with other route {"route": "other"}.
2. You must analyze the route descriptions and find the best match route for user latest intent.
3. You only response the name of the route that best matches the user's request, use the exact name in the <routes></routes>.

Based on your analysis, provide your response in the following JSON formats if you decide to match any route:
{"route": "route_name"}
"#;

pub type Result<T> = std::result::Result<T, RoutingModelError>;
pub struct RouterModelV1 {
    llm_route_json_str: String,
    llm_route_to_model_map: HashMap<String, String>,
    routing_model: String,
    max_token_length: usize,
}
impl RouterModelV1 {
    pub fn new(
        llm_routes: HashMap<String, Vec<RoutingPreference>>,
        routing_model: String,
        max_token_length: usize,
    ) -> Self {
        let llm_route_values: Vec<RoutingPreference> =
            llm_routes.values().flatten().cloned().collect();
        let llm_route_json_str =
            serde_json::to_string(&llm_route_values).unwrap_or_else(|_| "[]".to_string());
        let llm_route_to_model_map: HashMap<String, String> = llm_routes
            .iter()
            .flat_map(|(model, prefs)| prefs.iter().map(|pref| (pref.name.clone(), model.clone())))
            .collect();

        RouterModelV1 {
            routing_model,
            max_token_length,
            llm_route_json_str,
            llm_route_to_model_map,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LlmRouterResponse {
    pub route: Option<String>,
}

const TOKEN_LENGTH_DIVISOR: usize = 4; // Approximate token length divisor for UTF-8 characters

impl RouterModel for RouterModelV1 {
    fn generate_request(
        &self,
        messages: &[Message],
        usage_preferences_from_request: &Option<Vec<ModelUsagePreference>>,
    ) -> ChatCompletionsRequest {
        // remove system prompt, tool calls, tool call response and messages without content
        // if content is empty its likely a tool call
        // when role == tool its tool call response
        let messages_vec = messages
            .iter()
            .filter(|m| {
                m.role != Role::System
                    && m.role != Role::Tool
                    && !m.content.extract_text().is_empty()
            })
            .collect::<Vec<&Message>>();

        // Following code is to ensure that the conversation does not exceed max token length
        // Note: we use a simple heuristic to estimate token count based on character length to optimize for performance
        let mut token_count = ARCH_ROUTER_V1_SYSTEM_PROMPT.len() / TOKEN_LENGTH_DIVISOR;
        let mut selected_messages_list_reversed: Vec<&Message> = vec![];
        for (selected_messsage_count, message) in messages_vec.iter().rev().enumerate() {
            let message_token_count = message.content.extract_text().len() / TOKEN_LENGTH_DIVISOR;
            token_count += message_token_count;
            if token_count > self.max_token_length {
                debug!(
                    token_count = token_count,
                    max_tokens = self.max_token_length,
                    selected = selected_messsage_count,
                    total = messages_vec.len(),
                    "token count exceeds max, truncating conversation"
                );
                if message.role == Role::User {
                    // If message that exceeds max token length is from user, we need to keep it
                    selected_messages_list_reversed.push(message);
                }
                break;
            }
            // If we are here, it means that the message is within the max token length
            selected_messages_list_reversed.push(message);
        }

        if selected_messages_list_reversed.is_empty() {
            debug!("no messages selected, using last message");
            if let Some(last_message) = messages_vec.last() {
                selected_messages_list_reversed.push(last_message);
            }
        }

        // ensure that first and last selected message is from user
        if let Some(first_message) = selected_messages_list_reversed.first() {
            if first_message.role != Role::User {
                warn!("last message is not from user, may lead to incorrect routing");
            }
        }
        if let Some(last_message) = selected_messages_list_reversed.last() {
            if last_message.role != Role::User {
                warn!("first message is not from user, may lead to incorrect routing");
            }
        }

        // Reverse the selected messages to maintain the conversation order
        let selected_conversation_list = selected_messages_list_reversed
            .iter()
            .rev()
            .map(|message| {
                Message {
                    role: message.role.clone(),
                    // we can unwrap here because we have already filtered out messages without content
                    content: Some(MessageContent::Text(
                        message
                            .content
                            .as_ref()
                            .map_or(String::new(), |c| c.to_string()),
                    )),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                }
            })
            .collect::<Vec<Message>>();

        // Generate the router request message based on the usage preferences.
        // If preferences are passed in request then we use them otherwise we use the default routing model preferences.
        let router_message = match convert_to_router_preferences(usage_preferences_from_request) {
            Some(prefs) => generate_router_message(&prefs, &selected_conversation_list),
            None => generate_router_message(&self.llm_route_json_str, &selected_conversation_list),
        };

        ChatCompletionsRequest {
            model: self.routing_model.clone(),
            messages: vec![Message {
                content: Some(MessageContent::Text(router_message)),
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
        usage_preferences: &Option<Vec<ModelUsagePreference>>,
    ) -> Result<Option<(String, String)>> {
        if content.is_empty() {
            return Ok(None);
        }
        let router_resp_fixed = fix_json_response(content);
        let router_response: LlmRouterResponse = serde_json::from_str(router_resp_fixed.as_str())?;

        let selected_route = router_response.route.unwrap_or_default().to_string();

        if selected_route.is_empty() || selected_route == "other" {
            return Ok(None);
        }

        if let Some(usage_preferences) = usage_preferences {
            // If usage preferences are defined, we need to find the model that matches the selected route
            let model_name: Option<String> = usage_preferences
                .iter()
                .map(|pref| {
                    pref.routing_preferences
                        .iter()
                        .find(|routing_pref| routing_pref.name == selected_route)
                        .map(|_| pref.model.clone())
                })
                .find_map(|model| model);

            if let Some(model_name) = model_name {
                return Ok(Some((selected_route, model_name)));
            } else {
                warn!(
                    route = %selected_route,
                    preferences = ?usage_preferences,
                    "no matching model found for route"
                );
                return Ok(None);
            }
        }

        // If no usage preferences are passed in request then use the default routing model preferences
        if let Some(model) = self.llm_route_to_model_map.get(&selected_route).cloned() {
            return Ok(Some((selected_route, model)));
        }

        warn!(
            route = %selected_route,
            preferences = ?self.llm_route_to_model_map,
            "no model found for route"
        );

        Ok(None)
    }

    fn get_model_name(&self) -> String {
        self.routing_model.clone()
    }
}

fn generate_router_message(prefs: &str, selected_conversation_list: &Vec<Message>) -> String {
    ARCH_ROUTER_V1_SYSTEM_PROMPT
        .replace("{routes}", prefs)
        .replace(
            "{conversation}",
            &serde_json::to_string(&selected_conversation_list).unwrap_or_default(),
        )
}

fn convert_to_router_preferences(
    prefs_from_request: &Option<Vec<ModelUsagePreference>>,
) -> Option<String> {
    if let Some(usage_preferences) = prefs_from_request {
        let routing_preferences = usage_preferences
            .iter()
            .flat_map(|pref| {
                pref.routing_preferences
                    .iter()
                    .map(|routing_pref| RoutingPreference {
                        name: routing_pref.name.clone(),
                        description: routing_pref.description.clone(),
                    })
            })
            .collect::<Vec<RoutingPreference>>();

        return Some(serde_json::to_string(&routing_preferences).unwrap_or_default());
    }

    None
}

fn fix_json_response(body: &str) -> String {
    let mut updated_body = body.to_string();

    updated_body = updated_body.replace("'", "\"");

    if updated_body.contains("\\n") {
        updated_body = updated_body.replace("\\n", "");
    }

    if updated_body.starts_with("```json") {
        updated_body = updated_body
            .strip_prefix("```json")
            .unwrap_or(&updated_body)
            .to_string();
    }

    if updated_body.ends_with("```") {
        updated_body = updated_body
            .strip_suffix("```")
            .unwrap_or(&updated_body)
            .to_string();
    }

    updated_body
}

impl std::fmt::Debug for dyn RouterModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RouterModel")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_system_prompt_format() {
        let expected_prompt = r#"
You are a helpful assistant designed to find the best suited route.
You are provided with route description within <routes></routes> XML tags:
<routes>
[{"name":"Image generation","description":"generating image"}]
</routes>

<conversation>
[{"role":"user","content":"hi"},{"role":"assistant","content":"Hello! How can I assist you today?"},{"role":"user","content":"given the image In style of Andy Warhol, portrait of Bart and Lisa Simpson"}]
</conversation>

Your task is to decide which route is best suit with user intent on the conversation in <conversation></conversation> XML tags.  Follow the instruction:
1. If the latest intent from user is irrelevant or user intent is full filled, response with other route {"route": "other"}.
2. You must analyze the route descriptions and find the best match route for user latest intent.
3. You only response the name of the route that best matches the user's request, use the exact name in the <routes></routes>.

Based on your analysis, provide your response in the following JSON formats if you decide to match any route:
{"route": "route_name"}
"#;
        let routes_str = r#"
          {
            "gpt-4o": [
              {"name": "Image generation", "description": "generating image"}
            ]
        }
        "#;
        let llm_routes =
            serde_json::from_str::<HashMap<String, Vec<RoutingPreference>>>(routes_str).unwrap();
        let routing_model = "test-model".to_string();
        let router = RouterModelV1::new(llm_routes, routing_model, usize::MAX);

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

        let req = router.generate_request(&conversation, &None);

        let prompt = req.messages[0].content.extract_text();

        assert_eq!(expected_prompt, prompt);
    }

    #[test]
    fn test_system_prompt_format_usage_preferences() {
        let expected_prompt = r#"
You are a helpful assistant designed to find the best suited route.
You are provided with route description within <routes></routes> XML tags:
<routes>
[{"name":"code-generation","description":"generating new code snippets, functions, or boilerplate based on user prompts or requirements"}]
</routes>

<conversation>
[{"role":"user","content":"hi"},{"role":"assistant","content":"Hello! How can I assist you today?"},{"role":"user","content":"given the image In style of Andy Warhol, portrait of Bart and Lisa Simpson"}]
</conversation>

Your task is to decide which route is best suit with user intent on the conversation in <conversation></conversation> XML tags.  Follow the instruction:
1. If the latest intent from user is irrelevant or user intent is full filled, response with other route {"route": "other"}.
2. You must analyze the route descriptions and find the best match route for user latest intent.
3. You only response the name of the route that best matches the user's request, use the exact name in the <routes></routes>.

Based on your analysis, provide your response in the following JSON formats if you decide to match any route:
{"route": "route_name"}
"#;
        let routes_str = r#"
          {
            "gpt-4o": [
              {"name": "Image generation", "description": "generating image"}
            ]
        }
        "#;
        let llm_routes =
            serde_json::from_str::<HashMap<String, Vec<RoutingPreference>>>(routes_str).unwrap();
        let routing_model = "test-model".to_string();
        let router = RouterModelV1::new(llm_routes, routing_model, usize::MAX);

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

        let usage_preferences = Some(vec![ModelUsagePreference {
            model: "claude/claude-3-7-sonnet".to_string(),
            routing_preferences: vec![RoutingPreference {
                name: "code-generation".to_string(),
                description: "generating new code snippets, functions, or boilerplate based on user prompts or requirements".to_string(),
            }],
        }]);
        let req = router.generate_request(&conversation, &usage_preferences);

        let prompt = req.messages[0].content.extract_text();

        assert_eq!(expected_prompt, prompt);
    }

    #[test]
    fn test_conversation_exceed_token_count() {
        let expected_prompt = r#"
You are a helpful assistant designed to find the best suited route.
You are provided with route description within <routes></routes> XML tags:
<routes>
[{"name":"Image generation","description":"generating image"}]
</routes>

<conversation>
[{"role":"user","content":"given the image In style of Andy Warhol, portrait of Bart and Lisa Simpson"}]
</conversation>

Your task is to decide which route is best suit with user intent on the conversation in <conversation></conversation> XML tags.  Follow the instruction:
1. If the latest intent from user is irrelevant or user intent is full filled, response with other route {"route": "other"}.
2. You must analyze the route descriptions and find the best match route for user latest intent.
3. You only response the name of the route that best matches the user's request, use the exact name in the <routes></routes>.

Based on your analysis, provide your response in the following JSON formats if you decide to match any route:
{"route": "route_name"}
"#;

        let routes_str = r#"
          {
            "gpt-4o": [
              {"name": "Image generation", "description": "generating image"}
            ]
        }
        "#;
        let llm_routes =
            serde_json::from_str::<HashMap<String, Vec<RoutingPreference>>>(routes_str).unwrap();
        let routing_model = "test-model".to_string();
        let router = RouterModelV1::new(llm_routes, routing_model, 235);

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

        let req = router.generate_request(&conversation, &None);

        let prompt = req.messages[0].content.extract_text();

        assert_eq!(expected_prompt, prompt);
    }

    #[test]
    fn test_conversation_exceed_token_count_large_single_message() {
        let expected_prompt = r#"
You are a helpful assistant designed to find the best suited route.
You are provided with route description within <routes></routes> XML tags:
<routes>
[{"name":"Image generation","description":"generating image"}]
</routes>

<conversation>
[{"role":"user","content":"given the image In style of Andy Warhol, portrait of Bart and Lisa Simpson and this is a very long message that exceeds the max token length of the routing model, so it should be truncated and only the last user message should be included in the conversation for routing."}]
</conversation>

Your task is to decide which route is best suit with user intent on the conversation in <conversation></conversation> XML tags.  Follow the instruction:
1. If the latest intent from user is irrelevant or user intent is full filled, response with other route {"route": "other"}.
2. You must analyze the route descriptions and find the best match route for user latest intent.
3. You only response the name of the route that best matches the user's request, use the exact name in the <routes></routes>.

Based on your analysis, provide your response in the following JSON formats if you decide to match any route:
{"route": "route_name"}
"#;

        let routes_str = r#"
          {
            "gpt-4o": [
              {"name": "Image generation", "description": "generating image"}
            ]
        }
        "#;
        let llm_routes =
            serde_json::from_str::<HashMap<String, Vec<RoutingPreference>>>(routes_str).unwrap();

        let routing_model = "test-model".to_string();
        let router = RouterModelV1::new(llm_routes, routing_model, 200);

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

        let req = router.generate_request(&conversation, &None);

        let prompt = req.messages[0].content.extract_text();

        assert_eq!(expected_prompt, prompt);
    }

    #[test]
    fn test_conversation_trim_upto_user_message() {
        let expected_prompt = r#"
You are a helpful assistant designed to find the best suited route.
You are provided with route description within <routes></routes> XML tags:
<routes>
[{"name":"Image generation","description":"generating image"}]
</routes>

<conversation>
[{"role":"user","content":"given the image In style of Andy Warhol"},{"role":"assistant","content":"ok here is the image"},{"role":"user","content":"pls give me another image about Bart and Lisa"}]
</conversation>

Your task is to decide which route is best suit with user intent on the conversation in <conversation></conversation> XML tags.  Follow the instruction:
1. If the latest intent from user is irrelevant or user intent is full filled, response with other route {"route": "other"}.
2. You must analyze the route descriptions and find the best match route for user latest intent.
3. You only response the name of the route that best matches the user's request, use the exact name in the <routes></routes>.

Based on your analysis, provide your response in the following JSON formats if you decide to match any route:
{"route": "route_name"}
"#;

        let routes_str = r#"
          {
            "gpt-4o": [
              {"name": "Image generation", "description": "generating image"}
            ]
        }
        "#;
        let llm_routes =
            serde_json::from_str::<HashMap<String, Vec<RoutingPreference>>>(routes_str).unwrap();
        let routing_model = "test-model".to_string();
        let router = RouterModelV1::new(llm_routes, routing_model, 230);

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

        let req = router.generate_request(&conversation, &None);

        let prompt = req.messages[0].content.extract_text();

        assert_eq!(expected_prompt, prompt);
    }

    #[test]
    fn test_non_text_input() {
        let expected_prompt = r#"
You are a helpful assistant designed to find the best suited route.
You are provided with route description within <routes></routes> XML tags:
<routes>
[{"name":"Image generation","description":"generating image"}]
</routes>

<conversation>
[{"role":"user","content":"hi"},{"role":"assistant","content":"Hello! How can I assist you today?"},{"role":"user","content":"given the image In style of Andy Warhol, portrait of Bart and Lisa Simpson"}]
</conversation>

Your task is to decide which route is best suit with user intent on the conversation in <conversation></conversation> XML tags.  Follow the instruction:
1. If the latest intent from user is irrelevant or user intent is full filled, response with other route {"route": "other"}.
2. You must analyze the route descriptions and find the best match route for user latest intent.
3. You only response the name of the route that best matches the user's request, use the exact name in the <routes></routes>.

Based on your analysis, provide your response in the following JSON formats if you decide to match any route:
{"route": "route_name"}
"#;
        let routes_str = r#"
          {
            "gpt-4o": [
              {"name": "Image generation", "description": "generating image"}
            ]
        }
        "#;
        let llm_routes =
            serde_json::from_str::<HashMap<String, Vec<RoutingPreference>>>(routes_str).unwrap();
        let routing_model = "test-model".to_string();
        let router = RouterModelV1::new(llm_routes, routing_model, usize::MAX);

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

        let req = router.generate_request(&conversation, &None);

        let prompt = req.messages[0].content.extract_text();

        assert_eq!(expected_prompt, prompt);
    }

    #[test]
    fn test_skip_tool_call() {
        let expected_prompt = r#"
You are a helpful assistant designed to find the best suited route.
You are provided with route description within <routes></routes> XML tags:
<routes>
[{"name":"Image generation","description":"generating image"}]
</routes>

<conversation>
[{"role":"user","content":"What's the weather like in Tokyo?"},{"role":"assistant","content":"The current weather in Tokyo is 22°C and sunny."},{"role":"user","content":"What about in New York?"}]
</conversation>

Your task is to decide which route is best suit with user intent on the conversation in <conversation></conversation> XML tags.  Follow the instruction:
1. If the latest intent from user is irrelevant or user intent is full filled, response with other route {"route": "other"}.
2. You must analyze the route descriptions and find the best match route for user latest intent.
3. You only response the name of the route that best matches the user's request, use the exact name in the <routes></routes>.

Based on your analysis, provide your response in the following JSON formats if you decide to match any route:
{"route": "route_name"}
"#;
        let routes_str = r#"
          {
            "gpt-4o": [
              {"name": "Image generation", "description": "generating image"}
            ]
        }
        "#;
        let llm_routes =
            serde_json::from_str::<HashMap<String, Vec<RoutingPreference>>>(routes_str).unwrap();
        let routing_model = "test-model".to_string();
        let router = RouterModelV1::new(llm_routes, routing_model, usize::MAX);

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

        let req: ChatCompletionsRequest = router.generate_request(&conversation, &None);

        let prompt = req.messages[0].content.extract_text();

        assert_eq!(expected_prompt, prompt);
    }

    #[test]
    fn test_parse_response() {
        let routes_str = r#"
          {
            "gpt-4o": [
              {"name": "Image generation", "description": "generating image"}
            ]
        }
        "#;
        let llm_routes =
            serde_json::from_str::<HashMap<String, Vec<RoutingPreference>>>(routes_str).unwrap();

        let router = RouterModelV1::new(llm_routes, "test-model".to_string(), 2000);

        // Case 1: Valid JSON with non-empty route
        let input = r#"{"route": "Image generation"}"#;
        let result = router.parse_response(input, &None).unwrap();
        assert_eq!(
            result,
            Some(("Image generation".to_string(), "gpt-4o".to_string()))
        );

        // Case 2: Valid JSON with empty route
        let input = r#"{"route": ""}"#;
        let result = router.parse_response(input, &None).unwrap();
        assert_eq!(result, None);

        // Case 3: Valid JSON with null route
        let input = r#"{"route": null}"#;
        let result = router.parse_response(input, &None).unwrap();
        assert_eq!(result, None);

        // Case 4: JSON missing route field
        let input = r#"{}"#;
        let result = router.parse_response(input, &None).unwrap();
        assert_eq!(result, None);

        // Case 4.1: empty string
        let input = r#""#;
        let result = router.parse_response(input, &None).unwrap();
        assert_eq!(result, None);

        // Case 5: Malformed JSON
        let input = r#"{"route": "route1""#; // missing closing }
        let result = router.parse_response(input, &None);
        assert!(result.is_err());

        // Case 6: Single quotes and \n in JSON
        let input = "{'route': 'Image generation'}\\n";
        let result = router.parse_response(input, &None).unwrap();
        assert_eq!(
            result,
            Some(("Image generation".to_string(), "gpt-4o".to_string()))
        );

        // Case 7: Code block marker
        let input = "```json\n{\"route\": \"Image generation\"}\n```";
        let result = router.parse_response(input, &None).unwrap();
        assert_eq!(
            result,
            Some(("Image generation".to_string(), "gpt-4o".to_string()))
        );
    }
}
