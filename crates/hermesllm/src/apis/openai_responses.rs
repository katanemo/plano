use crate::providers::request::{ProviderRequest, ProviderRequestError};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::collections::HashMap;

impl TryFrom<&[u8]> for ResponsesAPIRequest {
    type Error = serde_json::Error;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        serde_json::from_slice(bytes)
    }
}

/// Parameterized conversion for ResponsesAPIResponse
impl TryFrom<&[u8]> for ResponsesAPIResponse {
    type Error = crate::apis::openai::OpenAIStreamError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        serde_json::from_slice(bytes).map_err(crate::apis::openai::OpenAIStreamError::from)
    }
}

// ============================================================================
// Request Structs - CreateResponse
// ============================================================================

/// Request to create a model response
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesAPIRequest {
    /// The model to use for generating the response
    pub model: String,

    /// Text, image, or file inputs to the model
    pub input: InputParam,

    /// Specify additional output data to include in the model response
    pub include: Option<Vec<IncludeEnum>>,

    /// Whether to allow the model to run tool calls in parallel
    pub parallel_tool_calls: Option<bool>,

    /// Whether to store the generated model response for later retrieval via API
    pub store: Option<bool>,

    /// A system (or developer) message inserted into the model's context
    pub instructions: Option<String>,

    /// If set to true, the model response data will be streamed to the client
    pub stream: Option<bool>,

    /// Stream options configuration
    pub stream_options: Option<ResponseStreamOptions>,

    /// Conversation state
    pub conversation: Option<ConversationParam>,

    /// Tools available to the model
    pub tools: Option<Vec<Tool>>,

    /// Tool choice option
    pub tool_choice: Option<ToolChoice>,

    /// Maximum number of output tokens
    pub max_output_tokens: Option<i32>,

    /// Temperature for sampling (0-2)
    pub temperature: Option<f32>,

    /// Top-p nucleus sampling parameter
    pub top_p: Option<f32>,

    /// Metadata for the response
    pub metadata: Option<HashMap<String, serde_json::Value>>,

    /// Previous response ID for conversation continuation
    pub previous_response_id: Option<String>,

    /// Response modalities
    pub modalities: Option<Vec<Modality>>,

    /// Audio output configuration
    pub audio: Option<AudioConfig>,

    /// Text output format configuration
    pub text: Option<TextConfig>,

    /// Reasoning effort level
    pub reasoning_effort: Option<ReasoningEffort>,

    /// Truncation strategy
    pub truncation: Option<String>,

    /// User identifier
    pub user: Option<String>,

    /// Maximum number of tool calls
    pub max_tool_calls: Option<i32>,

    /// Service tier
    pub service_tier: Option<String>,

    /// Whether to run in background
    pub background: Option<bool>,

    /// Number of top logprobs to include
    pub top_logprobs: Option<i32>,
}

/// Input parameter - can be a simple string or array of input items
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InputParam {
    /// Simple text input
    Text(String),
    /// Array of input items (messages, references, outputs, etc.)
    Items(Vec<InputItem>),
}

/// Input item - can be a message, item reference, function call output, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InputItem {
    /// Input message (role + content)
    Message(InputMessage),
    /// Item reference
    ItemReference {
        #[serde(rename = "type")]
        item_type: String,
        id: String,
    },
    /// Function call output
    FunctionCallOutput {
        #[serde(rename = "type")]
        item_type: String,
        call_id: String,
        output: String,
    },
}

/// Input message with role and content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputMessage {
    /// Message role
    pub role: MessageRole,
    /// Message content - can be a string or array of InputContent
    pub content: MessageContent,
}

/// Message content - can be either a simple string or array of content items
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// Simple text content
    Text(String),
    /// Array of content items
    Items(Vec<InputContent>),
}

/// Message roles
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Developer,
}

/// Input content types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputContent {
    /// Text input
    InputText { text: String },
    /// Image input via URL
    InputImage {
        image_url: String,
        detail: Option<String>,
    },
    /// File input via URL
    InputFile { file_url: String },
    /// Audio input
    InputAudio {
        data: Option<String>,
        format: Option<String>,
    },
}

/// Modality options
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Modality {
    Text,
    Audio,
}

/// Audio configuration
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    /// Voice to use for audio output
    pub voice: String,
    /// Audio output format
    pub format: Option<String>,
}

/// Text configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextConfig {
    /// Text format configuration
    pub format: TextFormat,
}

/// Text format
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TextFormat {
    Text,
    JsonObject,
    JsonSchema { json_schema: serde_json::Value },
}

/// Reasoning effort levels
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
}

/// Include enum for additional output data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IncludeEnum {
    #[serde(rename = "web_search_call.action.sources")]
    WebSearchCallActionSources,
    #[serde(rename = "code_interpreter_call.outputs")]
    CodeInterpreterCallOutputs,
    #[serde(rename = "computer_call_output.output.image_url")]
    ComputerCallOutputImageUrl,
    #[serde(rename = "file_search_call.results")]
    FileSearchCallResults,
    #[serde(rename = "message.input_image.image_url")]
    MessageInputImageImageUrl,
    #[serde(rename = "message.output_text.logprobs")]
    MessageOutputTextLogprobs,
    #[serde(rename = "reasoning.encrypted_content")]
    ReasoningEncryptedContent,
}

/// Response stream options
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseStreamOptions {
    /// Whether to include usage in stream
    pub include_usage: Option<bool>,
}

/// Conversation parameter
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationParam {
    /// Conversation ID
    pub id: Option<String>,
}

/// Tool definitions
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Tool {
    /// Function tool - flat structure in Responses API
    Function {
        name: String,
        description: Option<String>,
        parameters: Option<serde_json::Value>,
        strict: Option<bool>,
    },
    /// File search tool
    FileSearch {
        vector_store_ids: Option<Vec<String>>,
        max_num_results: Option<i32>,
        ranking_options: Option<RankingOptions>,
        filters: Option<serde_json::Value>,
    },
    /// Web search tool
    WebSearchPreview {
        domains: Option<Vec<String>>,
        search_context_size: Option<String>,
        user_location: Option<UserLocation>,
    },
    /// Code interpreter tool
    CodeInterpreter,
    /// Computer tool
    Computer {
        display_width_px: Option<i32>,
        display_height_px: Option<i32>,
        display_number: Option<i32>,
    },
}

/// Ranking options for file search
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingOptions {
    /// Ranker type
    pub ranker: String,
    /// Score threshold
    pub score_threshold: Option<f32>,
}

/// User location for web search
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserLocation {
    #[serde(rename = "type")]
    pub location_type: String,
    pub city: Option<String>,
    pub country: Option<String>,
    pub region: Option<String>,
    pub timezone: Option<String>,
}

/// Tool choice options
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    /// Auto, none, or required
    String(String),
    /// Named tool choice
    Named {
        #[serde(rename = "type")]
        tool_type: String,
        function: NamedFunction,
    },
}

/// Named function for tool choice
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedFunction {
    pub name: String,
}

// ============================================================================
// Response Structs - Response Object
// ============================================================================

/// The response object returned from the API
/// Request to create a model response
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesAPIResponse {
    /// Unique identifier for this Response
    pub id: String,

    /// The object type - always "response"
    pub object: String,

    /// Unix timestamp (in seconds) of when this Response was created
    pub created_at: i64,

    /// The status of the response generation
    pub status: ResponseStatus,

    /// Error information if the response failed
    pub error: Option<ResponseError>,

    /// Details about why the response is incomplete
    pub incomplete_details: Option<IncompleteDetails>,

    /// System/developer instructions
    pub instructions: Option<String>,

    /// The model used
    pub model: String,

    /// An array of content items generated by the model
    pub output: Vec<OutputItem>,

    /// Usage statistics
    pub usage: Option<ResponseUsage>,

    /// Whether to allow parallel tool calls
    pub parallel_tool_calls: bool,

    /// Conversation state
    pub conversation: Option<Conversation>,

    /// Previous response ID
    pub previous_response_id: Option<String>,

    /// Tools available
    pub tools: Vec<Tool>,

    /// Tool choice setting
    pub tool_choice: String,

    /// Temperature setting
    pub temperature: f32,

    /// Top-p setting
    pub top_p: f32,

    /// Metadata
    pub metadata: HashMap<String, serde_json::Value>,

    /// Truncation setting
    pub truncation: Option<String>,

    /// Maximum output tokens
    pub max_output_tokens: Option<i32>,

    /// Reasoning configuration
    pub reasoning: Option<Reasoning>,

    /// Whether response is stored
    pub store: Option<bool>,

    /// Text configuration
    pub text: Option<TextConfig>,

    /// Audio configuration
    pub audio: Option<AudioConfig>,

    /// Modalities
    pub modalities: Option<Vec<Modality>>,

    /// Service tier
    pub service_tier: Option<String>,

    /// Background execution
    pub background: Option<bool>,

    /// Top logprobs count
    pub top_logprobs: Option<i32>,

    /// Maximum tool calls
    pub max_tool_calls: Option<i32>,
}

/// Response status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    Completed,
    Failed,
    InProgress,
    Cancelled,
    Queued,
    Incomplete,
}

/// Response error information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseError {
    /// Error code
    pub code: ResponseErrorCode,
    /// Human-readable error message
    pub message: String,
}

/// Response error codes
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseErrorCode {
    ServerError,
    RateLimitExceeded,
    InvalidPrompt,
    VectorStoreTimeout,
    InvalidImage,
    InvalidImageFormat,
    InvalidBase64Image,
    InvalidImageUrl,
    ImageTooLarge,
    ImageTooSmall,
    ImageParseError,
    ImageContentPolicyViolation,
    InvalidImageMode,
    ImageFileTooLarge,
    UnsupportedImageMediaType,
    EmptyImageFile,
    FailedToDownloadImage,
    ImageFileNotFound,
}

/// Incomplete details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncompleteDetails {
    /// The reason why the response is incomplete
    pub reason: IncompleteReason,
}

/// Incomplete reasons
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncompleteReason {
    MaxOutputTokens,
    ContentFilter,
}

/// Output items from the model
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputItem {
    /// Output message
    Message {
        id: String,
        status: OutputItemStatus,
        role: String,
        content: Vec<OutputContent>,
    },
    /// Function tool call
    FunctionCall {
        id: String,
        status: OutputItemStatus,
        call_id: String,
        name: Option<String>,
        arguments: Option<String>,
    },
    /// Function call output
    FunctionCallOutput {
        id: String,
        call_id: String,
        output: String,
        status: Option<OutputItemStatus>,
    },
    /// File search tool call
    FileSearchCall {
        id: String,
        status: OutputItemStatus,
        queries: Option<Vec<String>>,
        results: Option<Vec<FileSearchResult>>,
    },
    /// Web search tool call
    WebSearchCall {
        id: String,
        status: OutputItemStatus,
    },
    /// Code interpreter tool call
    CodeInterpreterCall {
        id: String,
        status: OutputItemStatus,
        code: Option<String>,
        outputs: Option<Vec<CodeInterpreterOutput>>,
    },
    /// Computer tool call
    ComputerCall {
        id: String,
        status: OutputItemStatus,
        action: Option<serde_json::Value>,
    },
    /// Computer call output
    ComputerCallOutput {
        id: String,
        call_id: String,
        output: Option<serde_json::Value>,
        status: Option<OutputItemStatus>,
    },
    /// Custom tool call
    CustomToolCall {
        id: String,
        status: OutputItemStatus,
        call_id: String,
        input: Option<String>,
    },
    /// Custom tool call output
    CustomToolCallOutput {
        id: String,
        call_id: String,
        output: String,
        status: Option<OutputItemStatus>,
    },
    /// Reasoning item
    Reasoning {
        id: String,
        summary: Vec<serde_json::Value>,
    },
}

/// Output item status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputItemStatus {
    InProgress,
    Completed,
    Incomplete,
}

/// Output content types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputContent {
    /// Text output
    OutputText {
        text: String,
        annotations: Vec<Annotation>,
        logprobs: Option<Vec<LogProb>>,
    },
    /// Audio output
    OutputAudio {
        data: Option<String>,
        transcript: Option<String>,
    },
    /// Refusal output
    Refusal { refusal: String },
}

/// Annotations for output text
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Annotation {
    /// File citation
    FileCitation {
        index: i32,
        file_id: String,
        filename: String,
        quote: Option<String>,
    },
    /// URL citation
    UrlCitation {
        start_index: i32,
        end_index: i32,
        url: String,
        title: String,
    },
}

/// Log probability information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogProb {
    /// The token
    pub token: String,
    /// Log probability value
    pub logprob: f32,
    /// Token bytes
    pub bytes: Vec<u8>,
}

/// File search result
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSearchResult {
    /// File ID
    pub file_id: String,
    /// File name
    pub filename: String,
    /// Score
    pub score: Option<f32>,
    /// Content excerpt
    pub content: Option<String>,
}

/// Code interpreter output
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodeInterpreterOutput {
    /// Text output
    Text { text: String },
    /// Image output
    Image { image: String },
}

/// Response usage statistics
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseUsage {
    /// Input tokens used
    pub input_tokens: i32,
    /// Output tokens generated
    pub output_tokens: i32,
    /// Total tokens (input + output)
    pub total_tokens: i32,
    /// Input token details
    pub input_tokens_details: Option<TokenDetails>,
    /// Output token details
    pub output_tokens_details: Option<OutputTokenDetails>,
}

impl crate::providers::response::TokenUsage for ResponseUsage {
    fn completion_tokens(&self) -> usize {
        self.output_tokens as usize
    }

    fn prompt_tokens(&self) -> usize {
        self.input_tokens as usize
    }

    fn total_tokens(&self) -> usize {
        self.total_tokens as usize
    }
}

/// Token details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenDetails {
    /// Cached tokens
    pub cached_tokens: i32,
}

/// Output token details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputTokenDetails {
    /// Reasoning tokens
    pub reasoning_tokens: i32,
}

/// Reasoning configuration and summary
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reasoning {
    /// Reasoning effort level
    pub effort: Option<ReasoningEffort>,
    /// Summary of reasoning
    pub summary: Option<String>,
}

/// Conversation object
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    /// Conversation ID
    pub id: String,
    /// Conversation object type
    pub object: String,
}

// ============================================================================
// Streaming Response Events
// ============================================================================

/// Stream events for responses
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponsesAPIStreamEvent {
    /// Response created
    #[serde(rename = "response.created")]
    ResponseCreated {
        response: ResponsesAPIResponse,
        sequence_number: i32,
    },

    /// Response in progress
    #[serde(rename = "response.in_progress")]
    ResponseInProgress {
        response: ResponsesAPIResponse,
        sequence_number: i32,
    },

    /// Response completed
    #[serde(rename = "response.completed")]
    ResponseCompleted {
        response: ResponsesAPIResponse,
        sequence_number: i32,
    },

    /// Output item added
    #[serde(rename = "response.output_item.added")]
    ResponseOutputItemAdded {
        output_index: i32,
        item: OutputItem,
        sequence_number: i32,
    },

    /// Output item done
    #[serde(rename = "response.output_item.done")]
    ResponseOutputItemDone {
        output_index: i32,
        item: OutputItem,
        sequence_number: i32,
    },

    /// Content part added
    #[serde(rename = "response.content_part.added")]
    ResponseContentPartAdded {
        item_id: String,
        output_index: i32,
        content_index: i32,
        part: OutputContent,
        sequence_number: i32,
    },

    /// Content part done
    #[serde(rename = "response.content_part.done")]
    ResponseContentPartDone {
        item_id: String,
        output_index: i32,
        content_index: i32,
        part: OutputContent,
        sequence_number: i32,
    },

    /// Output text delta (incremental text streaming)
    #[serde(rename = "response.output_text.delta")]
    ResponseOutputTextDelta {
        item_id: String,
        output_index: i32,
        content_index: i32,
        delta: String,
        logprobs: Vec<serde_json::Value>,
        obfuscation: Option<String>,
        sequence_number: i32,
    },

    /// Output text done (final complete text)
    #[serde(rename = "response.output_text.done")]
    ResponseOutputTextDone {
        item_id: String,
        output_index: i32,
        content_index: i32,
        text: String,
        logprobs: Vec<serde_json::Value>,
        sequence_number: i32,
    },

    /// Audio delta
    #[serde(rename = "response.audio.delta")]
    ResponseAudioDelta {
        item_id: Option<String>,
        output_index: Option<i32>,
        content_index: Option<i32>,
        delta: String,
        sequence_number: i32,
    },

    /// Audio done
    #[serde(rename = "response.audio.done")]
    ResponseAudioDone {
        item_id: Option<String>,
        output_index: Option<i32>,
        content_index: Option<i32>,
        sequence_number: i32,
    },

    /// Audio transcript delta
    #[serde(rename = "response.audio_transcript.delta")]
    ResponseAudioTranscriptDelta {
        item_id: Option<String>,
        output_index: Option<i32>,
        content_index: Option<i32>,
        delta: String,
        sequence_number: i32,
    },

    /// Audio transcript done
    #[serde(rename = "response.audio_transcript.done")]
    ResponseAudioTranscriptDone {
        item_id: Option<String>,
        output_index: Option<i32>,
        content_index: Option<i32>,
        transcript: Option<String>,
        sequence_number: i32,
    },

    /// Function call arguments delta
    #[serde(rename = "response.function_call_arguments.delta")]
    ResponseFunctionCallArgumentsDelta {
        output_index: i32,
        item_id: String,
        delta: String,
        sequence_number: i32,
        call_id: Option<String>,
        name: Option<String>,
    },

    /// Function call arguments done
    #[serde(rename = "response.function_call_arguments.done")]
    ResponseFunctionCallArgumentsDone {
        output_index: i32,
        item_id: String,
        arguments: String,
        sequence_number: i32,
    },

    /// Code interpreter call code delta
    #[serde(rename = "response.code_interpreter_call.code.delta")]
    ResponseCodeInterpreterCallCodeDelta {
        output_index: i32,
        item_id: String,
        delta: String,
        sequence_number: i32,
    },

    /// Code interpreter call code done
    #[serde(rename = "response.code_interpreter_call.code.done")]
    ResponseCodeInterpreterCallCodeDone {
        output_index: i32,
        item_id: String,
        code: String,
        sequence_number: i32,
    },

    /// Code interpreter call in progress
    #[serde(rename = "response.code_interpreter_call.in_progress")]
    ResponseCodeInterpreterCallInProgress {
        output_index: i32,
        item_id: String,
        sequence_number: i32,
    },

    /// Code interpreter call interpreting
    #[serde(rename = "response.code_interpreter_call.interpreting")]
    ResponseCodeInterpreterCallInterpreting {
        output_index: i32,
        item_id: String,
        sequence_number: i32,
    },

    /// Code interpreter call completed
    #[serde(rename = "response.code_interpreter_call.completed")]
    ResponseCodeInterpreterCallCompleted {
        output_index: i32,
        item_id: String,
        sequence_number: i32,
    },

    /// Custom tool call input delta
    #[serde(rename = "response.custom_tool_call.input.delta")]
    ResponseCustomToolCallInputDelta {
        output_index: i32,
        item_id: String,
        delta: String,
        sequence_number: i32,
    },

    /// Custom tool call input done
    #[serde(rename = "response.custom_tool_call.input.done")]
    ResponseCustomToolCallInputDone {
        output_index: i32,
        item_id: String,
        input: String,
        sequence_number: i32,
    },

    /// Error event
    Error {
        code: String,
        message: String,
        sequence_number: i32,
    },

    /// Done event (end of stream)
    Done { sequence_number: i32 },
}

// ============================================================================
// Additional Response Operations
// ============================================================================

/// Retrieve response request (GET /responses/{response_id})
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetResponseRequest {
    /// Response ID to retrieve
    pub response_id: String,
}

/// Delete response request (DELETE /responses/{response_id})
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteResponseRequest {
    /// Response ID to delete
    pub response_id: String,
}

/// Delete response response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteResponseResponse {
    /// Response ID that was deleted
    pub id: String,
    /// Object type
    pub object: String,
    /// Whether deletion was successful
    pub deleted: bool,
}

/// Cancel response request (POST /responses/{response_id}/cancel)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelResponseRequest {
    /// Response ID to cancel
    pub response_id: String,
}

/// List input items request (GET /responses/{response_id}/input_items)
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListInputItemsRequest {
    /// Response ID
    pub response_id: String,
    /// Limit for pagination
    pub limit: Option<i32>,
    /// Order for pagination
    pub order: Option<String>,
    /// After cursor for pagination
    pub after: Option<String>,
    /// Before cursor for pagination
    pub before: Option<String>,
}

/// List input items response
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListInputItemsResponse {
    /// Object type - always "list"
    pub object: String,
    /// Array of input items
    pub data: Vec<InputItem>,
    /// First ID in the list
    pub first_id: Option<String>,
    /// Last ID in the list
    pub last_id: Option<String>,
    /// Whether there are more items
    pub has_more: bool,
}

// ============================================================================
// ProviderRequest Implementation
// ============================================================================

impl ProviderRequest for ResponsesAPIRequest {
    fn model(&self) -> &str {
        &self.model
    }

    fn set_model(&mut self, model: String) {
        self.model = model;
    }

    fn is_streaming(&self) -> bool {
        self.stream.unwrap_or_default()
    }

    fn extract_messages_text(&self) -> String {
        match &self.input {
            InputParam::Text(text) => text.clone(),
            InputParam::Items(items) => {
                items.iter().fold(String::new(), |acc, item| {
                    match item {
                        InputItem::Message(msg) => {
                            let content_text = match &msg.content {
                                MessageContent::Text(text) => text.clone(),
                                MessageContent::Items(content_items) => {
                                    content_items.iter().fold(String::new(), |acc, content| {
                                        acc + " "
                                            + &match content {
                                                InputContent::InputText { text } => text.clone(),
                                                InputContent::InputImage { .. } => {
                                                    "[Image]".to_string()
                                                }
                                                InputContent::InputFile { .. } => {
                                                    "[File]".to_string()
                                                }
                                                InputContent::InputAudio { .. } => {
                                                    "[Audio]".to_string()
                                                }
                                            }
                                    })
                                }
                            };
                            acc + " " + &content_text
                        }
                        // Skip non-message items (references, outputs, etc.)
                        _ => acc,
                    }
                })
            }
        }
    }

    fn get_recent_user_message(&self) -> Option<String> {
        match &self.input {
            InputParam::Text(text) => Some(text.clone()),
            InputParam::Items(items) => {
                items.iter().rev().find_map(|item| {
                    match item {
                        InputItem::Message(msg) if matches!(msg.role, MessageRole::User) => {
                            // Extract text from content
                            match &msg.content {
                                MessageContent::Text(text) => Some(text.clone()),
                                MessageContent::Items(content_items) => {
                                    content_items.iter().find_map(|content| match content {
                                        InputContent::InputText { text } => Some(text.clone()),
                                        _ => None,
                                    })
                                }
                            }
                        }
                        // Skip non-message items
                        _ => None,
                    }
                })
            }
        }
    }

    fn get_tool_names(&self) -> Option<Vec<String>> {
        self.tools.as_ref().map(|tools| {
            tools
                .iter()
                .filter_map(|tool| match tool {
                    Tool::Function { name, .. } => Some(name.clone()),
                    // Other tool types don't have user-defined names
                    _ => None,
                })
                .collect()
        })
    }

    fn to_bytes(&self) -> Result<Vec<u8>, ProviderRequestError> {
        serde_json::to_vec(&self).map_err(|e| ProviderRequestError {
            message: format!("Failed to serialize Responses API request: {}", e),
            source: Some(Box::new(e)),
        })
    }

    fn metadata(&self) -> &Option<HashMap<String, serde_json::Value>> {
        &self.metadata
    }

    fn remove_metadata_key(&mut self, key: &str) -> bool {
        if let Some(ref mut metadata) = self.metadata {
            metadata.remove(key).is_some()
        } else {
            false
        }
    }

    fn get_temperature(&self) -> Option<f32> {
        self.temperature
    }

    fn get_messages(&self) -> Vec<crate::apis::openai::Message> {
        use crate::transforms::request::from_openai::ResponsesInputConverter;

        // Use the shared converter to get the full conversion with image support
        let converter = ResponsesInputConverter {
            input: self.input.clone(),
            instructions: self.instructions.clone(),
        };

        // Convert and return, falling back to empty vec on error
        converter.try_into().unwrap_or_else(|_| Vec::new())
    }

    fn set_messages(&mut self, messages: &[crate::apis::openai::Message]) {
        // For ResponsesAPI, we need to convert messages back to input format
        // Extract system messages as instructions
        let system_text = messages
            .iter()
            .filter(|msg| msg.role == crate::apis::openai::Role::System)
            .filter_map(|msg| {
                if let crate::apis::openai::MessageContent::Text(text) = &msg.content {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        if !system_text.is_empty() {
            self.instructions = Some(system_text);
        }

        // Convert user/assistant messages to InputParam
        // For simplicity, we'll use the last user message as the input
        // or combine all non-system messages
        let input_messages: Vec<_> = messages
            .iter()
            .filter(|msg| msg.role != crate::apis::openai::Role::System)
            .collect();

        if !input_messages.is_empty() {
            // If there's only one message, use Text format
            if input_messages.len() == 1 {
                if let crate::apis::openai::MessageContent::Text(text) = &input_messages[0].content
                {
                    self.input = crate::apis::openai_responses::InputParam::Text(text.clone());
                }
            } else {
                // Multiple messages - combine them as text for now
                // A more sophisticated approach would use InputParam::Items
                let combined_text = input_messages
                    .iter()
                    .filter_map(|msg| {
                        if let crate::apis::openai::MessageContent::Text(text) = &msg.content {
                            Some(format!(
                                "{}: {}",
                                match msg.role {
                                    crate::apis::openai::Role::User => "User",
                                    crate::apis::openai::Role::Assistant => "Assistant",
                                    _ => "Unknown",
                                },
                                text
                            ))
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                self.input = crate::apis::openai_responses::InputParam::Text(combined_text);
            }
        }
    }
}

// ============================================================================
// Into<String> Implementation for SSE Formatting
// ============================================================================

impl From<ResponsesAPIStreamEvent> for String {
    fn from(val: ResponsesAPIStreamEvent) -> Self {
        let transformed_json = serde_json::to_string(&val).unwrap_or_default();
        let event_type = match &val {
            ResponsesAPIStreamEvent::ResponseCreated { .. } => "response.created",
            ResponsesAPIStreamEvent::ResponseInProgress { .. } => "response.in_progress",
            ResponsesAPIStreamEvent::ResponseCompleted { .. } => "response.completed",
            ResponsesAPIStreamEvent::ResponseOutputItemAdded { .. } => "response.output_item.added",
            ResponsesAPIStreamEvent::ResponseOutputItemDone { .. } => "response.output_item.done",
            ResponsesAPIStreamEvent::ResponseContentPartAdded { .. } => {
                "response.content_part.added"
            }
            ResponsesAPIStreamEvent::ResponseContentPartDone { .. } => "response.content_part.done",
            ResponsesAPIStreamEvent::ResponseOutputTextDelta { .. } => "response.output_text.delta",
            ResponsesAPIStreamEvent::ResponseOutputTextDone { .. } => "response.output_text.done",
            ResponsesAPIStreamEvent::ResponseAudioDelta { .. } => "response.audio.delta",
            ResponsesAPIStreamEvent::ResponseAudioDone { .. } => "response.audio.done",
            ResponsesAPIStreamEvent::ResponseAudioTranscriptDelta { .. } => {
                "response.audio_transcript.delta"
            }
            ResponsesAPIStreamEvent::ResponseAudioTranscriptDone { .. } => {
                "response.audio_transcript.done"
            }
            ResponsesAPIStreamEvent::ResponseFunctionCallArgumentsDelta { .. } => {
                "response.function_call_arguments.delta"
            }
            ResponsesAPIStreamEvent::ResponseFunctionCallArgumentsDone { .. } => {
                "response.function_call_arguments.done"
            }
            ResponsesAPIStreamEvent::ResponseCodeInterpreterCallCodeDelta { .. } => {
                "response.code_interpreter_call.code.delta"
            }
            ResponsesAPIStreamEvent::ResponseCodeInterpreterCallCodeDone { .. } => {
                "response.code_interpreter_call.code.done"
            }
            ResponsesAPIStreamEvent::ResponseCodeInterpreterCallInProgress { .. } => {
                "response.code_interpreter_call.in_progress"
            }
            ResponsesAPIStreamEvent::ResponseCodeInterpreterCallInterpreting { .. } => {
                "response.code_interpreter_call.interpreting"
            }
            ResponsesAPIStreamEvent::ResponseCodeInterpreterCallCompleted { .. } => {
                "response.code_interpreter_call.completed"
            }
            ResponsesAPIStreamEvent::ResponseCustomToolCallInputDelta { .. } => {
                "response.custom_tool_call.input.delta"
            }
            ResponsesAPIStreamEvent::ResponseCustomToolCallInputDone { .. } => {
                "response.custom_tool_call.input.done"
            }
            ResponsesAPIStreamEvent::Error { .. } => "error",
            ResponsesAPIStreamEvent::Done { .. } => "done",
        };

        let event = format!("event: {}\n", event_type);
        let data = format!("data: {}\n\n", transformed_json);
        event + &data
    }
}

// ============================================================================
// ProviderStreamResponse Implementation
// ============================================================================

impl crate::providers::streaming_response::ProviderStreamResponse for ResponsesAPIStreamEvent {
    fn content_delta(&self) -> Option<&str> {
        match self {
            ResponsesAPIStreamEvent::ResponseOutputTextDelta { delta, .. } => Some(delta),
            ResponsesAPIStreamEvent::ResponseAudioDelta { delta, .. } => Some(delta),
            ResponsesAPIStreamEvent::ResponseAudioTranscriptDelta { delta, .. } => Some(delta),
            ResponsesAPIStreamEvent::ResponseFunctionCallArgumentsDelta { delta, .. } => {
                Some(delta)
            }
            ResponsesAPIStreamEvent::ResponseCodeInterpreterCallCodeDelta { delta, .. } => {
                Some(delta)
            }
            ResponsesAPIStreamEvent::ResponseCustomToolCallInputDelta { delta, .. } => Some(delta),
            _ => None,
        }
    }

    fn is_final(&self) -> bool {
        matches!(
            self,
            ResponsesAPIStreamEvent::ResponseCompleted { .. }
                | ResponsesAPIStreamEvent::Done { .. }
        )
    }

    fn role(&self) -> Option<&str> {
        match self {
            ResponsesAPIStreamEvent::ResponseOutputItemDone {
                item: OutputItem::Message { role, .. },
                ..
            } => Some(role.as_str()),
            _ => None,
        }
    }

    fn event_type(&self) -> Option<&str> {
        Some(match self {
            ResponsesAPIStreamEvent::ResponseCreated { .. } => "response.created",
            ResponsesAPIStreamEvent::ResponseInProgress { .. } => "response.in_progress",
            ResponsesAPIStreamEvent::ResponseCompleted { .. } => "response.completed",
            ResponsesAPIStreamEvent::ResponseOutputItemAdded { .. } => "response.output_item.added",
            ResponsesAPIStreamEvent::ResponseOutputItemDone { .. } => "response.output_item.done",
            ResponsesAPIStreamEvent::ResponseContentPartAdded { .. } => {
                "response.content_part.added"
            }
            ResponsesAPIStreamEvent::ResponseContentPartDone { .. } => "response.content_part.done",
            ResponsesAPIStreamEvent::ResponseOutputTextDelta { .. } => "response.output_text.delta",
            ResponsesAPIStreamEvent::ResponseOutputTextDone { .. } => "response.output_text.done",
            ResponsesAPIStreamEvent::ResponseAudioDelta { .. } => "response.audio.delta",
            ResponsesAPIStreamEvent::ResponseAudioDone { .. } => "response.audio.done",
            ResponsesAPIStreamEvent::ResponseAudioTranscriptDelta { .. } => {
                "response.audio_transcript.delta"
            }
            ResponsesAPIStreamEvent::ResponseAudioTranscriptDone { .. } => {
                "response.audio_transcript.done"
            }
            ResponsesAPIStreamEvent::ResponseFunctionCallArgumentsDelta { .. } => {
                "response.function_call_arguments.delta"
            }
            ResponsesAPIStreamEvent::ResponseFunctionCallArgumentsDone { .. } => {
                "response.function_call_arguments.done"
            }
            ResponsesAPIStreamEvent::ResponseCodeInterpreterCallCodeDelta { .. } => {
                "response.code_interpreter_call.code.delta"
            }
            ResponsesAPIStreamEvent::ResponseCodeInterpreterCallCodeDone { .. } => {
                "response.code_interpreter_call.code.done"
            }
            ResponsesAPIStreamEvent::ResponseCodeInterpreterCallInProgress { .. } => {
                "response.code_interpreter_call.in_progress"
            }
            ResponsesAPIStreamEvent::ResponseCodeInterpreterCallInterpreting { .. } => {
                "response.code_interpreter_call.interpreting"
            }
            ResponsesAPIStreamEvent::ResponseCodeInterpreterCallCompleted { .. } => {
                "response.code_interpreter_call.completed"
            }
            ResponsesAPIStreamEvent::ResponseCustomToolCallInputDelta { .. } => {
                "response.custom_tool_call.input.delta"
            }
            ResponsesAPIStreamEvent::ResponseCustomToolCallInputDone { .. } => {
                "response.custom_tool_call.input.done"
            }
            ResponsesAPIStreamEvent::Error { .. } => "error",
            ResponsesAPIStreamEvent::Done { .. } => "done",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_response_output_text_delta_deserialization() {
        let json = r#"{
            "type":"response.output_text.delta",
            "sequence_number":811,
            "item_id":"msg_0d87415661475591006924ce5465748190bdc8874257743b5c",
            "output_index":1,
            "content_index":0,
            "delta":" first",
            "logprobs":[],
            "obfuscation":"sRhca4PA06"
        }"#;

        let event: ResponsesAPIStreamEvent =
            serde_json::from_str(json).expect("Failed to deserialize");

        match event {
            ResponsesAPIStreamEvent::ResponseOutputTextDelta {
                item_id,
                output_index,
                content_index,
                delta,
                sequence_number,
                logprobs,
                obfuscation,
            } => {
                assert_eq!(
                    item_id,
                    "msg_0d87415661475591006924ce5465748190bdc8874257743b5c"
                );
                assert_eq!(output_index, 1);
                assert_eq!(content_index, 0);
                assert_eq!(delta, " first");
                assert_eq!(sequence_number, 811);
                assert_eq!(logprobs.len(), 0);
                assert_eq!(obfuscation, Some("sRhca4PA06".to_string()));
            }
            _ => panic!("Expected ResponseOutputTextDelta event"),
        }
    }

    #[test]
    fn test_response_output_text_done_deserialization() {
        let json = r#"{
            "type":"response.output_text.done",
            "sequence_number":818,
            "item_id":"msg_0d87415661475591006924ce5465748190bdc8874257743b5c",
            "output_index":1,
            "content_index":0,
            "text":"The otters linked paws and laughed.",
            "logprobs":[]
        }"#;

        let event: ResponsesAPIStreamEvent =
            serde_json::from_str(json).expect("Failed to deserialize");

        match event {
            ResponsesAPIStreamEvent::ResponseOutputTextDone {
                item_id,
                output_index,
                content_index,
                text,
                sequence_number,
                logprobs,
            } => {
                assert_eq!(
                    item_id,
                    "msg_0d87415661475591006924ce5465748190bdc8874257743b5c"
                );
                assert_eq!(output_index, 1);
                assert_eq!(content_index, 0);
                assert_eq!(text, "The otters linked paws and laughed.");
                assert_eq!(sequence_number, 818);
                assert_eq!(logprobs.len(), 0);
            }
            _ => panic!("Expected ResponseOutputTextDone event"),
        }
    }

    #[test]
    fn test_response_completed_deserialization() {
        // Simplified response.completed event
        let json = r#"{
            "type":"response.completed",
            "sequence_number":821,
            "response":{
                "id":"resp_test123",
                "object":"response",
                "created_at":1764019793,
                "status":"completed",
                "background":false,
                "error":null,
                "incomplete_details":null,
                "instructions":null,
                "max_output_tokens":null,
                "max_tool_calls":null,
                "model":"o3-2025-04-16",
                "output":[],
                "output_text":null,
                "usage":{
                    "input_tokens":17,
                    "output_tokens":946,
                    "total_tokens":963
                },
                "parallel_tool_calls":true,
                "conversation":null,
                "previous_response_id":null,
                "tools":[],
                "tool_choice":"auto",
                "temperature":1.0,
                "top_p":1.0,
                "metadata":{},
                "truncation":null,
                "user":null,
                "reasoning":null,
                "store":true,
                "text":null,
                "audio":null,
                "modalities":null,
                "service_tier":"default",
                "top_logprobs":0
            }
        }"#;

        let event: ResponsesAPIStreamEvent =
            serde_json::from_str(json).expect("Failed to deserialize");

        match event {
            ResponsesAPIStreamEvent::ResponseCompleted {
                response,
                sequence_number,
            } => {
                assert_eq!(response.id, "resp_test123");
                assert_eq!(sequence_number, 821);
                assert_eq!(response.model, "o3-2025-04-16");
            }
            _ => panic!("Expected ResponseCompleted event"),
        }
    }
}
