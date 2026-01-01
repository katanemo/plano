use super::{OpenAIConversationState, StateStorage, StateStorageError};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// In-memory storage backend for conversation state
/// Uses a HashMap wrapped in Arc<RwLock<>> for thread-safe access
#[derive(Clone)]
pub struct MemoryConversationalStorage {
    storage: Arc<RwLock<HashMap<String, OpenAIConversationState>>>,
}

impl MemoryConversationalStorage {
    pub fn new() -> Self {
        Self {
            storage: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for MemoryConversationalStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl StateStorage for MemoryConversationalStorage {
    async fn put(&self, state: OpenAIConversationState) -> Result<(), StateStorageError> {
        let response_id = state.response_id.clone();
        let mut storage = self.storage.write().await;

        debug!(
            "[PLANO | BRIGHTSTAFF | MEMORY_STORAGE] RESP_ID:{} | Storing conversation state: model={}, provider={}, input_items={}",
            response_id, state.model, state.provider, state.input_items.len()
        );

        storage.insert(response_id, state);
        Ok(())
    }

    async fn get(&self, response_id: &str) -> Result<OpenAIConversationState, StateStorageError> {
        let storage = self.storage.read().await;

        match storage.get(response_id) {
            Some(state) => {
                debug!(
                    "[PLANO | MEMORY_STORAGE | RESP_ID:{} | Retrieved conversation state: input_items={}",
                    response_id, state.input_items.len()
                );
                Ok(state.clone())
            }
            None => {
                warn!(
                    "[PLANO_RESP_ID:{} | MEMORY_STORAGE | Conversation state not found",
                    response_id
                );
                Err(StateStorageError::NotFound(response_id.to_string()))
            }
        }
    }

    async fn exists(&self, response_id: &str) -> Result<bool, StateStorageError> {
        let storage = self.storage.read().await;
        Ok(storage.contains_key(response_id))
    }

    async fn delete(&self, response_id: &str) -> Result<(), StateStorageError> {
        let mut storage = self.storage.write().await;

        if storage.remove(response_id).is_some() {
            debug!(
                "[PLANO | BRIGHTSTAFF | MEMORY_STORAGE] RESP_ID:{} | Deleted conversation state",
                response_id
            );
            Ok(())
        } else {
            Err(StateStorageError::NotFound(response_id.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hermesllm::apis::openai_responses::{
        InputContent, InputItem, InputMessage, MessageContent, MessageRole,
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

    #[tokio::test]
    async fn test_put_and_get_success() {
        let storage = MemoryConversationalStorage::new();
        let state: OpenAIConversationState = create_test_state("resp_001", 3);

        // Store
        storage.put(state.clone()).await.unwrap();

        // Retrieve
        let retrieved = storage.get("resp_001").await.unwrap();
        assert_eq!(retrieved.response_id, state.response_id);
        assert_eq!(retrieved.model, state.model);
        assert_eq!(retrieved.provider, state.provider);
        assert_eq!(retrieved.input_items.len(), 3);
        assert_eq!(retrieved.created_at, state.created_at);
    }

    #[tokio::test]
    async fn test_put_overwrites_existing() {
        let storage = MemoryConversationalStorage::new();

        // First state
        let state1 = create_test_state("resp_002", 2);
        storage.put(state1).await.unwrap();

        // Overwrite with new state
        let state2 = OpenAIConversationState {
            response_id: "resp_002".to_string(),
            input_items: vec![],
            created_at: 9999999999,
            model: "gpt-4".to_string(),
            provider: "openai".to_string(),
        };
        storage.put(state2.clone()).await.unwrap();

        // Should retrieve the new state
        let retrieved = storage.get("resp_002").await.unwrap();
        assert_eq!(retrieved.model, "gpt-4");
        assert_eq!(retrieved.provider, "openai");
        assert_eq!(retrieved.input_items.len(), 0);
        assert_eq!(retrieved.created_at, 9999999999);
    }

    #[tokio::test]
    async fn test_get_not_found() {
        let storage = MemoryConversationalStorage::new();

        let result = storage.get("nonexistent").await;
        assert!(result.is_err());

        match result.unwrap_err() {
            StateStorageError::NotFound(id) => {
                assert_eq!(id, "nonexistent");
            }
            _ => panic!("Expected NotFound error"),
        }
    }

    #[tokio::test]
    async fn test_exists_returns_false_for_nonexistent() {
        let storage = MemoryConversationalStorage::new();
        assert!(!storage.exists("resp_003").await.unwrap());
    }

    #[tokio::test]
    async fn test_exists_returns_true_after_put() {
        let storage = MemoryConversationalStorage::new();
        let state = create_test_state("resp_004", 1);

        assert!(!storage.exists("resp_004").await.unwrap());
        storage.put(state).await.unwrap();
        assert!(storage.exists("resp_004").await.unwrap());
    }

    #[tokio::test]
    async fn test_delete_success() {
        let storage = MemoryConversationalStorage::new();
        let state = create_test_state("resp_005", 2);

        storage.put(state).await.unwrap();
        assert!(storage.exists("resp_005").await.unwrap());

        // Delete
        storage.delete("resp_005").await.unwrap();

        // Should no longer exist
        assert!(!storage.exists("resp_005").await.unwrap());
        assert!(storage.get("resp_005").await.is_err());
    }

    #[tokio::test]
    async fn test_delete_not_found() {
        let storage = MemoryConversationalStorage::new();

        let result = storage.delete("nonexistent").await;
        assert!(result.is_err());

        match result.unwrap_err() {
            StateStorageError::NotFound(id) => {
                assert_eq!(id, "nonexistent");
            }
            _ => panic!("Expected NotFound error"),
        }
    }

    #[tokio::test]
    async fn test_merge_combines_inputs() {
        let storage = MemoryConversationalStorage::new();

        // Create a previous state with 2 messages
        let prev_state = create_test_state("resp_006", 2);

        // Create current input with 1 message
        let current_input = vec![InputItem::Message(InputMessage {
            role: MessageRole::User,
            content: MessageContent::Items(vec![InputContent::InputText {
                text: "New message".to_string(),
            }]),
        })];

        // Merge
        let merged = storage.merge(&prev_state, current_input);

        // Should have 3 messages total (2 from prev + 1 current)
        assert_eq!(merged.len(), 3);
    }

    #[tokio::test]
    async fn test_merge_preserves_order() {
        let storage = MemoryConversationalStorage::new();

        // Previous state has messages 0 and 1
        let prev_state = create_test_state("resp_007", 2);

        // Current input has message 2
        let current_input = vec![InputItem::Message(InputMessage {
            role: MessageRole::User,
            content: MessageContent::Items(vec![InputContent::InputText {
                text: "Message 2".to_string(),
            }]),
        })];

        let merged = storage.merge(&prev_state, current_input);

        // Verify order: prev messages first, then current
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

        // Should just have the previous state's items
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

        // Should just have the current input
        assert_eq!(merged.len(), 1);
    }

    #[tokio::test]
    async fn test_concurrent_access() {
        let storage = MemoryConversationalStorage::new();

        // Spawn multiple tasks that write concurrently
        let mut handles = vec![];

        for i in 0..10 {
            let storage_clone = storage.clone();
            let handle = tokio::spawn(async move {
                let state = create_test_state(&format!("resp_{}", i), i % 3);
                storage_clone.put(state).await.unwrap();
            });
            handles.push(handle);
        }

        // Wait for all tasks
        for handle in handles {
            handle.await.unwrap();
        }

        // Verify all states were stored
        for i in 0..10 {
            assert!(storage.exists(&format!("resp_{}", i)).await.unwrap());
        }
    }

    #[tokio::test]
    async fn test_multiple_operations_on_same_id() {
        let storage = MemoryConversationalStorage::new();
        let state = create_test_state("resp_010", 1);

        // Put
        storage.put(state.clone()).await.unwrap();

        // Get
        let retrieved = storage.get("resp_010").await.unwrap();
        assert_eq!(retrieved.response_id, "resp_010");

        // Exists
        assert!(storage.exists("resp_010").await.unwrap());

        // Put again (overwrite)
        let new_state = create_test_state("resp_010", 5);
        storage.put(new_state).await.unwrap();

        // Get updated
        let updated = storage.get("resp_010").await.unwrap();
        assert_eq!(updated.input_items.len(), 5);

        // Delete
        storage.delete("resp_010").await.unwrap();

        // Should not exist
        assert!(!storage.exists("resp_010").await.unwrap());
    }

    #[tokio::test]
    async fn test_merge_with_tool_call_flow() {
        // This test simulates a realistic tool call conversation flow:
        // 1. User sends message: "What's the weather?"
        // 2. Model responds with function call (converted to assistant message)
        // 3. User sends function call output in next request with previous_response_id
        // The merge should combine: user message + assistant function call + function output

        let storage = MemoryConversationalStorage::new();

        // Step 1: Previous state contains the initial exchange
        // - User message: "What's the weather in SF?"
        // - Assistant message (converted from FunctionCall): "Called function: get_weather..."
        let prev_state = OpenAIConversationState {
            response_id: "resp_tool_001".to_string(),
            input_items: vec![
                // Original user message
                InputItem::Message(InputMessage {
                    role: MessageRole::User,
                    content: MessageContent::Items(vec![InputContent::InputText {
                        text: "What's the weather in San Francisco?".to_string(),
                    }]),
                }),
                // Assistant's function call (converted from OutputItem::FunctionCall)
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

        // Step 2: Current request includes function call output
        let current_input = vec![InputItem::Message(InputMessage {
            role: MessageRole::User,
            content: MessageContent::Items(vec![InputContent::InputText {
                text: "Function result: {\"temperature\": 72, \"condition\": \"sunny\"}"
                    .to_string(),
            }]),
        })];

        // Step 3: Merge should combine all conversation history
        let merged = storage.merge(&prev_state, current_input);

        // Should have 3 items: user question + assistant function call + function output
        assert_eq!(merged.len(), 3);

        // Verify the order and content
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
        // Test a more complex scenario with multiple tool calls
        let storage = MemoryConversationalStorage::new();

        // Previous state has: user message + 2 function calls from assistant
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

        // Current input: function outputs for both calls
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

        // Should have 5 items total: 1 user + 2 assistant calls + 2 function outputs
        assert_eq!(merged.len(), 5);

        // Verify first item is original user message
        let InputItem::Message(first) = &merged[0] else {
            panic!("Expected Message")
        };
        assert!(matches!(first.role, MessageRole::User));

        // Verify last two are function outputs
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
        // Simulate a multi-turn conversation with tool calls
        let storage = MemoryConversationalStorage::new();

        // Previous state: full conversation history up to this point
        let prev_state = OpenAIConversationState {
            response_id: "resp_tool_003".to_string(),
            input_items: vec![
                // Turn 1: User asks about weather
                InputItem::Message(InputMessage {
                    role: MessageRole::User,
                    content: MessageContent::Items(vec![InputContent::InputText {
                        text: "What's the weather?".to_string(),
                    }]),
                }),
                // Turn 1: Assistant calls get_weather
                InputItem::Message(InputMessage {
                    role: MessageRole::Assistant,
                    content: MessageContent::Items(vec![InputContent::InputText {
                        text: "Called function: get_weather".to_string(),
                    }]),
                }),
                // Turn 2: User provides function output
                InputItem::Message(InputMessage {
                    role: MessageRole::User,
                    content: MessageContent::Items(vec![InputContent::InputText {
                        text: "Weather: sunny, 72°F".to_string(),
                    }]),
                }),
                // Turn 2: Assistant responds with text
                InputItem::Message(InputMessage {
                    role: MessageRole::Assistant,
                    content: MessageContent::Items(vec![InputContent::InputText {
                        text: "It's sunny and 72°F in San Francisco today!".to_string(),
                    }]),
                }),
            ],
            created_at: 1234567890,
            model: "claude-3".to_string(),
            provider: "anthropic".to_string(),
        };

        // Turn 3: User asks follow-up question
        let current_input = vec![InputItem::Message(InputMessage {
            role: MessageRole::User,
            content: MessageContent::Items(vec![InputContent::InputText {
                text: "Should I bring an umbrella?".to_string(),
            }]),
        })];

        let merged = storage.merge(&prev_state, current_input);

        // Should have all 5 messages in order
        assert_eq!(merged.len(), 5);

        // Verify the entire conversation flow is preserved
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
