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
macro_rules! generate_storage_tests {
    ($create_storage:expr) => {
        #[tokio::test]
        async fn test_put_and_get_success() {
            let storage = $create_storage;
            let state = create_test_state("resp_001");
            storage.put(state.clone()).await.unwrap();
            let retrieved = storage.get("resp_001").await.unwrap();
            assert_eq!(retrieved.response_id, "resp_001");
            assert_eq!(retrieved.model, state.model);
            assert_eq!(retrieved.provider, state.provider);
            assert_eq!(retrieved.input_items.len(), state.input_items.len());
            assert_eq!(retrieved.created_at, state.created_at);
        }

        #[tokio::test]
        async fn test_put_overwrites_existing() {
            let storage = $create_storage;
            let state1 = create_test_state("resp_002");
            storage.put(state1).await.unwrap();
            let state2 = OpenAIConversationState {
                response_id: "resp_002".to_string(),
                input_items: vec![],
                created_at: 9999999999,
                model: "gpt-4".to_string(),
                provider: "openai".to_string(),
            };
            storage.put(state2).await.unwrap();
            let retrieved = storage.get("resp_002").await.unwrap();
            assert_eq!(retrieved.model, "gpt-4");
            assert_eq!(retrieved.provider, "openai");
            assert_eq!(retrieved.input_items.len(), 0);
            assert_eq!(retrieved.created_at, 9999999999);
        }

        #[tokio::test]
        async fn test_get_not_found() {
            let storage = $create_storage;
            let result = storage.get("nonexistent").await;
            assert!(result.is_err());
            assert!(matches!(
                result.unwrap_err(),
                StateStorageError::NotFound(_)
            ));
        }

        #[tokio::test]
        async fn test_exists_returns_false_for_nonexistent() {
            let storage = $create_storage;
            assert!(!storage.exists("nonexistent").await.unwrap());
        }

        #[tokio::test]
        async fn test_exists_returns_true_after_put() {
            let storage = $create_storage;
            let state = create_test_state("resp_004");
            assert!(!storage.exists("resp_004").await.unwrap());
            storage.put(state).await.unwrap();
            assert!(storage.exists("resp_004").await.unwrap());
        }

        #[tokio::test]
        async fn test_delete_success() {
            let storage = $create_storage;
            let state = create_test_state("resp_005");
            storage.put(state).await.unwrap();
            assert!(storage.exists("resp_005").await.unwrap());
            storage.delete("resp_005").await.unwrap();
            assert!(!storage.exists("resp_005").await.unwrap());
            assert!(storage.get("resp_005").await.is_err());
        }

        #[tokio::test]
        async fn test_delete_not_found() {
            let storage = $create_storage;
            let result = storage.delete("nonexistent").await;
            assert!(result.is_err());
            assert!(matches!(
                result.unwrap_err(),
                StateStorageError::NotFound(_)
            ));
        }
    };
}

#[cfg(test)]
pub(crate) use generate_storage_tests;

#[cfg(test)]
mod tests {
    use super::extract_input_items;
    use super::memory::MemoryConversationalStorage;
    use super::{OpenAIConversationState, StateStorage};
    use hermesllm::apis::openai_responses::{
        InputContent, InputItem, InputMessage, InputParam, MessageContent, MessageRole,
    };

    fn create_test_state(response_id: &str, num_messages: usize) -> OpenAIConversationState {
        let mut input_items = Vec::new();
        for i in 0..num_messages {
            input_items.push(InputItem::Message(InputMessage {
                role: if i % 2 == 0 {
                    MessageRole::User
                } else {
                    MessageRole::Assistant
                },
                content: MessageContent::Items(vec![InputContent::InputText {
                    text: format!("Message {}", i),
                }]),
            }));
        }

        OpenAIConversationState {
            response_id: response_id.to_string(),
            input_items,
            created_at: 1234567890,
            model: "claude-3".to_string(),
            provider: "anthropic".to_string(),
        }
    }

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

    // === Merge tests (testing the default trait method) ===

    #[tokio::test]
    async fn test_merge_combines_inputs() {
        let storage = MemoryConversationalStorage::new();
        let prev_state = create_test_state("resp_006", 2);

        let current_input = vec![InputItem::Message(InputMessage {
            role: MessageRole::User,
            content: MessageContent::Items(vec![InputContent::InputText {
                text: "New message".to_string(),
            }]),
        })];

        let merged = storage.merge(&prev_state, current_input);
        assert_eq!(merged.len(), 3);
    }

    #[tokio::test]
    async fn test_merge_preserves_order() {
        let storage = MemoryConversationalStorage::new();
        let prev_state = create_test_state("resp_007", 2);

        let current_input = vec![InputItem::Message(InputMessage {
            role: MessageRole::User,
            content: MessageContent::Items(vec![InputContent::InputText {
                text: "Message 2".to_string(),
            }]),
        })];

        let merged = storage.merge(&prev_state, current_input);

        let InputItem::Message(msg) = &merged[0] else {
            panic!("Expected Message")
        };
        match &msg.content {
            MessageContent::Items(items) => match &items[0] {
                InputContent::InputText { text } => assert_eq!(text, "Message 0"),
                _ => panic!("Expected InputText"),
            },
            _ => panic!("Expected MessageContent::Items"),
        }

        let InputItem::Message(msg) = &merged[2] else {
            panic!("Expected Message")
        };
        match &msg.content {
            MessageContent::Items(items) => match &items[0] {
                InputContent::InputText { text } => assert_eq!(text, "Message 2"),
                _ => panic!("Expected InputText"),
            },
            _ => panic!("Expected MessageContent::Items"),
        }
    }

    #[tokio::test]
    async fn test_merge_with_empty_current_input() {
        let storage = MemoryConversationalStorage::new();
        let prev_state = create_test_state("resp_008", 3);

        let merged = storage.merge(&prev_state, vec![]);
        assert_eq!(merged.len(), 3);
    }

    #[tokio::test]
    async fn test_merge_with_empty_previous_state() {
        let storage = MemoryConversationalStorage::new();

        let prev_state = OpenAIConversationState {
            response_id: "resp_009".to_string(),
            input_items: vec![],
            created_at: 1234567890,
            model: "gpt-4".to_string(),
            provider: "openai".to_string(),
        };

        let current_input = vec![InputItem::Message(InputMessage {
            role: MessageRole::User,
            content: MessageContent::Items(vec![InputContent::InputText {
                text: "Only message".to_string(),
            }]),
        })];

        let merged = storage.merge(&prev_state, current_input);
        assert_eq!(merged.len(), 1);
    }

    #[tokio::test]
    async fn test_merge_with_tool_call_flow() {
        let storage = MemoryConversationalStorage::new();

        let prev_state = OpenAIConversationState {
            response_id: "resp_tool_001".to_string(),
            input_items: vec![
                InputItem::Message(InputMessage {
                    role: MessageRole::User,
                    content: MessageContent::Items(vec![InputContent::InputText {
                        text: "What's the weather in San Francisco?".to_string(),
                    }]),
                }),
                InputItem::Message(InputMessage {
                    role: MessageRole::Assistant,
                    content: MessageContent::Items(vec![InputContent::InputText {
                        text: "Called function: get_weather with arguments: {\"location\":\"San Francisco, CA\"}".to_string(),
                    }]),
                }),
            ],
            created_at: 1234567890,
            model: "claude-3".to_string(),
            provider: "anthropic".to_string(),
        };

        let current_input = vec![InputItem::Message(InputMessage {
            role: MessageRole::User,
            content: MessageContent::Items(vec![InputContent::InputText {
                text: "Function result: {\"temperature\": 72, \"condition\": \"sunny\"}"
                    .to_string(),
            }]),
        })];

        let merged = storage.merge(&prev_state, current_input);
        assert_eq!(merged.len(), 3);

        let InputItem::Message(msg1) = &merged[0] else {
            panic!("Expected Message")
        };
        assert!(matches!(msg1.role, MessageRole::User));
        match &msg1.content {
            MessageContent::Items(items) => match &items[0] {
                InputContent::InputText { text } => {
                    assert!(text.contains("weather in San Francisco"));
                }
                _ => panic!("Expected InputText"),
            },
            _ => panic!("Expected MessageContent::Items"),
        }

        let InputItem::Message(msg2) = &merged[1] else {
            panic!("Expected Message")
        };
        assert!(matches!(msg2.role, MessageRole::Assistant));
        match &msg2.content {
            MessageContent::Items(items) => match &items[0] {
                InputContent::InputText { text } => {
                    assert!(text.contains("get_weather"));
                }
                _ => panic!("Expected InputText"),
            },
            _ => panic!("Expected MessageContent::Items"),
        }

        let InputItem::Message(msg3) = &merged[2] else {
            panic!("Expected Message")
        };
        assert!(matches!(msg3.role, MessageRole::User));
        match &msg3.content {
            MessageContent::Items(items) => match &items[0] {
                InputContent::InputText { text } => {
                    assert!(text.contains("Function result"));
                    assert!(text.contains("temperature"));
                }
                _ => panic!("Expected InputText"),
            },
            _ => panic!("Expected MessageContent::Items"),
        }
    }

    #[tokio::test]
    async fn test_merge_with_multiple_tool_calls() {
        let storage = MemoryConversationalStorage::new();

        let prev_state = OpenAIConversationState {
            response_id: "resp_tool_002".to_string(),
            input_items: vec![
                InputItem::Message(InputMessage {
                    role: MessageRole::User,
                    content: MessageContent::Items(vec![InputContent::InputText {
                        text: "What's the weather and time in SF?".to_string(),
                    }]),
                }),
                InputItem::Message(InputMessage {
                    role: MessageRole::Assistant,
                    content: MessageContent::Items(vec![InputContent::InputText {
                        text: "Called function: get_weather with arguments: {\"location\":\"SF\"}".to_string(),
                    }]),
                }),
                InputItem::Message(InputMessage {
                    role: MessageRole::Assistant,
                    content: MessageContent::Items(vec![InputContent::InputText {
                        text: "Called function: get_time with arguments: {\"timezone\":\"America/Los_Angeles\"}".to_string(),
                    }]),
                }),
            ],
            created_at: 1234567890,
            model: "gpt-4".to_string(),
            provider: "openai".to_string(),
        };

        let current_input = vec![
            InputItem::Message(InputMessage {
                role: MessageRole::User,
                content: MessageContent::Items(vec![InputContent::InputText {
                    text: "Weather result: {\"temp\": 68}".to_string(),
                }]),
            }),
            InputItem::Message(InputMessage {
                role: MessageRole::User,
                content: MessageContent::Items(vec![InputContent::InputText {
                    text: "Time result: {\"time\": \"14:30\"}".to_string(),
                }]),
            }),
        ];

        let merged = storage.merge(&prev_state, current_input);
        assert_eq!(merged.len(), 5);

        let InputItem::Message(first) = &merged[0] else {
            panic!("Expected Message")
        };
        assert!(matches!(first.role, MessageRole::User));

        let InputItem::Message(second_last) = &merged[3] else {
            panic!("Expected Message")
        };
        assert!(matches!(second_last.role, MessageRole::User));
        match &second_last.content {
            MessageContent::Items(items) => match &items[0] {
                InputContent::InputText { text } => assert!(text.contains("Weather result")),
                _ => panic!("Expected InputText"),
            },
            _ => panic!("Expected MessageContent::Items"),
        }

        let InputItem::Message(last) = &merged[4] else {
            panic!("Expected Message")
        };
        assert!(matches!(last.role, MessageRole::User));
        match &last.content {
            MessageContent::Items(items) => match &items[0] {
                InputContent::InputText { text } => assert!(text.contains("Time result")),
                _ => panic!("Expected InputText"),
            },
            _ => panic!("Expected MessageContent::Items"),
        }
    }

    #[tokio::test]
    async fn test_merge_preserves_conversation_context_for_multi_turn() {
        let storage = MemoryConversationalStorage::new();

        let prev_state = OpenAIConversationState {
            response_id: "resp_tool_003".to_string(),
            input_items: vec![
                InputItem::Message(InputMessage {
                    role: MessageRole::User,
                    content: MessageContent::Items(vec![InputContent::InputText {
                        text: "What's the weather?".to_string(),
                    }]),
                }),
                InputItem::Message(InputMessage {
                    role: MessageRole::Assistant,
                    content: MessageContent::Items(vec![InputContent::InputText {
                        text: "Called function: get_weather".to_string(),
                    }]),
                }),
                InputItem::Message(InputMessage {
                    role: MessageRole::User,
                    content: MessageContent::Items(vec![InputContent::InputText {
                        text: "Weather: sunny, 72\u{00b0}F".to_string(),
                    }]),
                }),
                InputItem::Message(InputMessage {
                    role: MessageRole::Assistant,
                    content: MessageContent::Items(vec![InputContent::InputText {
                        text: "It's sunny and 72\u{00b0}F in San Francisco today!".to_string(),
                    }]),
                }),
            ],
            created_at: 1234567890,
            model: "claude-3".to_string(),
            provider: "anthropic".to_string(),
        };

        let current_input = vec![InputItem::Message(InputMessage {
            role: MessageRole::User,
            content: MessageContent::Items(vec![InputContent::InputText {
                text: "Should I bring an umbrella?".to_string(),
            }]),
        })];

        let merged = storage.merge(&prev_state, current_input);
        assert_eq!(merged.len(), 5);

        let InputItem::Message(first) = &merged[0] else {
            panic!("Expected Message")
        };
        match &first.content {
            MessageContent::Items(items) => match &items[0] {
                InputContent::InputText { text } => assert!(text.contains("What's the weather")),
                _ => panic!("Expected InputText"),
            },
            _ => panic!("Expected MessageContent::Items"),
        }

        let InputItem::Message(last) = &merged[4] else {
            panic!("Expected Message")
        };
        match &last.content {
            MessageContent::Items(items) => match &items[0] {
                InputContent::InputText { text } => assert!(text.contains("umbrella")),
                _ => panic!("Expected InputText"),
            },
            _ => panic!("Expected MessageContent::Items"),
        }
    }
}
