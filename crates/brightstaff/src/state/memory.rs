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
    use crate::state::generate_storage_tests;
    use hermesllm::apis::openai_responses::{
        InputContent, InputItem, InputMessage, MessageContent, MessageRole,
    };

    fn create_test_state(response_id: &str) -> OpenAIConversationState {
        OpenAIConversationState {
            response_id: response_id.to_string(),
            input_items: vec![InputItem::Message(InputMessage {
                role: MessageRole::User,
                content: MessageContent::Items(vec![InputContent::InputText {
                    text: "Test message".to_string(),
                }]),
            })],
            created_at: 1234567890,
            model: "claude-3".to_string(),
            provider: "anthropic".to_string(),
        }
    }

    fn create_test_state_with_messages(
        response_id: &str,
        num_messages: usize,
    ) -> OpenAIConversationState {
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

    // Generate the standard CRUD tests via macro
    generate_storage_tests!(MemoryConversationalStorage::new());

    #[tokio::test]
    async fn test_concurrent_access() {
        let storage = MemoryConversationalStorage::new();

        // Spawn multiple tasks that write concurrently
        let mut handles = vec![];

        for i in 0..10 {
            let storage_clone = storage.clone();
            let handle = tokio::spawn(async move {
                let state = create_test_state_with_messages(&format!("resp_{}", i), i % 3);
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
        let state = create_test_state_with_messages("resp_010", 1);

        // Put
        storage.put(state.clone()).await.unwrap();

        // Get
        let retrieved = storage.get("resp_010").await.unwrap();
        assert_eq!(retrieved.response_id, "resp_010");

        // Exists
        assert!(storage.exists("resp_010").await.unwrap());

        // Put again (overwrite)
        let new_state = create_test_state_with_messages("resp_010", 5);
        storage.put(new_state).await.unwrap();

        // Get updated
        let updated = storage.get("resp_010").await.unwrap();
        assert_eq!(updated.input_items.len(), 5);

        // Delete
        storage.delete("resp_010").await.unwrap();

        // Should not exist
        assert!(!storage.exists("resp_010").await.unwrap());
    }
}
