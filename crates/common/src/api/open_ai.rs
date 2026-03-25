use crate::{
    configuration::LlmProvider,
    consts::{ARCH_FC_MODEL_NAME, ASSISTANT_ROLE},
};
use serde::{ser::SerializeMap, Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    fmt::Display,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionsRequest {
    #[serde(default)]
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ChatCompletionTool>>,
    #[serde(default)]
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<StreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ToolType {
    #[serde(rename = "function")]
    Function,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionTool {
    #[serde(rename = "type")]
    pub tool_type: ToolType,
    pub function: FunctionDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: FunctionParameters,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FunctionParameters {
    pub properties: HashMap<String, FunctionParameter>,
}

impl Serialize for FunctionParameters {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // select all requried parameters
        let required: Vec<&String> = self
            .properties
            .iter()
            .filter(|(_, v)| v.required.unwrap_or(false))
            .map(|(k, _)| k)
            .collect();
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("properties", &self.properties)?;
        if !required.is_empty() {
            map.serialize_entry("required", &required)?;
        }
        map.end()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FunctionParameter {
    #[serde(rename = "type")]
    #[serde(default = "ParameterType::string")]
    pub parameter_type: ParameterType,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "enum")]
    pub enum_values: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

impl Serialize for FunctionParameter {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(Some(5))?;
        map.serialize_entry("type", &self.parameter_type)?;
        map.serialize_entry("description", &self.description)?;
        if let Some(enum_values) = &self.enum_values {
            map.serialize_entry("enum", enum_values)?;
        }
        if let Some(default) = &self.default {
            map.serialize_entry("default", default)?;
        }
        if let Some(format) = &self.format {
            map.serialize_entry("format", format)?;
        }
        map.end()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ParameterType {
    #[serde(rename = "int")]
    Int,
    #[serde(rename = "float")]
    Float,
    #[serde(rename = "bool")]
    Bool,
    #[serde(rename = "str")]
    String,
    #[serde(rename = "list")]
    List,
    #[serde(rename = "dict")]
    Dict,
}

impl From<String> for ParameterType {
    fn from(s: String) -> Self {
        match s.as_str() {
            "int" => ParameterType::Int,
            "integer" => ParameterType::Int,
            "float" => ParameterType::Float,
            "bool" => ParameterType::Bool,
            "boolean" => ParameterType::Bool,
            "str" => ParameterType::String,
            "string" => ParameterType::String,
            "list" => ParameterType::List,
            "array" => ParameterType::List,
            "dict" => ParameterType::Dict,
            "dictionary" => ParameterType::Dict,
            _ => {
                log::warn!("Unknown parameter type: {}, assuming type str", s);
                ParameterType::String
            }
        }
    }
}

impl ParameterType {
    pub fn string() -> ParameterType {
        ParameterType::String
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamOptions {
    pub include_usage: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MultiPartContentType {
    #[serde(rename = "text")]
    Text,
    #[serde(rename = "image_url")]
    ImageUrl,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MultiPartContent {
    pub text: Option<String>,
    #[serde(rename = "type")]
    pub content_type: MultiPartContentType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ContentType {
    Text(String),
    MultiPart(Vec<MultiPartContent>),
}

impl Display for ContentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContentType::Text(text) => write!(f, "{}", text),
            ContentType::MultiPart(multi_part) => {
                let text_parts: Vec<String> = multi_part
                    .iter()
                    .filter_map(|part| {
                        if part.content_type == MultiPartContentType::Text {
                            part.text.clone()
                        } else if part.content_type == MultiPartContentType::ImageUrl {
                            // skip image URLs or their data in text representation
                            None
                        } else {
                            None
                        }
                    })
                    .collect();
                let combined_text = text_parts.join("\n");
                write!(f, "{}", combined_text)
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<ContentType>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn new(role: String, content: String) -> Self {
        let content = Some(ContentType::Text(content));
        Message {
            role,
            content,
            model: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }
}

impl Default for Message {
    fn default() -> Self {
        Message {
            role: ASSISTANT_ROLE.to_string(),
            content: None,
            model: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    pub finish_reason: Option<String>,
    pub index: Option<usize>,
    pub message: Message,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub tool_type: ToolType,
    pub function: FunctionCallDetail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCallDetail {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ToolCallState {
    pub key: String,
    pub message: Option<Message>,
    pub tool_call: FunctionCallDetail,
    pub tool_response: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ArchState {
    ToolCall(Vec<ToolCallState>),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelServerErrorResponse {
    pub result: String,
    pub intent_latency: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionsResponse {
    pub usage: Option<Usage>,
    pub choices: Vec<Choice>,
    pub model: String,
    pub metadata: Option<HashMap<String, String>>,
}

impl ChatCompletionsResponse {
    pub fn new(message: String) -> Self {
        ChatCompletionsResponse {
            choices: vec![Choice {
                message: Message {
                    role: ASSISTANT_ROLE.to_string(),
                    content: Some(ContentType::Text(message)),
                    model: Some(ARCH_FC_MODEL_NAME.to_string()),
                    tool_calls: None,
                    tool_call_id: None,
                },
                index: Some(0),
                finish_reason: Some("done".to_string()),
            }],
            usage: None,
            model: ARCH_FC_MODEL_NAME.to_string(),
            metadata: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub completion_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionStreamResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub choices: Vec<ChunkChoice>,
}

impl ChatCompletionStreamResponse {
    pub fn new(
        response: Option<String>,
        role: Option<String>,
        model: Option<String>,
        tool_calls: Option<Vec<ToolCall>>,
    ) -> Self {
        ChatCompletionStreamResponse {
            model,
            choices: vec![ChunkChoice {
                delta: Delta {
                    role,
                    content: response,
                    tool_calls,
                    model: None,
                    tool_call_id: None,
                },
                finish_reason: None,
            }],
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ChatCompletionChunkResponseError {
    #[error("failed to deserialize")]
    Deserialization(#[from] serde_json::Error),
    #[error("empty content in data chunk")]
    EmptyContent,
    #[error("no chunks present")]
    NoChunks,
}

pub struct ChatCompletionStreamResponseServerEvents {
    pub events: Vec<ChatCompletionStreamResponse>,
}

impl Display for ChatCompletionStreamResponseServerEvents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tokens_str = self
            .events
            .iter()
            .map(|response_chunk| {
                if response_chunk.choices.is_empty() {
                    return "".to_string();
                }
                response_chunk.choices[0]
                    .delta
                    .content
                    .clone()
                    .unwrap_or("".to_string())
            })
            .collect::<Vec<String>>()
            .join("");

        write!(f, "{}", tokens_str)
    }
}

impl TryFrom<&str> for ChatCompletionStreamResponseServerEvents {
    type Error = ChatCompletionChunkResponseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let response_chunks: VecDeque<ChatCompletionStreamResponse> = value
            .lines()
            .filter(|line| line.starts_with("data: "))
            .filter(|line| !line.starts_with(r#"data: {"type": "ping"}"#))
            .map(|line| line.get(6..).unwrap())
            .filter(|data_chunk| *data_chunk != "[DONE]")
            .map(serde_json::from_str::<ChatCompletionStreamResponse>)
            .collect::<Result<VecDeque<ChatCompletionStreamResponse>, _>>()?;

        Ok(ChatCompletionStreamResponseServerEvents {
            events: response_chunks.into(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkChoice {
    pub delta: Delta,
    // TODO: could this be an enum?
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

pub fn to_server_events(chunks: Vec<ChatCompletionStreamResponse>) -> String {
    let mut response_str = String::new();
    for chunk in chunks.iter() {
        response_str.push_str("data: ");
        response_str.push_str(&serde_json::to_string(&chunk).unwrap());
        response_str.push_str("\n\n");
    }
    response_str
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDetail {
    pub id: String,
    pub object: String,
    pub created: usize,
    pub owned_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModelObject {
    #[serde(rename = "list")]
    List,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Models {
    pub object: ModelObject,
    pub data: Vec<ModelDetail>,
}

impl From<Vec<LlmProvider>> for Models {
    fn from(llm_providers: Vec<LlmProvider>) -> Self {
        let data = llm_providers
            .iter()
            .map(|provider| ModelDetail {
                id: provider.name.clone(),
                object: "model".to_string(),
                created: 0,
                owned_by: "system".to_string(),
            })
            .collect();

        Models {
            object: ModelObject::List,
            data,
        }
    }
}

#[cfg(test)]
mod test {
    use crate::api::open_ai::{ChatCompletionsRequest, ContentType, MultiPartContentType};

    use super::{ChatCompletionStreamResponseServerEvents, Message};
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;

    const TOOL_SERIALIZED: &str = r#"{
  "model": "gpt-3.5-turbo",
  "messages": [
    {
      "role": "user",
      "content": "What city do you want to know the weather for?"
    }
  ],
  "tools": [
    {
      "type": "function",
      "function": {
        "name": "weather_forecast",
        "description": "function to retrieve weather forecast",
        "parameters": {
          "properties": {
            "city": {
              "type": "str",
              "description": "city for weather forecast",
              "default": "test"
            }
          },
          "required": [
            "city"
          ]
        }
      }
    }
  ],
  "stream": true,
  "stream_options": {
    "include_usage": true
  }
}"#;

    #[test]
    fn test_tool_type_request() {
        use super::{
            ChatCompletionTool, ChatCompletionsRequest, FunctionDefinition, FunctionParameter,
            FunctionParameters, ParameterType, StreamOptions, ToolType,
        };

        let mut properties = HashMap::new();
        properties.insert(
            "city".to_string(),
            FunctionParameter {
                parameter_type: ParameterType::String,
                description: "city for weather forecast".to_string(),
                required: Some(true),
                enum_values: None,
                default: Some("test".to_string()),
                format: None,
            },
        );

        let function_definition = FunctionDefinition {
            name: "weather_forecast".to_string(),
            description: "function to retrieve weather forecast".to_string(),
            parameters: FunctionParameters { properties },
        };

        let chat_completions_request = ChatCompletionsRequest {
            model: "gpt-3.5-turbo".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: Some(ContentType::Text(
                    "What city do you want to know the weather for?".to_string(),
                )),
                model: None,
                tool_calls: None,
                tool_call_id: None,
            }],
            tools: Some(vec![ChatCompletionTool {
                tool_type: ToolType::Function,
                function: function_definition,
            }]),
            stream: true,
            stream_options: Some(StreamOptions {
                include_usage: true,
            }),
            metadata: None,
        };

        let serialized = serde_json::to_string_pretty(&chat_completions_request).unwrap();
        println!("{}", serialized);
        assert_eq!(TOOL_SERIALIZED, serialized);
    }

    #[test]
    fn test_parameter_types() {
        use super::{FunctionParameter, ParameterType};

        const PARAMETER_SERIALZIED: &str = r#"{
  "city": {
    "type": "str",
    "description": "city for weather forecast",
    "default": "test"
  }
}"#;

        let properties = HashMap::from([(
            "city".to_string(),
            FunctionParameter {
                parameter_type: ParameterType::String,
                description: "city for weather forecast".to_string(),
                required: Some(true),
                enum_values: None,
                default: Some("test".to_string()),
                format: None,
            },
        )]);

        let serialized = serde_json::to_string_pretty(&properties).unwrap();
        assert_eq!(PARAMETER_SERIALZIED, serialized);

        // ensure that if type is missing it is set to string
        const PARAMETER_SERIALZIED_MISSING_TYPE: &str = r#"
        {
          "city": {
            "description": "city for weather forecast"
          }
        }"#;

        let missing_type_deserialized: HashMap<String, FunctionParameter> =
            serde_json::from_str(PARAMETER_SERIALZIED_MISSING_TYPE).unwrap();
        println!("{:?}", missing_type_deserialized);
        assert_eq!(
            missing_type_deserialized
                .get("city")
                .unwrap()
                .parameter_type,
            ParameterType::String
        );
    }

    #[test]
    fn stream_chunk_parse() {
        const CHUNK_RESPONSE: &str = r#"data: {"id":"chatcmpl-ALmdmtKulBMEq3fRLbrnxJwcKOqvS","object":"chat.completion.chunk","created":1729755226,"model":"gpt-3.5-turbo-0125","system_fingerprint":null,"choices":[{"index":0,"delta":{"role":"assistant","content":"","refusal":null},"logprobs":null,"finish_reason":null}]}

data: {"id":"chatcmpl-ALmdmtKulBMEq3fRLbrnxJwcKOqvS","object":"chat.completion.chunk","created":1729755226,"model":"gpt-3.5-turbo-0125","system_fingerprint":null,"choices":[{"index":0,"delta":{"content":"Hello"},"logprobs":null,"finish_reason":null}]}

data: {"id":"chatcmpl-ALmdmtKulBMEq3fRLbrnxJwcKOqvS","object":"chat.completion.chunk","created":1729755226,"model":"gpt-3.5-turbo-0125","system_fingerprint":null,"choices":[{"index":0,"delta":{"content":"!"},"logprobs":null,"finish_reason":null}]}

data: {"id":"chatcmpl-ALmdmtKulBMEq3fRLbrnxJwcKOqvS","object":"chat.completion.chunk","created":1729755226,"model":"gpt-3.5-turbo-0125","system_fingerprint":null,"choices":[{"index":0,"delta":{"content":" How"},"logprobs":null,"finish_reason":null}]}

data: {"id":"chatcmpl-ALmdmtKulBMEq3fRLbrnxJwcKOqvS","object":"chat.completion.chunk","created":1729755226,"model":"gpt-3.5-turbo-0125","system_fingerprint":null,"choices":[{"index":0,"delta":{"content":" can"},"logprobs":null,"finish_reason":null}]}


"#;

        let sever_events =
            ChatCompletionStreamResponseServerEvents::try_from(CHUNK_RESPONSE).unwrap();
        assert_eq!(sever_events.events.len(), 5);
        assert_eq!(
            sever_events.events[0].choices[0]
                .delta
                .content
                .as_ref()
                .unwrap(),
            ""
        );
        assert_eq!(
            sever_events.events[1].choices[0]
                .delta
                .content
                .as_ref()
                .unwrap(),
            "Hello"
        );
        assert_eq!(
            sever_events.events[2].choices[0]
                .delta
                .content
                .as_ref()
                .unwrap(),
            "!"
        );
        assert_eq!(
            sever_events.events[3].choices[0]
                .delta
                .content
                .as_ref()
                .unwrap(),
            " How"
        );
        assert_eq!(
            sever_events.events[4].choices[0]
                .delta
                .content
                .as_ref()
                .unwrap(),
            " can"
        );
        assert_eq!(sever_events.to_string(), "Hello! How can");
    }

    #[test]
    fn stream_chunk_parse_done() {
        const CHUNK_RESPONSE: &str = r#"data: {"id":"chatcmpl-ALn2KTfmrIpYd9N3Un4Kyg08WIIP6","object":"chat.completion.chunk","created":1729756748,"model":"gpt-3.5-turbo-0125","system_fingerprint":null,"choices":[{"index":0,"delta":{"content":" I"},"logprobs":null,"finish_reason":null}]}

data: {"id":"chatcmpl-ALn2KTfmrIpYd9N3Un4Kyg08WIIP6","object":"chat.completion.chunk","created":1729756748,"model":"gpt-3.5-turbo-0125","system_fingerprint":null,"choices":[{"index":0,"delta":{"content":" assist"},"logprobs":null,"finish_reason":null}]}

data: {"id":"chatcmpl-ALn2KTfmrIpYd9N3Un4Kyg08WIIP6","object":"chat.completion.chunk","created":1729756748,"model":"gpt-3.5-turbo-0125","system_fingerprint":null,"choices":[{"index":0,"delta":{"content":" you"},"logprobs":null,"finish_reason":null}]}

data: {"id":"chatcmpl-ALn2KTfmrIpYd9N3Un4Kyg08WIIP6","object":"chat.completion.chunk","created":1729756748,"model":"gpt-3.5-turbo-0125","system_fingerprint":null,"choices":[{"index":0,"delta":{"content":" today"},"logprobs":null,"finish_reason":null}]}

data: {"id":"chatcmpl-ALn2KTfmrIpYd9N3Un4Kyg08WIIP6","object":"chat.completion.chunk","created":1729756748,"model":"gpt-3.5-turbo-0125","system_fingerprint":null,"choices":[{"index":0,"delta":{"content":"?"},"logprobs":null,"finish_reason":null}]}

data: {"id":"chatcmpl-ALn2KTfmrIpYd9N3Un4Kyg08WIIP6","object":"chat.completion.chunk","created":1729756748,"model":"gpt-3.5-turbo-0125","system_fingerprint":null,"choices":[{"index":0,"delta":{},"logprobs":null,"finish_reason":"stop"}]}

data: [DONE]
"#;

        let sever_events: ChatCompletionStreamResponseServerEvents =
            ChatCompletionStreamResponseServerEvents::try_from(CHUNK_RESPONSE).unwrap();
        assert_eq!(sever_events.events.len(), 6);
        assert_eq!(
            sever_events.events[0].choices[0]
                .delta
                .content
                .as_ref()
                .unwrap(),
            " I"
        );
        assert_eq!(
            sever_events.events[1].choices[0]
                .delta
                .content
                .as_ref()
                .unwrap(),
            " assist"
        );
        assert_eq!(
            sever_events.events[2].choices[0]
                .delta
                .content
                .as_ref()
                .unwrap(),
            " you"
        );
        assert_eq!(
            sever_events.events[3].choices[0]
                .delta
                .content
                .as_ref()
                .unwrap(),
            " today"
        );
        assert_eq!(
            sever_events.events[4].choices[0]
                .delta
                .content
                .as_ref()
                .unwrap(),
            "?"
        );
        assert_eq!(sever_events.events[5].choices[0].delta.content, None);

        assert_eq!(sever_events.to_string(), " I assist you today?");
    }

    #[test]
    fn stream_chunk_parse_mistral() {
        const CHUNK_RESPONSE: &str = r#"data: {"id":"e1ebce16de5443b79613512c2d757936","object":"chat.completion.chunk","created":1729805261,"model":"ministral-8b-latest","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}

data: {"id":"e1ebce16de5443b79613512c2d757936","object":"chat.completion.chunk","created":1729805261,"model":"ministral-8b-latest","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}

data: {"id":"e1ebce16de5443b79613512c2d757936","object":"chat.completion.chunk","created":1729805261,"model":"ministral-8b-latest","choices":[{"index":0,"delta":{"content":"!"},"finish_reason":null}]}

data: {"id":"e1ebce16de5443b79613512c2d757936","object":"chat.completion.chunk","created":1729805261,"model":"ministral-8b-latest","choices":[{"index":0,"delta":{"content":" How"},"finish_reason":null}]}

data: {"id":"e1ebce16de5443b79613512c2d757936","object":"chat.completion.chunk","created":1729805261,"model":"ministral-8b-latest","choices":[{"index":0,"delta":{"content":" can"},"finish_reason":null}]}

data: {"id":"e1ebce16de5443b79613512c2d757936","object":"chat.completion.chunk","created":1729805261,"model":"ministral-8b-latest","choices":[{"index":0,"delta":{"content":" I"},"finish_reason":null}]}

data: {"id":"e1ebce16de5443b79613512c2d757936","object":"chat.completion.chunk","created":1729805261,"model":"ministral-8b-latest","choices":[{"index":0,"delta":{"content":" assist"},"finish_reason":null}]}

data: {"id":"e1ebce16de5443b79613512c2d757936","object":"chat.completion.chunk","created":1729805261,"model":"ministral-8b-latest","choices":[{"index":0,"delta":{"content":" you"},"finish_reason":null}]}

data: {"id":"e1ebce16de5443b79613512c2d757936","object":"chat.completion.chunk","created":1729805261,"model":"ministral-8b-latest","choices":[{"index":0,"delta":{"content":" today"},"finish_reason":null}]}

data: {"id":"e1ebce16de5443b79613512c2d757936","object":"chat.completion.chunk","created":1729805261,"model":"ministral-8b-latest","choices":[{"index":0,"delta":{"content":"?"},"finish_reason":null}]}

data: {"id":"e1ebce16de5443b79613512c2d757936","object":"chat.completion.chunk","created":1729805261,"model":"ministral-8b-latest","choices":[{"index":0,"delta":{"content":""},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"total_tokens":13,"completion_tokens":9}}

data: [DONE]
"#;

        let sever_events: ChatCompletionStreamResponseServerEvents =
            ChatCompletionStreamResponseServerEvents::try_from(CHUNK_RESPONSE).unwrap();
        assert_eq!(sever_events.events.len(), 11);

        assert_eq!(
            sever_events.to_string(),
            "Hello! How can I assist you today?"
        );
    }

    #[test]
    fn test_chat_completions_request() {
        const CHAT_COMPLETIONS_REQUEST: &str = r#"
{
  "model": "gpt-3.5-turbo",
  "messages": [
    {
      "role": "user",
      "content": "What city do you want to know the weather for?"
    }
  ]
}"#;

        let chat_completions_request: ChatCompletionsRequest =
            serde_json::from_str(CHAT_COMPLETIONS_REQUEST).unwrap();
        assert_eq!(chat_completions_request.model, "gpt-3.5-turbo");
        assert_eq!(
            chat_completions_request.messages[0].content,
            Some(ContentType::Text(
                "What city do you want to know the weather for?".to_string()
            ))
        );
    }

    #[test]
    fn test_chat_completions_request_text_type() {
        const CHAT_COMPLETIONS_REQUEST: &str = r#"
{
  "model": "gpt-3.5-turbo",
  "messages": [
    {
      "role": "user",
      "content": [
        {
          "type": "text",
          "text": "What city do you want to know the weather for?"
        }
      ]
    }
  ]
}
"#;

        let chat_completions_request: ChatCompletionsRequest =
            serde_json::from_str(CHAT_COMPLETIONS_REQUEST).unwrap();
        assert_eq!(chat_completions_request.model, "gpt-3.5-turbo");
        if let Some(ContentType::MultiPart(multi_part_content)) =
            chat_completions_request.messages[0].content.as_ref()
        {
            assert_eq!(
                multi_part_content[0].content_type,
                MultiPartContentType::Text
            );
            assert_eq!(
                multi_part_content[0].text,
                Some("What city do you want to know the weather for?".to_string())
            );
        } else {
            panic!("Expected MultiPartContent");
        }
    }

    #[test]
    fn test_chat_completions_request_text_type_array() {
        const CHAT_COMPLETIONS_REQUEST: &str = r#"
{
  "model": "gpt-3.5-turbo",
  "messages": [
    {
      "role": "user",
      "content": [
        {
          "type": "text",
          "text": "What city do you want to know the weather for?"
        },
        {
          "type": "text",
          "text": "hello world"
        }
      ]
    }
  ]
}
"#;

        let chat_completions_request: ChatCompletionsRequest =
            serde_json::from_str(CHAT_COMPLETIONS_REQUEST).unwrap();
        assert_eq!(chat_completions_request.model, "gpt-3.5-turbo");
        if let Some(ContentType::MultiPart(multi_part_content)) =
            chat_completions_request.messages[0].content.as_ref()
        {
            assert_eq!(multi_part_content.len(), 2);
            assert_eq!(
                multi_part_content[0].content_type,
                MultiPartContentType::Text
            );
            assert_eq!(
                multi_part_content[0].text,
                Some("What city do you want to know the weather for?".to_string())
            );
            assert_eq!(
                multi_part_content[1].content_type,
                MultiPartContentType::Text
            );
            assert_eq!(multi_part_content[1].text, Some("hello world".to_string()));
        } else {
            panic!("Expected MultiPartContent");
        }
    }

    #[test]
    fn stream_chunk_parse_claude() {
        const CHUNK_RESPONSE: &str = r#"data: {"id":"msg_01DZDMxYSgq8aPQxMQoBv6Kb","choices":[{"index":0,"delta":{"role":"assistant"}}],"created":1747685264,"model":"claude-3-7-sonnet-latest","object":"chat.completion.chunk"}

data: {"type": "ping"}

data: {"id":"msg_01DZDMxYSgq8aPQxMQoBv6Kb","choices":[{"index":0,"delta":{"content":"Hello!"}}],"created":1747685264,"model":"claude-3-7-sonnet-latest","object":"chat.completion.chunk"}

data: {"id":"msg_01DZDMxYSgq8aPQxMQoBv6Kb","choices":[{"index":0,"delta":{"content":" How can I assist you today? Whether"}}],"created":1747685264,"model":"claude-3-7-sonnet-latest","object":"chat.completion.chunk"}

data: {"id":"msg_01DZDMxYSgq8aPQxMQoBv6Kb","choices":[{"index":0,"delta":{"content":" you have a question, need information"}}],"created":1747685264,"model":"claude-3-7-sonnet-latest","object":"chat.completion.chunk"}

data: {"id":"msg_01DZDMxYSgq8aPQxMQoBv6Kb","choices":[{"index":0,"delta":{"content":", or just want to chat about"}}],"created":1747685264,"model":"claude-3-7-sonnet-latest","object":"chat.completion.chunk"}

data: {"id":"msg_01DZDMxYSgq8aPQxMQoBv6Kb","choices":[{"index":0,"delta":{"content":" something, I'm here to help. What woul"}}],"created":1747685264,"model":"claude-3-7-sonnet-latest","object":"chat.completion.chunk"}

data: {"id":"msg_01DZDMxYSgq8aPQxMQoBv6Kb","choices":[{"index":0,"delta":{"content":"d you like to talk about?"}}],"created":1747685264,"model":"claude-3-7-sonnet-latest","object":"chat.completion.chunk"}

data: {"id":"msg_01DZDMxYSgq8aPQxMQoBv6Kb","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"created":1747685264,"model":"claude-3-7-sonnet-latest","object":"chat.completion.chunk"}

data: [DONE]
"#;

        let sever_events: ChatCompletionStreamResponseServerEvents =
            ChatCompletionStreamResponseServerEvents::try_from(CHUNK_RESPONSE).unwrap();
        assert_eq!(sever_events.events.len(), 8);

        assert_eq!(
            sever_events.to_string(),
            "Hello! How can I assist you today? Whether you have a question, need information, or just want to chat about something, I'm here to help. What would you like to talk about?"
        );
    }
}
