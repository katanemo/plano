use bytes::Bytes;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use hermesllm::apis::openai::{
    ChatCompletionsRequest, ChatCompletionsResponse, Choice, FinishReason, FunctionCall, Message,
    MessageContent, ResponseMessage, Role, Tool, ToolCall, Usage,
};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use thiserror::Error;
use tracing::{error, info};

// ============================================================================
// CONSTANTS FOR HALLUCINATION DETECTION
// ============================================================================

const FUNC_NAME_START_PATTERN: &[&str] = &[r#"{"name":""#, r#"{'name':'"#];
const FUNC_NAME_END_TOKEN: &[&str] = &["\",", "',"];
const END_TOOL_CALL_TOKEN: &str = "}}";

const FIRST_PARAM_NAME_START_PATTERN: &[&str] = &[r#""arguments":{"#, r#"'arguments':{'"#];
const PARAMETER_NAME_END_TOKENS: &[&str] = &["\":", ":\"", "':", ":'", "\":\"", "':'"];
const PARAMETER_NAME_START_PATTERN: &[&str] = &["\",\"", "','"];
const PARAMETER_VALUE_START_PATTERN: &[&str] = &["\":", "':"];
const PARAMETER_VALUE_END_TOKEN: &[&str] = &["\",", "\"}"];
const ARCH_FUNCTION_MODEL_NAME: &str = "Arch-Function";

/// Default hallucination detection thresholds
#[derive(Debug, Clone)]
pub struct HallucinationThresholds {
    pub entropy: f64,
    pub varentropy: f64,
    pub probability: f64,
}

impl Default for HallucinationThresholds {
    fn default() -> Self {
        Self {
            entropy: 0.0001,
            varentropy: 0.0001,
            probability: 0.8,
        }
    }
}

// ============================================================================
// ERROR TYPES
// ============================================================================

#[derive(Debug, Error)]
pub enum FunctionCallingError {
    #[error("Failed to parse JSON: {0}")]
    JsonParseError(#[from] serde_json::Error),

    #[error("Failed to fix malformed JSON: {0}")]
    JsonFixError(String),

    #[error("Invalid model response: {0}")]
    InvalidModelResponse(String),

    #[error("Tool call verification failed: {0}")]
    ToolCallVerificationError(String),

    #[error("Data type conversion error: {0}")]
    DataTypeConversionError(String),

    #[error("Unsupported data type: {0}")]
    UnsupportedDataType(String),

    #[error("HTTP request error: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("Invalid tool call: {0}")]
    InvalidToolCall(String),
}

pub type Result<T> = std::result::Result<T, FunctionCallingError>;

// ============================================================================
// CONFIGURATION STRUCTURES
// ============================================================================

/// Configuration for Arch Function Calling
#[derive(Debug, Clone)]
pub struct ArchFunctionConfig {
    pub task_prompt: String,
    pub format_prompt: String,
    pub generation_params: GenerationParams,
    pub support_data_types: Vec<String>,
}

impl Default for ArchFunctionConfig {
    fn default() -> Self {
        Self {
            // Raw string so that \n sequences remain literal in the final prompt
            task_prompt: r#"You are a helpful assistant designed to assist with the user query by making one or more function calls if needed.\n\nYou are provided with function signatures within <tools></tools> XML tags:\n<tools>\n{tools}\n</tools>\n\nYour task is to decide which functions are needed and collect missing parameters if necessary."#.to_string(),
            // Use raw string to preserve literal \n sequences instead of real newlines
            format_prompt: r#"\n\nBased on your analysis, provide your response in one of the following JSON formats:\n1. If no functions are needed:\n```json\n{\"response\": \"Your response text here\"}\n```\n2. If functions are needed but some required parameters are missing:\n```json\n{\"required_functions\": [\"func_name1\", \"func_name2\", ...], \"clarification\": \"Text asking for missing parameters\"}\n```\n3. If functions are needed and all required parameters are available:\n```json\n{\"tool_calls\": [{\"name\": \"func_name1\", \"arguments\": {\"argument1\": \"value1\", \"argument2\": \"value2\"}},... (more tool calls as required)]}\n```"#.to_string(),
            generation_params: GenerationParams::default(),
            support_data_types: vec![
                "int".to_string(),
                "float".to_string(),
                "bool".to_string(),
                "str".to_string(),
                "list".to_string(),
                "tuple".to_string(),
                "set".to_string(),
                "dict".to_string(),
                // JSON Schema names (standard)
                "integer".to_string(),
                "number".to_string(),
                "boolean".to_string(),
                "string".to_string(),
                "array".to_string(),
                "object".to_string(),
            ],
        }
    }
}

/// Configuration for Arch Agent (extends ArchFunctionConfig with different generation params)
#[derive(Debug, Clone)]
pub struct ArchAgentConfig {
    pub task_prompt: String,
    pub format_prompt: String,
    pub generation_params: GenerationParams,
    pub support_data_types: Vec<String>,
}

impl Default for ArchAgentConfig {
    fn default() -> Self {
        let base = ArchFunctionConfig::default();
        Self {
            task_prompt: base.task_prompt,
            format_prompt: base.format_prompt,
            generation_params: GenerationParams {
                temperature: 0.01,
                top_p: 1.0,
                top_k: 10,
                max_tokens: 1024,
                stop_token_ids: vec![151645],
                logprobs: Some(true),
                top_logprobs: Some(10),
            },
            support_data_types: base.support_data_types,
        }
    }
}

/// Generation parameters for LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationParams {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: u32,
    pub max_tokens: u32,
    pub stop_token_ids: Vec<u32>,
    pub logprobs: Option<bool>,
    pub top_logprobs: Option<u32>,
}

impl Default for GenerationParams {
    fn default() -> Self {
        Self {
            temperature: 0.1,
            top_p: 1.0,
            top_k: 10,
            max_tokens: 1024,
            stop_token_ids: vec![151645],
            logprobs: Some(true),
            top_logprobs: Some(10),
        }
    }
}

// ============================================================================
// PARSED MODEL RESPONSE
// ============================================================================

/// Parsed response from the model
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ParsedModelResponse {
    pub raw_response: String,
    pub response: Option<String>,
    pub required_functions: Vec<String>,
    pub clarification: String,
    pub tool_calls: Vec<ToolCall>,
    pub is_valid: bool,
    pub error_message: String,
}

// ============================================================================
// TOOL CALL VERIFICATION RESULT
// ============================================================================

/// Result of tool call verification
#[derive(Debug, Clone)]
pub struct ToolCallVerification {
    pub is_valid: bool,
    pub invalid_tool_call: Option<ToolCall>,
    pub error_message: String,
}

impl Default for ToolCallVerification {
    fn default() -> Self {
        Self {
            is_valid: true,
            invalid_tool_call: None,
            error_message: String::new(),
        }
    }
}

/// Main handler for Arch Function Calling
pub struct ArchFunctionHandler {
    pub model_name: String,
    pub config: ArchFunctionConfig,
    pub default_prefix: String,
    pub clarify_prefix: String,
    pub endpoint_url: String,
    pub http_client: reqwest::Client,
}

impl ArchFunctionHandler {
    /// Creates a new ArchFunctionHandler
    pub fn new(model_name: String, config: ArchFunctionConfig, endpoint_url: String) -> Self {
        use common::consts::ARCH_PROVIDER_HINT_HEADER;
        use reqwest::header;

        // Create custom HTTP client with Arch provider hint header
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::HeaderName::from_static(ARCH_PROVIDER_HINT_HEADER),
            header::HeaderValue::from_str(&model_name).unwrap(),
        );

        let http_client = reqwest::ClientBuilder::new()
            .default_headers(headers)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            model_name,
            config,
            default_prefix: r#"```json\n{\""#.to_string(),
            clarify_prefix: r#"```json\n{\"required_functions\":"#.to_string(),
            endpoint_url,
            http_client,
        }
    }

    /// Converts a list of tools into JSON format string
    pub fn convert_tools(&self, tools: &[Tool]) -> Result<String> {
        let converted: std::result::Result<Vec<String>, serde_json::Error> = tools
            .iter()
            .map(|tool| serde_json::to_string(&tool.function))
            .collect();

        converted
            .map(|v| v.join("\\n"))
            .map_err(FunctionCallingError::from)
    }

    /// Fixes malformed JSON strings by ensuring proper bracket matching
    pub fn fix_json_string(&self, json_str: &str) -> Result<String> {
        let json_str = json_str.trim();
        let mut stack: Vec<char> = Vec::new();
        let mut fixed_str = String::new();

        let matching_bracket: HashMap<char, char> = [(')', '('), ('}', '{'), (']', '[')]
            .iter()
            .cloned()
            .collect();

        let opening_bracket: HashMap<char, char> =
            matching_bracket.iter().map(|(k, v)| (*v, *k)).collect();

        for ch in json_str.chars() {
            if ch == '{' || ch == '[' || ch == '(' {
                stack.push(ch);
                fixed_str.push(ch);
            } else if ch == '}' || ch == ']' || ch == ')' {
                if let Some(&last) = stack.last() {
                    if matching_bracket.get(&ch) == Some(&last) {
                        stack.pop();
                        fixed_str.push(ch);
                    }
                    // Ignore unmatched closing brackets
                }
            } else {
                fixed_str.push(ch);
            }
        }

        // Add corresponding closing brackets for unmatched opening brackets
        while let Some(unmatched_opening) = stack.pop() {
            if let Some(&closing) = opening_bracket.get(&unmatched_opening) {
                fixed_str.push(closing);
            }
        }

        // Try to parse the fixed JSON
        match serde_json::from_str::<Value>(&fixed_str) {
            Ok(val) => serde_json::to_string(&val).map_err(FunctionCallingError::from),
            Err(_) => {
                // Try replacing single quotes with double quotes
                let fixed_str = fixed_str.replace('\'', "\"");
                match serde_json::from_str::<Value>(&fixed_str) {
                    Ok(val) => serde_json::to_string(&val).map_err(FunctionCallingError::from),
                    Err(e) => Err(FunctionCallingError::JsonFixError(format!(
                        "Failed to fix JSON: {}",
                        e
                    ))),
                }
            }
        }
    }

    /// Parses the model response and extracts tool call information
    pub fn parse_model_response(&self, content: &str) -> ParsedModelResponse {
        let mut response_dict = ParsedModelResponse::default();

        // Remove markdown code blocks
        let mut content = content.trim().to_string();
        if content.starts_with("```") && content.ends_with("```") {
            content = content
                .trim_start_matches("```")
                .trim_end_matches("```")
                .to_string();
            if content.starts_with("json") {
                content = content.trim_start_matches("json").to_string();
            }
            // Trim again after removing code blocks to eliminate internal whitespace
            content = content
                .trim_start_matches(r"\n")
                .trim_end_matches(r"\n")
                .to_string();
            content = content.trim().to_string();
            // Unescape the quotes: \" -> "
            // The model sometimes returns escaped JSON inside markdown blocks
            content = content.replace(r#"\""#, "\"");
        }

        // Try to fix JSON if needed
        let fixed_content = match self.fix_json_string(&content) {
            Ok(fixed) => {
                response_dict.raw_response = format!("```json\n{}\n```", fixed);
                fixed
            }
            Err(e) => {
                response_dict.is_valid = false;
                response_dict.error_message = format!("Failed to fix JSON: {}", e);
                return response_dict;
            }
        };
        // Parse the JSON
        match serde_json::from_str::<Value>(&fixed_content) {
            Ok(model_response) => {
                // Successfully parsed - mark as valid
                response_dict.is_valid = true;

                // Extract response field
                if let Some(resp) = model_response.get("response") {
                    if let Some(resp_str) = resp.as_str() {
                        response_dict.response = Some(resp_str.to_string());
                    }
                }

                // Extract required_functions
                if let Some(funcs) = model_response.get("required_functions") {
                    if let Some(funcs_arr) = funcs.as_array() {
                        response_dict.required_functions = funcs_arr
                            .iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect();
                    }
                }

                // Extract clarification
                if let Some(clarif) = model_response.get("clarification") {
                    if let Some(clarif_str) = clarif.as_str() {
                        response_dict.clarification = clarif_str.to_string();
                    }
                }

                // Extract tool_calls
                if let Some(tool_calls) = model_response.get("tool_calls") {
                    if let Some(tool_calls_arr) = tool_calls.as_array() {
                        for tool_call_val in tool_calls_arr {
                            let id = format!("call_{}", rand::random::<u32>() % 10000 + 1000);

                            let name = tool_call_val
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();

                            let arguments = tool_call_val
                                .get("arguments")
                                .map(|v| serde_json::to_string(v).unwrap_or_default())
                                .unwrap_or_default();

                            response_dict.tool_calls.push(ToolCall {
                                id,
                                call_type: "function".to_string(),
                                function: FunctionCall { name, arguments },
                            });
                        }
                    }
                }
            }
            Err(e) => {
                response_dict.is_valid = false;
                response_dict.error_message = format!("Failed to parse model response: {}", e);
            }
        }

        response_dict
    }

    /// Converts data type from one type to another
    pub fn convert_data_type(&self, value: &Value, target_type: &str) -> Result<Value> {
        match target_type {
            // Handle float/number conversions
            "float" | "number" => {
                if let Some(int_val) = value.as_i64() {
                    return Ok(json!(int_val as f64));
                }
            }
            // Handle list/array conversions
            "list" | "array" => {
                if let Some(str_val) = value.as_str() {
                    // Try to parse as JSON array
                    if let Ok(arr) = serde_json::from_str::<Vec<Value>>(str_val) {
                        return Ok(json!(arr));
                    }
                }
            }
            // Handle str/string conversions
            "str" | "string" => {
                if !value.is_string() {
                    return Ok(json!(value.to_string()));
                }
            }
            _ => {}
        }
        Ok(value.clone())
    }

    /// Helper method to check if a value matches the expected type
    fn check_value_type(&self, value: &Value, target_type: &str) -> bool {
        match target_type {
            "int" | "integer" => value.is_i64() || value.is_u64(),
            "float" | "number" => value.is_f64() || value.is_i64() || value.is_u64(),
            "bool" | "boolean" => value.is_boolean(),
            "str" | "string" => value.is_string(),
            "list" | "array" => value.is_array(),
            "dict" | "object" => value.is_object(),
            _ => true,
        }
    }

    /// Helper method to validate and potentially convert a parameter value to match the target type
    /// Returns Ok(true) if the value is valid (either originally or after conversion)
    /// Returns Ok(false) if the value cannot be converted to the target type
    fn validate_or_convert_parameter(
        &self,
        param_value: &Value,
        target_type: &str,
    ) -> Result<bool> {
        // First check: Is it already the correct type?
        if self.check_value_type(param_value, target_type) {
            return Ok(true);
        }

        // Try to convert
        let converted = self.convert_data_type(param_value, target_type)?;

        // Second check: Is it the correct type after conversion?
        Ok(self.check_value_type(&converted, target_type))
    }

    /// Verifies the validity of extracted tool calls against the provided tools
    pub fn verify_tool_calls(
        &self,
        tools: &[Tool],
        tool_calls: &[ToolCall],
    ) -> ToolCallVerification {
        let mut verification = ToolCallVerification::default();

        // Build a map of function name to parameters
        let mut functions: HashMap<String, &Value> = HashMap::new();
        for tool in tools {
            functions.insert(tool.function.name.clone(), &tool.function.parameters);
        }

        for tool_call in tool_calls {
            if !verification.is_valid {
                break;
            }

            let func_name = &tool_call.function.name;

            // Parse arguments as JSON
            let func_args: HashMap<String, Value> =
                match serde_json::from_str(&tool_call.function.arguments) {
                    Ok(args) => args,
                    Err(e) => {
                        verification.is_valid = false;
                        verification.invalid_tool_call = Some(tool_call.clone());
                        verification.error_message = format!(
                            "Failed to parse arguments for function '{}': {}",
                            func_name, e
                        );
                        break;
                    }
                };

            // Check if function is available
            if let Some(function_params) = functions.get(func_name) {
                // Check if all required parameters are present
                if let Some(required) = function_params.get("required") {
                    if let Some(required_arr) = required.as_array() {
                        for required_param in required_arr {
                            if let Some(param_name) = required_param.as_str() {
                                if !func_args.contains_key(param_name) {
                                    verification.is_valid = false;
                                    verification.invalid_tool_call = Some(tool_call.clone());
                                    verification.error_message = format!(
                                        "`{}` is required by the function `{}` but not found in the tool call!",
                                        param_name, func_name
                                    );
                                    break;
                                }
                            }
                        }
                    }
                }

                // Verify the data type of each parameter
                if let Some(properties) = function_params.get("properties") {
                    if let Some(properties_obj) = properties.as_object() {
                        for (param_name, param_value) in &func_args {
                            if let Some(param_schema) = properties_obj.get(param_name) {
                                if let Some(target_type) =
                                    param_schema.get("type").and_then(|v| v.as_str())
                                {
                                    if self
                                        .config
                                        .support_data_types
                                        .contains(&target_type.to_string())
                                    {
                                        // Validate data type using helper method
                                        match self
                                            .validate_or_convert_parameter(param_value, target_type)
                                        {
                                            Ok(is_valid) => {
                                                if !is_valid {
                                                    verification.is_valid = false;
                                                    verification.invalid_tool_call =
                                                        Some(tool_call.clone());
                                                    verification.error_message = format!(
                                                        "Parameter `{}` is expected to have the data type `{}`, got incompatible type.",
                                                        param_name, target_type
                                                    );
                                                    break;
                                                }
                                            }
                                            Err(_) => {
                                                verification.is_valid = false;
                                                verification.invalid_tool_call =
                                                    Some(tool_call.clone());
                                                verification.error_message = format!(
                                                    "Parameter `{}` is expected to have the data type `{}`, got incompatible type.",
                                                    param_name, target_type
                                                );
                                                break;
                                            }
                                        }
                                    } else {
                                        verification.is_valid = false;
                                        verification.invalid_tool_call = Some(tool_call.clone());
                                        verification.error_message = format!(
                                            "Data type `{}` is not supported.",
                                            target_type
                                        );
                                        break;
                                    }
                                }
                            } else {
                                verification.is_valid = false;
                                verification.invalid_tool_call = Some(tool_call.clone());
                                verification.error_message = format!(
                                    "Parameter `{}` is not defined in the function `{}`.",
                                    param_name, func_name
                                );
                                break;
                            }
                        }
                    }
                }
            } else {
                verification.is_valid = false;
                verification.invalid_tool_call = Some(tool_call.clone());
                verification.error_message = format!("{} is not available!", func_name);
            }
        }

        verification
    }

    /// Formats the system prompt with tools
    pub fn format_system_prompt(&self, tools: &[Tool]) -> Result<String> {
        let tools_str = self.convert_tools(tools)?;
        let system_prompt =
            self.config.task_prompt.replace("{tools}", &tools_str) + &self.config.format_prompt;

        Ok(system_prompt)
    }

    /// Processes messages and formats them appropriately for the model
    pub fn process_messages(
        &self,
        messages: &[Message],
        tools: Option<&[Tool]>,
        extra_instruction: Option<&str>,
        max_tokens: usize,
        metadata: Option<&HashMap<String, Value>>,
    ) -> Result<Vec<Message>> {
        let mut processed_messages = Vec::new();

        // Add system message with tools if provided
        if let Some(tools) = tools {
            let system_prompt = self.format_system_prompt(tools)?;
            processed_messages.push(Message {
                role: Role::System,
                content: Some(MessageContent::Text(system_prompt)),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
        }

        // Process each message
        for (idx, message) in messages.iter().enumerate() {
            let mut role = message.role.clone();
            let mut content = match &message.content {
                Some(MessageContent::Text(text)) => text.clone(),
                Some(MessageContent::Parts(_)) => String::new(),
                None => String::new(),
            };

            // Handle tool calls
            if let Some(tool_calls) = &message.tool_calls {
                if !tool_calls.is_empty() {
                    role = Role::Assistant;
                    let tool_call_json = serde_json::to_string(&tool_calls[0].function)?;
                    content = format!("<tool_call>\n{}\n</tool_call>", tool_call_json);
                }
            } else if role == Role::Tool {
                role = Role::User;

                // Check if we should optimize context window
                let optimize_context = metadata
                    .and_then(|m| m.get("optimize_context_window"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_lowercase() == "true")
                    .unwrap_or(false);

                if optimize_context {
                    content = "<tool_response>\n\n</tool_response>".to_string();
                } else {
                    // Get the tool call from previous message
                    if idx > 0 {
                        if let Some(MessageContent::Text(prev_content)) = &messages[idx - 1].content
                        {
                            let mut tool_call_msg = prev_content.clone();

                            // Strip markdown code blocks
                            if tool_call_msg.starts_with("```") && tool_call_msg.ends_with("```") {
                                tool_call_msg = tool_call_msg
                                    .trim_start_matches("```")
                                    .trim_end_matches("```")
                                    .trim()
                                    .to_string();
                                if tool_call_msg.starts_with("json") {
                                    tool_call_msg =
                                        tool_call_msg.trim_start_matches("json").trim().to_string();
                                }
                            }

                            // Extract function name
                            if let Ok(parsed) = serde_json::from_str::<Value>(&tool_call_msg) {
                                if let Some(tool_calls_arr) =
                                    parsed.get("tool_calls").and_then(|v| v.as_array())
                                {
                                    if let Some(first_tool_call) = tool_calls_arr.first() {
                                        let func_name = first_tool_call
                                            .get("name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("no_name");

                                        let tool_response = json!({
                                            "name": func_name,
                                            "result": content,
                                        });

                                        content = format!(
                                            "<tool_response>\n{}\n</tool_response>",
                                            serde_json::to_string(&tool_response)?
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }

            processed_messages.push(Message {
                role,
                content: Some(MessageContent::Text(content)),
                name: message.name.clone(),
                tool_calls: None,
                tool_call_id: None,
            });
        }

        // Ensure last message is from user
        if let Some(last) = processed_messages.last() {
            if last.role != Role::User {
                return Err(FunctionCallingError::InvalidModelResponse(
                    "Last message must be from user".to_string(),
                ));
            }
        }

        // Add extra instruction if provided
        if let Some(instruction) = extra_instruction {
            if let Some(last) = processed_messages.last_mut() {
                if let Some(MessageContent::Text(content)) = &mut last.content {
                    content.push('\n');
                    content.push_str(instruction);
                }
            }
        }

        // Truncate messages if they exceed max_tokens
        let processed_messages = self.truncate_messages(processed_messages, max_tokens);

        Ok(processed_messages)
    }

    /// Truncates messages to fit within max_tokens limit
    fn truncate_messages(&self, messages: Vec<Message>, max_tokens: usize) -> Vec<Message> {
        let mut num_tokens = 0;
        let mut conversation_idx = 0;

        // Keep system message if present
        if let Some(first) = messages.first() {
            if first.role == Role::System {
                if let Some(MessageContent::Text(content)) = &first.content {
                    num_tokens += content.len() / 4; // Approximate 4 chars per token
                }
                conversation_idx = 1;
            }
        }

        // Calculate from the end backwards
        // Start with message_idx pointing past the end (will be used if no truncation needed)
        let mut message_idx = messages.len();
        for i in (conversation_idx..messages.len()).rev() {
            if let Some(MessageContent::Text(content)) = &messages[i].content {
                num_tokens += content.len() / 4;
                if num_tokens >= max_tokens && messages[i].role == Role::User {
                    // Set message_idx to current position and break
                    // This matches Python's behavior where message_idx is set before break
                    message_idx = i;
                    break;
                }
            }
            // Only update message_idx if we haven't hit the token limit yet
            // This ensures message_idx points to where truncation should start
            if num_tokens < max_tokens {
                message_idx = i;
            }
        }

        // Return system message + truncated conversation
        let mut result = Vec::new();
        if conversation_idx > 0 {
            result.push(messages[0].clone());
        }
        result.extend_from_slice(&messages[message_idx..]);

        result
    }

    /// Prefills a message by adding an assistant message with the prefix
    pub fn prefill_message(&self, mut messages: Vec<Message>, prefill: &str) -> Vec<Message> {
        messages.push(Message {
            role: Role::Assistant,
            content: Some(MessageContent::Text(prefill.to_string())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        });
        messages
    }

    /// Helper to create a request with VLLM-specific parameters
    fn create_request_with_extra_body(
        &self,
        messages: Vec<Message>,
        stream: bool,
    ) -> ChatCompletionsRequest {
        ChatCompletionsRequest {
            model: self.model_name.clone(),
            messages,
            temperature: Some(self.config.generation_params.temperature),
            top_p: Some(self.config.generation_params.top_p),
            max_tokens: Some(self.config.generation_params.max_tokens),
            stream: Some(stream),
            logprobs: self.config.generation_params.logprobs,
            top_logprobs: self.config.generation_params.top_logprobs,
            // VLLM-specific parameters
            continue_final_message: Some(true),
            add_generation_prompt: Some(false),
            top_k: Some(self.config.generation_params.top_k),
            stop_token_ids: if !self.config.generation_params.stop_token_ids.is_empty() {
                Some(self.config.generation_params.stop_token_ids.clone())
            } else {
                None
            },
            ..Default::default()
        }
    }

    /// Makes a streaming request and returns the SSE event stream
    async fn make_streaming_request(
        &self,
        request: ChatCompletionsRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn futures::Stream<Item = std::result::Result<Value, String>> + Send>>,
    > {
        let request_body = serde_json::to_string(&request).map_err(|e| {
            FunctionCallingError::InvalidModelResponse(format!(
                "Failed to serialize request: {}",
                e
            ))
        })?;

        let response = self
            .http_client
            .post(&self.endpoint_url)
            .header("Content-Type", "application/json")
            .body(request_body)
            .send()
            .await
            .map_err(FunctionCallingError::HttpError)?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(FunctionCallingError::InvalidModelResponse(format!(
                "HTTP error {}: {}",
                status, error_text
            )));
        }

        // Parse SSE stream
        let stream = response.bytes_stream().eventsource();
        let parsed_stream = stream.filter_map(|event_result| async move {
            match event_result {
                Ok(event) => {
                    // Skip [DONE] sentinel
                    if event.data == "[DONE]" {
                        return None;
                    }
                    // Parse JSON
                    match serde_json::from_str::<Value>(&event.data) {
                        Ok(json) => Some(Ok(json)),
                        Err(e) => Some(Err(format!("JSON parse error: {}", e))),
                    }
                }
                Err(e) => Some(Err(format!("SSE stream error: {}", e))),
            }
        });

        Ok(Box::pin(parsed_stream))
    }

    /// Makes a non-streaming request and returns the response
    async fn make_non_streaming_request(
        &self,
        request: ChatCompletionsRequest,
    ) -> Result<ChatCompletionsResponse> {
        let request_body = serde_json::to_string(&request).map_err(|e| {
            FunctionCallingError::InvalidModelResponse(format!(
                "Failed to serialize request: {}",
                e
            ))
        })?;

        let response = self
            .http_client
            .post(&self.endpoint_url)
            .header("Content-Type", "application/json")
            .body(request_body)
            .send()
            .await
            .map_err(FunctionCallingError::HttpError)?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(FunctionCallingError::InvalidModelResponse(format!(
                "HTTP error {}: {}",
                status, error_text
            )));
        }

        let response_text = response
            .text()
            .await
            .map_err(FunctionCallingError::HttpError)?;

        serde_json::from_str(&response_text).map_err(FunctionCallingError::JsonParseError)
    }

    pub async fn function_calling_chat(
        &self,
        request: ChatCompletionsRequest,
    ) -> Result<ChatCompletionsResponse> {
        use tracing::{error, info};

        info!("[Arch-Function] - ChatCompletion");

        let messages = self.process_messages(
            &request.messages,
            request.tools.as_deref(),
            None,
            self.config.generation_params.max_tokens as usize,
            request.metadata.as_ref(),
        )?;

        info!(
            "[request to arch-fc]: model: {}, messages count: {}",
            self.model_name,
            messages.len()
        );

        let use_agent_orchestrator = request
            .metadata
            .as_ref()
            .and_then(|m| m.get("use_agent_orchestrator"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let prefilled_messages = self.prefill_message(messages.clone(), &self.default_prefix);

        // Create request with extra_body parameters
        let stream_request = self.create_request_with_extra_body(prefilled_messages.clone(), true);
        let mut stream = self.make_streaming_request(stream_request).await?;

        let mut model_response = String::new();

        if use_agent_orchestrator {
            while let Some(chunk_result) = stream.next().await {
                let chunk = chunk_result.map_err(FunctionCallingError::InvalidModelResponse)?;
                // Extract content from JSON response
                if let Some(choices) = chunk.get("choices").and_then(|v| v.as_array()) {
                    if let Some(choice) = choices.first() {
                        if let Some(content) = choice
                            .get("delta")
                            .and_then(|d| d.get("content"))
                            .and_then(|c| c.as_str())
                        {
                            model_response.push_str(content);
                        }
                    }
                }
            }
            info!("[Agent Orchestrator]: response received");
        } else if let Some(tools) = request.tools.as_ref() {
            let mut hallucination_state = HallucinationState::new(tools);
            let mut has_tool_calls = None;
            let mut has_hallucination = false;

            while let Some(chunk_result) = stream.next().await {
                let chunk = chunk_result.map_err(FunctionCallingError::InvalidModelResponse)?;

                // Extract content and logprobs from JSON response
                if let Some(choices) = chunk.get("choices").and_then(|v| v.as_array()) {
                    if let Some(choice) = choices.first() {
                        if let Some(content) = choice
                            .get("delta")
                            .and_then(|d| d.get("content"))
                            .and_then(|c| c.as_str())
                        {
                            // Extract logprobs
                            let logprobs: Vec<f64> = choice
                                .get("logprobs")
                                .and_then(|lp| lp.get("content"))
                                .and_then(|c| c.as_array())
                                .and_then(|arr| arr.first())
                                .and_then(|token| token.get("top_logprobs"))
                                .and_then(|tlp| tlp.as_array())
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(|v| v.get("logprob").and_then(|lp| lp.as_f64()))
                                        .collect()
                                })
                                .unwrap_or_default();

                            if hallucination_state
                                .append_and_check_token_hallucination(content.to_string(), logprobs)
                            {
                                has_hallucination = true;
                                break;
                            }

                            if hallucination_state.tokens.len() > 5 && has_tool_calls.is_none() {
                                let collected_content = hallucination_state.tokens.join("");
                                has_tool_calls = Some(collected_content.contains("tool_calls"));
                            }
                        }
                    }
                }
            }

            if has_tool_calls == Some(true) && has_hallucination {
                info!("[Hallucination]: {}", hallucination_state.error_message);

                let clarify_messages = self.prefill_message(messages.clone(), &self.clarify_prefix);
                let clarify_request = self.create_request_with_extra_body(clarify_messages, false);

                let retry_response = self.make_non_streaming_request(clarify_request).await?;

                if let Some(choice) = retry_response.choices.first() {
                    if let Some(content) = &choice.message.content {
                        model_response = content.clone();
                    }
                }
            } else {
                model_response = hallucination_state.tokens.join("");
            }
        } else {
            while let Some(chunk_result) = stream.next().await {
                let chunk = chunk_result.map_err(FunctionCallingError::InvalidModelResponse)?;
                if let Some(choices) = chunk.get("choices").and_then(|v| v.as_array()) {
                    if let Some(choice) = choices.first() {
                        if let Some(content) = choice
                            .get("delta")
                            .and_then(|d| d.get("content"))
                            .and_then(|c| c.as_str())
                        {
                            model_response.push_str(content);
                        }
                    }
                }
            }
        }

        let response_dict = self.parse_model_response(&model_response);

        info!(
            "[arch-fc]: raw model response: {}",
            response_dict.raw_response
        );

        // General model response (no intent matched - should route to default target)
        let model_message = if response_dict
            .response
            .as_ref()
            .is_some_and(|s| !s.is_empty())
        {
            // When arch-fc returns a "response" field, it means no intent was matched
            // Return empty content and empty tool_calls so prompt_gateway routes to default target
            ResponseMessage {
                role: Role::Assistant,
                content: Some(String::new()),
                refusal: None,
                annotations: None,
                audio: None,
                function_call: None,
                tool_calls: None,
            }
        } else if !response_dict.required_functions.is_empty() {
            if !use_agent_orchestrator {
                ResponseMessage {
                    role: Role::Assistant,
                    content: Some(response_dict.clarification.clone()),
                    refusal: None,
                    annotations: None,
                    audio: None,
                    function_call: None,
                    tool_calls: None,
                }
            } else {
                ResponseMessage {
                    role: Role::Assistant,
                    content: Some(String::new()),
                    refusal: None,
                    annotations: None,
                    audio: None,
                    function_call: None,
                    tool_calls: None,
                }
            }
        } else if !response_dict.tool_calls.is_empty() {
            if response_dict.is_valid {
                if !use_agent_orchestrator {
                    if let Some(tools) = request.tools.as_ref() {
                        let verification = self.verify_tool_calls(tools, &response_dict.tool_calls);

                        if verification.is_valid {
                            info!(
                                "[Tool calls]: {:?}",
                                response_dict
                                    .tool_calls
                                    .iter()
                                    .map(|tc| &tc.function)
                                    .collect::<Vec<_>>()
                            );
                            ResponseMessage {
                                role: Role::Assistant,
                                content: Some(String::new()),
                                refusal: None,
                                annotations: None,
                                audio: None,
                                function_call: None,
                                tool_calls: Some(response_dict.tool_calls.clone()),
                            }
                        } else {
                            error!("Invalid tool call - {}", verification.error_message);
                            ResponseMessage {
                                role: Role::Assistant,
                                content: Some(String::new()),
                                refusal: None,
                                annotations: None,
                                audio: None,
                                function_call: None,
                                tool_calls: None,
                            }
                        }
                    } else {
                        error!("Tool calls present but no tools provided in request");
                        ResponseMessage {
                            role: Role::Assistant,
                            content: Some(String::new()),
                            refusal: None,
                            annotations: None,
                            audio: None,
                            function_call: None,
                            tool_calls: None,
                        }
                    }
                } else {
                    info!(
                        "[Tool calls]: {:?}",
                        response_dict
                            .tool_calls
                            .iter()
                            .map(|tc| &tc.function)
                            .collect::<Vec<_>>()
                    );
                    ResponseMessage {
                        role: Role::Assistant,
                        content: Some(String::new()),
                        refusal: None,
                        annotations: None,
                        audio: None,
                        function_call: None,
                        tool_calls: Some(response_dict.tool_calls.clone()),
                    }
                }
            } else {
                error!(
                    "Invalid tool calls in response: {}",
                    response_dict.error_message
                );
                ResponseMessage {
                    role: Role::Assistant,
                    content: Some(String::new()),
                    refusal: None,
                    annotations: None,
                    audio: None,
                    function_call: None,
                    tool_calls: None,
                }
            }
        } else {
            error!("Invalid model response - {}", model_response);
            ResponseMessage {
                role: Role::Assistant,
                content: Some(String::new()),
                refusal: None,
                annotations: None,
                audio: None,
                function_call: None,
                tool_calls: None,
            }
        };

        // Create metadata with the raw model response
        let mut metadata = HashMap::new();
        metadata.insert(
            "x-arch-fc-model-response".to_string(),
            serde_json::to_value(&response_dict.raw_response)
                .unwrap_or_else(|_| Value::String(response_dict.raw_response.clone())),
        );

        let chat_completion_response = ChatCompletionsResponse {
            id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
            object: Some("chat.completion".to_string()),
            created: chrono::Utc::now().timestamp() as u64,
            model: request.model.clone(),
            choices: vec![Choice {
                index: 0,
                message: model_message,
                finish_reason: Some(FinishReason::Stop),
                logprobs: None,
            }],
            usage: Usage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            system_fingerprint: None,
            service_tier: None,
            metadata: Some(metadata),
        };

        info!("[response arch-fc]: {:?}", chat_completion_response);

        Ok(chat_completion_response)
    }
}

// ============================================================================
// ARCH AGENT HANDLER
// ============================================================================

/// Handler for Arch Agent (extends ArchFunctionHandler with specialized behavior)
pub struct ArchAgentHandler {
    pub function_handler: ArchFunctionHandler,
}

impl ArchAgentHandler {
    /// Creates a new ArchAgentHandler
    pub fn new(model_name: String, endpoint_url: String) -> Self {
        let config = ArchAgentConfig::default();
        Self {
            function_handler: ArchFunctionHandler::new(
                model_name,
                ArchFunctionConfig {
                    task_prompt: config.task_prompt,
                    format_prompt: config.format_prompt,
                    generation_params: GenerationParams {
                        temperature: config.generation_params.temperature,
                        top_p: config.generation_params.top_p,
                        top_k: config.generation_params.top_k,
                        max_tokens: config.generation_params.max_tokens,
                        stop_token_ids: config.generation_params.stop_token_ids,
                        logprobs: config.generation_params.logprobs,
                        top_logprobs: config.generation_params.top_logprobs,
                    },
                    support_data_types: config.support_data_types,
                },
                endpoint_url,
            ),
        }
    }

    /// Converts tools with special handling for empty parameters
    /// This is the key difference from ArchFunctionHandler
    pub fn convert_tools(&self, tools: &[Tool]) -> Result<String> {
        let mut converted = Vec::new();

        for tool in tools {
            let mut tool_copy = tool.clone();

            // Delete parameters key if its empty
            if let Some(props) = tool_copy.function.parameters.get("properties") {
                if props.is_object() && props.as_object().unwrap().is_empty() {
                    // Create new parameters without properties
                    if let Some(params_obj) = tool_copy.function.parameters.as_object_mut() {
                        params_obj.remove("properties");
                    }
                }
            }

            converted.push(serde_json::to_string(&tool_copy.function)?);
        }

        Ok(converted.join("\n"))
    }
}

// ============================================================================
// HTTP HANDLER FOR FUNCTION CALLING ENDPOINT
// ============================================================================

fn full<T: Into<Bytes>>(chunk: T) -> BoxBody<Bytes, hyper::Error> {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}

pub async fn function_calling_chat_handler(
    req: Request<Incoming>,
    llm_provider_url: String,
) -> std::result::Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    use hermesllm::apis::openai::ChatCompletionsRequest;
    let whole_body = req.collect().await?.to_bytes();

    // Parse as JSON Value first to modify it
    let mut body_json: Value = match serde_json::from_slice(&whole_body) {
        Ok(json) => json,
        Err(e) => {
            error!("Failed to parse request body as JSON: {}", e);
            let mut response = Response::new(full(
                serde_json::json!({
                    "error": format!("Invalid request body: {}", e)
                })
                .to_string(),
            ));
            *response.status_mut() = StatusCode::BAD_REQUEST;
            response
                .headers_mut()
                .insert("Content-Type", "application/json".parse().unwrap());
            return Ok(response);
        }
    };

    // Add "model": "Arch-Function" to the request
    if let Some(obj) = body_json.as_object_mut() {
        obj.insert("model".to_string(), ARCH_FUNCTION_MODEL_NAME.into());
    }

    // Parse as ChatCompletionsRequest
    let chat_request: ChatCompletionsRequest = match serde_json::from_value(body_json) {
        Ok(req) => {
            info!(
                "[request body]: {}",
                serde_json::to_string(&req).unwrap_or_default()
            );
            req
        }
        Err(e) => {
            error!("Failed to parse request body: {}", e);
            let mut response = Response::new(full(
                serde_json::json!({
                    "error": format!("Invalid request body: {}", e)
                })
                .to_string(),
            ));
            *response.status_mut() = StatusCode::BAD_REQUEST;
            response
                .headers_mut()
                .insert("Content-Type", "application/json".parse().unwrap());
            return Ok(response);
        }
    };

    // Determine which handler to use based on metadata
    let use_agent_orchestrator = chat_request
        .metadata
        .as_ref()
        .and_then(|m| m.get("use_agent_orchestrator"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    info!("Use agent orchestrator: {}", use_agent_orchestrator);

    // Create the appropriate handler
    let handler_name = if use_agent_orchestrator {
        "Arch-Agent"
    } else {
        "Arch-Function"
    };

    // Call the handler
    let final_response = if use_agent_orchestrator {
        let handler = ArchAgentHandler::new(
            ARCH_FUNCTION_MODEL_NAME.to_string(),
            llm_provider_url.clone(),
        );
        handler
            .function_handler
            .function_calling_chat(chat_request)
            .await
    } else {
        let handler = ArchFunctionHandler::new(
            ARCH_FUNCTION_MODEL_NAME.to_string(),
            ArchFunctionConfig::default(),
            llm_provider_url.clone(),
        );
        handler.function_calling_chat(chat_request).await
    };

    match final_response {
        Ok(response_data) => {
            let response_json = serde_json::to_string(&response_data).unwrap_or_else(|e| {
                error!("Failed to serialize response: {}", e);
                serde_json::json!({"error": "Failed to serialize response"}).to_string()
            });

            let mut response = Response::new(full(response_json));
            *response.status_mut() = StatusCode::OK;
            response
                .headers_mut()
                .insert("Content-Type", "application/json".parse().unwrap());

            Ok(response)
        }
        Err(e) => {
            error!("[{}] - Error in function calling: {}", handler_name, e);

            let error_response = serde_json::json!({
                "error": format!("[{}] - Error in function calling: {}", handler_name, e)
            });

            let mut response = Response::new(full(error_response.to_string()));
            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            response
                .headers_mut()
                .insert("Content-Type", "application/json".parse().unwrap());
            Ok(response)
        }
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arch_function_config_default() {
        let config = ArchFunctionConfig::default();
        assert!(config.task_prompt.contains("helpful assistant"));
        assert!(config.format_prompt.contains("JSON formats"));
        assert_eq!(config.generation_params.temperature, 0.1);
        assert_eq!(config.support_data_types.len(), 14); // 8 Python-style + 6 JSON Schema names

        // Verify prompt formatting for literal escaped newlines ("\\n") instead of actual newline chars
        // The user requirement changed prompts to display "\\n" sequences literally.
        assert!(config.task_prompt.contains("\\n\\nYou are provided"));
        assert!(config.task_prompt.contains("</tools>\\n\\n"));

        // Format prompt should contain literal escaped newlines and proper JSON examples
        assert!(config
            .format_prompt
            .contains("\\n\\nBased on your analysis"));
        assert!(config
            .format_prompt
            .contains(r#"{\"response\": \"Your response text here\"}"#));
        assert!(config.format_prompt.contains(r#"{\"tool_calls\": [{"#));
    }

    #[test]
    fn test_arch_agent_config_default() {
        let config = ArchAgentConfig::default();
        assert_eq!(config.generation_params.temperature, 0.01); // Different from ArchFunctionConfig
    }

    #[test]
    fn test_fix_json_string_valid() {
        let handler = ArchFunctionHandler::new(
            "test-model".to_string(),
            ArchFunctionConfig::default(),
            "http://localhost:8000".to_string(),
        );
        let json_str = r#"{"name": "test", "value": 123}"#;
        let result = handler.fix_json_string(json_str);
        assert!(result.is_ok());
    }

    #[test]
    fn test_fix_json_string_missing_bracket() {
        let handler = ArchFunctionHandler::new(
            "test-model".to_string(),
            ArchFunctionConfig::default(),
            "http://localhost:8000".to_string(),
        );
        let json_str = r#"{"name": "test", "value": 123"#;
        let result = handler.fix_json_string(json_str);
        assert!(result.is_ok());
        let fixed = result.unwrap();
        assert!(fixed.contains("}"));
    }

    #[test]
    fn test_parse_model_response_with_tool_calls() {
        let handler = ArchFunctionHandler::new(
            "test-model".to_string(),
            ArchFunctionConfig::default(),
            "http://localhost:8000".to_string(),
        );
        let content =
            r#"{"tool_calls": [{"name": "get_weather", "arguments": {"location": "NYC"}}]}"#;
        let result = handler.parse_model_response(content);

        assert!(result.is_valid);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].function.name, "get_weather");
    }

    #[test]
    fn test_parse_model_response_with_clarification() {
        let handler = ArchFunctionHandler::new(
            "test-model".to_string(),
            ArchFunctionConfig::default(),
            "http://localhost:8000".to_string(),
        );
        let content =
            r#"{"required_functions": ["get_weather"], "clarification": "What location?"}"#;
        let result = handler.parse_model_response(content);

        assert!(result.is_valid);
        assert_eq!(result.required_functions.len(), 1);
        assert_eq!(result.clarification, "What location?");
    }

    #[test]
    fn test_convert_data_type_int_to_float() {
        let handler = ArchFunctionHandler::new(
            "test-model".to_string(),
            ArchFunctionConfig::default(),
            "http://localhost:8000".to_string(),
        );
        let value = json!(42);
        let result = handler.convert_data_type(&value, "float");
        assert!(result.is_ok());
        assert!(result.unwrap().is_f64());
    }
}

// ============================================================================
// HALLUCINATION DETECTION MODULE
// ============================================================================

/// Mask token types for tracking parsing state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskToken {
    FunctionName,
    ParameterValue,
    ParameterName,
    NotUsed,
    ToolCall,
}

/// Uncertainty metrics calculated from log probabilities
#[derive(Debug, Clone)]
pub struct UncertaintyMetrics {
    pub entropy: f64,
    pub varentropy: f64,
    pub probability: f64,
}

/// Calculates uncertainty metrics from log probabilities
///
/// This is a simplified Rust implementation that avoids torch/tensor dependencies.
/// Uses basic statistical calculations instead of tensor operations.
pub fn calculate_uncertainty(log_probs: &[f64]) -> UncertaintyMetrics {
    if log_probs.is_empty() {
        return UncertaintyMetrics {
            entropy: 0.0,
            varentropy: 0.0,
            probability: 0.0,
        };
    }

    // Convert log probabilities to probabilities
    let token_probs: Vec<f64> = log_probs.iter().map(|&lp| lp.exp()).collect();

    // Calculate entropy: -sum(p * log(p)) / log(2)
    let mut entropy = 0.0;
    for i in 0..log_probs.len() {
        entropy -= log_probs[i] * token_probs[i];
    }
    entropy /= 2_f64.ln(); // Convert to bits

    // Calculate variance of entropy
    let mut varentropy = 0.0;
    for i in 0..log_probs.len() {
        let diff = log_probs[i] / 2_f64.ln() + entropy;
        varentropy += token_probs[i] * diff * diff;
    }

    // Get the top probability
    let probability = token_probs.first().copied().unwrap_or(0.0);

    UncertaintyMetrics {
        entropy,
        varentropy,
        probability,
    }
}

/// Checks if uncertainty metrics exceed thresholds
pub fn check_threshold(
    entropy: f64,
    varentropy: f64,
    thresholds: &HallucinationThresholds,
) -> bool {
    entropy > thresholds.entropy && varentropy > thresholds.varentropy
}

/// Checks if a parameter is required in the function description
pub fn is_parameter_required(function_description: &Value, parameter_name: &str) -> bool {
    if let Some(required) = function_description.get("required") {
        if let Some(required_arr) = required.as_array() {
            return required_arr
                .iter()
                .any(|v| v.as_str() == Some(parameter_name));
        }
    }
    false
}

/// Checks if a parameter has a specific property
pub fn is_parameter_property(
    function_description: &Value,
    parameter_name: &str,
    property_name: &str,
) -> bool {
    if let Some(properties) = function_description.get("properties") {
        if let Some(param_info) = properties.get(parameter_name) {
            return param_info.get(property_name).is_some();
        }
    }
    false
}

/// State for hallucination detection during streaming
///
/// This is a simplified version of the Python HallucinationState that doesn't
/// require torch/tensor dependencies. It provides the core functionality needed
/// for detecting hallucinations during function calling.
#[derive(Debug)]
pub struct HallucinationState {
    pub tokens: Vec<String>,
    pub logprobs: Vec<Vec<f64>>,
    pub state: Option<String>,
    pub mask: Vec<MaskToken>,
    pub parameter_name_done: bool,
    pub hallucination: bool,
    pub error_message: String,
    pub parameter_name: Vec<String>,
    pub token_probs_map: Vec<(String, f64, f64, f64)>,
    pub function_properties: HashMap<String, Value>,
    pub open_bracket: bool,
    pub bracket: Option<char>,
    pub function_name: String,
    pub check_parameter_name: HashMap<String, bool>,
    pub thresholds: HallucinationThresholds,
}

impl HallucinationState {
    /// Creates a new HallucinationState with function definitions
    pub fn new(functions: &[Tool]) -> Self {
        let function_properties: HashMap<String, Value> = functions
            .iter()
            .map(|tool| (tool.function.name.clone(), tool.function.parameters.clone()))
            .collect();

        Self {
            tokens: Vec::new(),
            logprobs: Vec::new(),
            state: None,
            mask: Vec::new(),
            parameter_name_done: false,
            hallucination: false,
            error_message: String::new(),
            parameter_name: Vec::new(),
            token_probs_map: Vec::new(),
            function_properties,
            open_bracket: false,
            bracket: None,
            function_name: String::new(),
            check_parameter_name: HashMap::new(),
            thresholds: HallucinationThresholds::default(),
        }
    }

    /// Appends a token and checks for hallucination
    pub fn append_and_check_token_hallucination(
        &mut self,
        token: String,
        logprob: Vec<f64>,
    ) -> bool {
        self.tokens.push(token);
        self.logprobs.push(logprob);
        self.process_token();
        self.hallucination
    }

    /// Resets internal parameters
    fn reset_parameters(&mut self) {
        self.state = None;
        self.parameter_name_done = false;
        self.hallucination = false;
        self.error_message.clear();
        self.open_bracket = false;
        self.bracket = None;
        self.check_parameter_name.clear();
    }

    /// Processes the current token and updates state
    fn process_token(&mut self) {
        let content: String = self.tokens.join("").replace(' ', "");

        // Handle end of tool call
        if content.ends_with(END_TOOL_CALL_TOKEN) {
            self.reset_parameters();
        }

        // Function name extraction logic
        if self.state.as_deref() == Some("function_name") {
            if !FUNC_NAME_END_TOKEN
                .iter()
                .any(|&t| self.tokens.last().is_some_and(|tok| tok == t))
            {
                self.mask.push(MaskToken::FunctionName);
            } else {
                self.state = None;
                self.get_function_name();
            }
        }

        // Check for function name start
        if FUNC_NAME_START_PATTERN
            .iter()
            .any(|&p| content.ends_with(p))
        {
            self.state = Some("function_name".to_string());
        }

        // Parameter name extraction logic
        if self.state.as_deref() == Some("parameter_name")
            && !PARAMETER_NAME_END_TOKENS
                .iter()
                .any(|&t| content.ends_with(t))
        {
            self.mask.push(MaskToken::ParameterName);
        } else if self.state.as_deref() == Some("parameter_name")
            && PARAMETER_NAME_END_TOKENS
                .iter()
                .any(|&t| content.ends_with(t))
        {
            self.state = None;
            self.parameter_name_done = true;
            self.get_parameter_name();
        } else if self.parameter_name_done
            && !self.open_bracket
            && PARAMETER_NAME_START_PATTERN
                .iter()
                .any(|&p| content.ends_with(p))
        {
            self.state = Some("parameter_name".to_string());
        }

        // First parameter value start
        if FIRST_PARAM_NAME_START_PATTERN
            .iter()
            .any(|&p| content.ends_with(p))
        {
            self.state = Some("parameter_name".to_string());
        }

        // Parameter value extraction logic
        if self.state.as_deref() == Some("parameter_value")
            && !PARAMETER_VALUE_END_TOKEN
                .iter()
                .any(|&t| content.ends_with(t))
        {
            // Check for brackets
            if let Some(last_token) = self.tokens.last() {
                let open_brackets: Vec<char> = last_token
                    .trim()
                    .chars()
                    .filter(|&c| c == '(' || c == '{' || c == '[')
                    .collect();

                if !open_brackets.is_empty() {
                    self.open_bracket = true;
                    self.bracket = Some(open_brackets[0]);
                }

                if self.open_bracket {
                    let closing = match self.bracket {
                        Some('(') => ')',
                        Some('{') => '}',
                        Some('[') => ']',
                        _ => '\0',
                    };
                    if last_token.trim().contains(closing) {
                        self.open_bracket = false;
                        self.bracket = None;
                    }
                }

                // Check if token has actual value content
                let has_non_punct = last_token.trim().chars().any(|c| !c.is_ascii_punctuation());
                if has_non_punct && !last_token.trim().is_empty() {
                    self.mask.push(MaskToken::ParameterValue);

                    // Check hallucination for required parameters without enum
                    if self.function_properties.contains_key(&self.function_name) {
                        if self.mask.len() > 1
                            && self.mask[self.mask.len() - 2] != MaskToken::ParameterValue
                            && !self.parameter_name.is_empty()
                        {
                            let last_param =
                                self.parameter_name[self.parameter_name.len() - 1].clone();
                            if let Some(func_props) =
                                self.function_properties.get(&self.function_name)
                            {
                                if is_parameter_required(func_props, &last_param)
                                    && !is_parameter_property(func_props, &last_param, "enum")
                                    && !self.check_parameter_name.contains_key(&last_param)
                                {
                                    self.check_logprob();
                                    self.check_parameter_name.insert(last_param, true);
                                }
                            }
                        }
                    } else if !self.function_name.is_empty() {
                        self.check_logprob();
                        self.error_message = format!(
                            "Function name {} not found in function properties",
                            self.function_name
                        );
                    }
                } else {
                    self.mask.push(MaskToken::NotUsed);
                }
            }
        } else if self.state.as_deref() == Some("parameter_value")
            && !self.open_bracket
            && PARAMETER_VALUE_END_TOKEN
                .iter()
                .any(|&t| content.ends_with(t))
        {
            self.state = None;
        } else if self.parameter_name_done
            && PARAMETER_VALUE_START_PATTERN
                .iter()
                .any(|&p| content.ends_with(p))
        {
            self.state = Some("parameter_value".to_string());
        }

        // Maintain consistency between tokens and mask
        if self.mask.len() != self.tokens.len() {
            self.mask.push(MaskToken::NotUsed);
        }
    }

    /// Checks log probability and detects hallucination
    fn check_logprob(&mut self) {
        if let Some(probs) = self.logprobs.last() {
            let metrics = calculate_uncertainty(probs);

            if let Some(token) = self.tokens.last() {
                self.token_probs_map.push((
                    token.clone(),
                    metrics.entropy,
                    metrics.varentropy,
                    metrics.probability,
                ));

                if check_threshold(metrics.entropy, metrics.varentropy, &self.thresholds) {
                    self.hallucination = true;
                    self.error_message = format!(
                        "token '{}' is uncertain. Generated response:\n{}",
                        token,
                        self.tokens.join("")
                    );
                }
            }
        }
    }

    /// Counts consecutive tokens of a specific type in the mask
    fn count_consecutive_token(&self, token_type: MaskToken) -> usize {
        if self.mask.is_empty() || self.mask.last() != Some(&token_type) {
            return 0;
        }

        self.mask
            .iter()
            .rev()
            .take_while(|&&t| t == token_type)
            .count()
    }

    /// Extracts the parameter name from recent tokens
    fn get_parameter_name(&mut self) {
        let p_len = self.count_consecutive_token(MaskToken::ParameterName);
        if p_len > 0 && self.tokens.len() > 1 {
            let start_idx = self.tokens.len().saturating_sub(p_len + 1);
            let end_idx = self.tokens.len().saturating_sub(1);
            let parameter_name: String = self.tokens[start_idx..end_idx].join("");
            self.parameter_name.push(parameter_name);
        }
    }

    /// Extracts the function name from recent tokens
    fn get_function_name(&mut self) {
        let f_len = self.count_consecutive_token(MaskToken::FunctionName);
        if f_len > 0 && self.tokens.len() > 1 {
            let start_idx = self.tokens.len().saturating_sub(f_len + 1);
            let end_idx = self.tokens.len().saturating_sub(1);
            self.function_name = self.tokens[start_idx..end_idx].join("");
        }
    }
}

#[cfg(test)]
mod hallucination_tests {
    use super::*;

    #[test]
    fn test_calculate_uncertainty() {
        let log_probs = vec![-0.1, -2.0, -3.0];
        let metrics = calculate_uncertainty(&log_probs);
        assert!(metrics.entropy >= 0.0);
        assert!(metrics.varentropy >= 0.0);
        assert!(metrics.probability > 0.0 && metrics.probability <= 1.0);
    }

    #[test]
    fn test_calculate_uncertainty_empty() {
        let log_probs: Vec<f64> = vec![];
        let metrics = calculate_uncertainty(&log_probs);
        assert_eq!(metrics.entropy, 0.0);
        assert_eq!(metrics.varentropy, 0.0);
        assert_eq!(metrics.probability, 0.0);
    }

    #[test]
    fn test_check_threshold() {
        let thresholds = HallucinationThresholds::default();
        assert!(check_threshold(0.001, 0.001, &thresholds));
        assert!(!check_threshold(0.00001, 0.00001, &thresholds));
    }

    #[test]
    fn test_is_parameter_required() {
        let func_desc = json!({
            "required": ["param1", "param2"]
        });
        assert!(is_parameter_required(&func_desc, "param1"));
        assert!(!is_parameter_required(&func_desc, "param3"));
    }

    #[test]
    fn test_is_parameter_property() {
        let func_desc = json!({
            "properties": {
                "param1": {
                    "type": "string",
                    "enum": ["a", "b"]
                }
            }
        });
        assert!(is_parameter_property(&func_desc, "param1", "enum"));
        assert!(!is_parameter_property(&func_desc, "param1", "default"));
    }

    #[test]
    fn test_check_value_type() {
        let handler = ArchFunctionHandler::new(
            "test-model".to_string(),
            ArchFunctionConfig::default(),
            "http://localhost:8000".to_string(),
        );

        // Test integer types
        assert!(handler.check_value_type(&json!(42), "integer"));
        assert!(handler.check_value_type(&json!(42), "int"));
        assert!(!handler.check_value_type(&json!(3.15), "integer"));

        // Test number types (accepts both int and float)
        assert!(handler.check_value_type(&json!(3.15), "number"));
        assert!(handler.check_value_type(&json!(42), "number"));
        assert!(handler.check_value_type(&json!(3.15), "float"));

        // Test boolean
        assert!(handler.check_value_type(&json!(true), "boolean"));
        assert!(handler.check_value_type(&json!(false), "bool"));
        assert!(!handler.check_value_type(&json!("true"), "boolean"));

        // Test string
        assert!(handler.check_value_type(&json!("hello"), "string"));
        assert!(handler.check_value_type(&json!("hello"), "str"));
        assert!(!handler.check_value_type(&json!(123), "string"));

        // Test array
        assert!(handler.check_value_type(&json!([1, 2, 3]), "array"));
        assert!(handler.check_value_type(&json!([1, 2, 3]), "list"));
        assert!(!handler.check_value_type(&json!({}), "array"));

        // Test object
        assert!(handler.check_value_type(&json!({"key": "value"}), "object"));
        assert!(handler.check_value_type(&json!({"key": "value"}), "dict"));
        assert!(!handler.check_value_type(&json!([]), "object"));

        // Test unknown type (should return true)
        assert!(handler.check_value_type(&json!(42), "unknown_type"));
    }

    #[test]
    fn test_validate_or_convert_parameter() {
        let handler = ArchFunctionHandler::new(
            "test-model".to_string(),
            ArchFunctionConfig::default(),
            "http://localhost:8000".to_string(),
        );

        // Test valid type - no conversion needed
        assert!(handler
            .validate_or_convert_parameter(&json!(42), "integer")
            .unwrap());
        assert!(handler
            .validate_or_convert_parameter(&json!("hello"), "string")
            .unwrap());

        // Test integer to float conversion (convert_data_type supports this)
        let result = handler.validate_or_convert_parameter(&json!(42), "float");
        assert!(result.is_ok());
        assert!(result.unwrap()); // Should be valid after conversion

        // Test invalid type that cannot be converted
        // A string cannot be converted to integer (convert_data_type doesn't support this)
        let result = handler.validate_or_convert_parameter(&json!("abc"), "integer");
        // Since convert_data_type returns Ok(value.clone()) for unsupported conversions,
        // the validation will fail because "abc" string is not an integer
        assert!(!result.unwrap());

        // Test number accepting both int and float
        assert!(handler
            .validate_or_convert_parameter(&json!(42), "number")
            .unwrap());
        assert!(handler
            .validate_or_convert_parameter(&json!(3.15), "number")
            .unwrap());
    }

    #[test]
    fn test_hallucination_state_new() {
        let tools = vec![Tool {
            tool_type: "function".to_string(),
            function: hermesllm::apis::openai::Function {
                name: "test_func".to_string(),
                description: Some("Test function".to_string()),
                parameters: json!({"type": "object"}),
                strict: None,
            },
        }];

        let state = HallucinationState::new(&tools);
        assert_eq!(state.tokens.len(), 0);
        assert!(!state.hallucination);
        assert!(state.function_properties.contains_key("test_func"));
    }
}
