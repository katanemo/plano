use async_trait::async_trait;
use hermesllm::apis::openai_responses::{
    InputContent, InputItem, InputMessage, InputParam, MessageContent, MessageRole,
};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;
use std::sync::Arc;
use tracing::debug;

pub mod memory;
pub mod postgresql;
pub mod response_state_processor;

/// Represents the conversational state for a v1/responses request
/// Contains the complete input/output history that can be restored
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIConversationState {
    /// The response ID this state is associated with
    pub response_id: String,

    /// The complete input history (original input + accumulated outputs)
    /// This is what gets prepended to new requests via previous_response_id
    pub input_items: Vec<InputItem>,

    /// Timestamp when this state was created
    pub created_at: i64,

    /// Model used for this response
    pub model: String,

    /// Provider that generated this response (e.g., "anthropic", "openai")
    pub provider: String,
}

/// Error types for state storage operations
#[derive(Debug)]
pub enum StateStorageError {
    /// State not found for given response_id
    NotFound(String),

    /// Storage backend error (network, database, etc.)
    StorageError(String),

    /// Serialization/deserialization error
    SerializationError(String),
}

impl fmt::Display for StateStorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateStorageError::NotFound(id) => {
                write!(f, "Conversation state not found for response_id: {}", id)
            }
            StateStorageError::StorageError(msg) => write!(f, "Storage error: {}", msg),
            StateStorageError::SerializationError(msg) => write!(f, "Serialization error: {}", msg),
        }
    }
}

impl Error for StateStorageError {}

/// Trait for conversation state storage backends
#[async_trait]
pub trait StateStorage: Send + Sync {
    /// Store conversation state for a response
    async fn put(&self, state: OpenAIConversationState) -> Result<(), StateStorageError>;

    /// Retrieve conversation state by response_id
    async fn get(&self, response_id: &str) -> Result<OpenAIConversationState, StateStorageError>;

    /// Check if state exists for a response_id
    async fn exists(&self, response_id: &str) -> Result<bool, StateStorageError>;

    /// Delete state for a response_id (optional, for cleanup)
    async fn delete(&self, response_id: &str) -> Result<(), StateStorageError>;

    fn merge(
        &self,
        prev_state: &OpenAIConversationState,
        current_input: Vec<InputItem>,
    ) -> Vec<InputItem> {
        // Default implementation: prepend previous input, append current
        let prev_count = prev_state.input_items.len();
        let current_count = current_input.len();

        let mut combined_input = prev_state.input_items.clone();
        combined_input.extend(current_input);

        debug!(
            "PLANO | BRIGHTSTAFF | STATE_STORAGE | RESP_ID:{} | Merged state: prev_items={}, current_items={}, total_items={}, combined_json={}",
            prev_state.response_id,
            prev_count,
            current_count,
            combined_input.len(),
            serde_json::to_string(&combined_input).unwrap_or_else(|_| "serialization_error".to_string())
        );

        combined_input
    }
}

/// Storage backend type enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageBackend {
    Memory,
    Supabase,
}

impl StorageBackend {
    pub fn parse_backend(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "memory" => Some(StorageBackend::Memory),
            "supabase" => Some(StorageBackend::Supabase),
            _ => None,
        }
    }
}

// === Utility functions for state management ===

/// Extract input items from InputParam, converting text to structured format
pub fn extract_input_items(input: &InputParam) -> Vec<InputItem> {
    match input {
        InputParam::Text(text) => {
            vec![InputItem::Message(InputMessage {
                role: MessageRole::User,
                content: MessageContent::Items(vec![InputContent::InputText {
                    text: text.clone(),
                }]),
            })]
        }
        InputParam::SingleItem(item) => vec![item.clone()],
        InputParam::Items(items) => items.clone(),
    }
}

/// Retrieve previous conversation state and combine with current input
/// Returns combined input if previous state found, or original input if not found/error
pub async fn retrieve_and_combine_input(
    storage: Arc<dyn StateStorage>,
    previous_response_id: &str,
    current_input: Vec<InputItem>,
) -> Result<Vec<InputItem>, StateStorageError> {
    // First get the previous state
    let prev_state = storage.get(previous_response_id).await?;
    let combined_input = storage.merge(&prev_state, current_input);
    Ok(combined_input)
}

#[cfg(test)]
mod tests {
    use super::extract_input_items;
    use hermesllm::apis::openai_responses::{
        InputContent, InputItem, InputMessage, InputParam, MessageContent, MessageRole,
    };

    #[test]
    fn test_extract_input_items_converts_text_to_user_message_item() {
        let extracted = extract_input_items(&InputParam::Text("hello world".to_string()));
        assert_eq!(extracted.len(), 1);

        let InputItem::Message(message) = &extracted[0] else {
            panic!("expected InputItem::Message");
        };
        assert!(matches!(message.role, MessageRole::User));

        let MessageContent::Items(items) = &message.content else {
            panic!("expected MessageContent::Items");
        };
        assert_eq!(items.len(), 1);

        let InputContent::InputText { text } = &items[0] else {
            panic!("expected InputContent::InputText");
        };
        assert_eq!(text, "hello world");
    }

    #[test]
    fn test_extract_input_items_preserves_single_item() {
        let item = InputItem::Message(InputMessage {
            role: MessageRole::Assistant,
            content: MessageContent::Items(vec![InputContent::InputText {
                text: "assistant note".to_string(),
            }]),
        });

        let extracted = extract_input_items(&InputParam::SingleItem(item.clone()));
        assert_eq!(extracted.len(), 1);
        let InputItem::Message(message) = &extracted[0] else {
            panic!("expected InputItem::Message");
        };
        assert!(matches!(message.role, MessageRole::Assistant));
        let MessageContent::Items(items) = &message.content else {
            panic!("expected MessageContent::Items");
        };
        let InputContent::InputText { text } = &items[0] else {
            panic!("expected InputContent::InputText");
        };
        assert_eq!(text, "assistant note");
    }

    #[test]
    fn test_extract_input_items_preserves_items_list() {
        let items = vec![
            InputItem::Message(InputMessage {
                role: MessageRole::User,
                content: MessageContent::Items(vec![InputContent::InputText {
                    text: "first".to_string(),
                }]),
            }),
            InputItem::Message(InputMessage {
                role: MessageRole::Assistant,
                content: MessageContent::Items(vec![InputContent::InputText {
                    text: "second".to_string(),
                }]),
            }),
        ];

        let extracted = extract_input_items(&InputParam::Items(items.clone()));
        assert_eq!(extracted.len(), items.len());

        let InputItem::Message(first) = &extracted[0] else {
            panic!("expected first item to be message");
        };
        assert!(matches!(first.role, MessageRole::User));
        let MessageContent::Items(first_items) = &first.content else {
            panic!("expected MessageContent::Items");
        };
        let InputContent::InputText { text: first_text } = &first_items[0] else {
            panic!("expected InputContent::InputText");
        };
        assert_eq!(first_text, "first");

        let InputItem::Message(second) = &extracted[1] else {
            panic!("expected second item to be message");
        };
        assert!(matches!(second.role, MessageRole::Assistant));
        let MessageContent::Items(second_items) = &second.content else {
            panic!("expected MessageContent::Items");
        };
        let InputContent::InputText { text: second_text } = &second_items[0] else {
            panic!("expected InputContent::InputText");
        };
        assert_eq!(second_text, "second");
    }
}
