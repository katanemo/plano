use super::{OpenAIConversationState, StateStorage, StateStorageError};
use async_trait::async_trait;
use serde_json;
use std::sync::Arc;
use tokio::sync::OnceCell;
use tokio_postgres::{Client, NoTls};
use tracing::{debug, info, warn};

/// Supabase/PostgreSQL storage backend for conversation state
#[derive(Clone)]
pub struct PostgreSQLConversationStorage {
    client: Arc<Client>,
    table_verified: Arc<OnceCell<()>>,
}

impl PostgreSQLConversationStorage {
    /// Creates a new Supabase storage instance with the given connection string
    pub async fn new(connection_string: String) -> Result<Self, StateStorageError> {
        let (client, connection) = tokio_postgres::connect(&connection_string, NoTls)
            .await
            .map_err(|e| {
                StateStorageError::StorageError(format!("Failed to connect to database: {}", e))
            })?;

        // Spawn the connection to run in the background
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                warn!("Database connection error: {}", e);
            }
        });

        Ok(Self {
            client: Arc::new(client),
            table_verified: Arc::new(OnceCell::new()),
        })
    }

    /// Ensures the conversation_states table exists (checks once, caches result)
    async fn ensure_ready(&self) -> Result<(), StateStorageError> {
        self.table_verified
            .get_or_try_init(|| async {
                let row = self
                    .client
                    .query_one(
                        "SELECT EXISTS (
                            SELECT FROM pg_tables
                            WHERE tablename = 'conversation_states'
                        )",
                        &[],
                    )
                    .await
                    .map_err(|e| {
                        StateStorageError::StorageError(format!(
                            "Failed to verify table existence: {}",
                            e
                        ))
                    })?;

                let exists: bool = row.get(0);

                if !exists {
                    return Err(StateStorageError::StorageError(
                        "Table 'conversation_states' does not exist. \
                         Please run the setup SQL from docs/db_setup/conversation_states.sql"
                            .to_string(),
                    ));
                }

                info!("Conversation state storage table verified");
                Ok(())
            })
            .await?;

        Ok(())
    }
}

#[async_trait]
impl StateStorage for PostgreSQLConversationStorage {
    async fn put(&self, state: OpenAIConversationState) -> Result<(), StateStorageError> {
        self.ensure_ready().await?;

        // Serialize input_items to JSONB
        let input_items_json = serde_json::to_value(&state.input_items).map_err(|e| {
            StateStorageError::StorageError(format!("Failed to serialize input_items: {}", e))
        })?;

        // Upsert the conversation state
        self.client
            .execute(
                r#"
                INSERT INTO conversation_states
                    (response_id, input_items, created_at, model, provider, updated_at)
                VALUES ($1, $2, $3, $4, $5, NOW())
                ON CONFLICT (response_id)
                DO UPDATE SET
                    input_items = EXCLUDED.input_items,
                    model = EXCLUDED.model,
                    provider = EXCLUDED.provider,
                    updated_at = NOW()
                "#,
                &[
                    &state.response_id,
                    &input_items_json,
                    &state.created_at,
                    &state.model,
                    &state.provider,
                ],
            )
            .await
            .map_err(|e| {
                StateStorageError::StorageError(format!(
                    "Failed to store conversation state for {}: {}",
                    state.response_id, e
                ))
            })?;

        debug!("Stored conversation state for {}", state.response_id);
        Ok(())
    }

    async fn get(&self, response_id: &str) -> Result<OpenAIConversationState, StateStorageError> {
        self.ensure_ready().await?;

        let row = self
            .client
            .query_opt(
                r#"
                SELECT response_id, input_items, created_at, model, provider
                FROM conversation_states
                WHERE response_id = $1
                "#,
                &[&response_id],
            )
            .await
            .map_err(|e| {
                StateStorageError::StorageError(format!(
                    "Failed to fetch conversation state for {}: {}",
                    response_id, e
                ))
            })?;

        match row {
            Some(row) => {
                let response_id: String = row.get("response_id");
                let input_items_json: serde_json::Value = row.get("input_items");
                let created_at: i64 = row.get("created_at");
                let model: String = row.get("model");
                let provider: String = row.get("provider");

                // Deserialize input_items from JSONB
                let input_items = serde_json::from_value(input_items_json).map_err(|e| {
                    StateStorageError::StorageError(format!(
                        "Failed to deserialize input_items: {}",
                        e
                    ))
                })?;

                Ok(OpenAIConversationState {
                    response_id,
                    input_items,
                    created_at,
                    model,
                    provider,
                })
            }
            None => Err(StateStorageError::NotFound(format!(
                "Conversation state not found for response_id: {}",
                response_id
            ))),
        }
    }

    async fn exists(&self, response_id: &str) -> Result<bool, StateStorageError> {
        self.ensure_ready().await?;

        let row = self
            .client
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM conversation_states WHERE response_id = $1)",
                &[&response_id],
            )
            .await
            .map_err(|e| {
                StateStorageError::StorageError(format!(
                    "Failed to check existence for {}: {}",
                    response_id, e
                ))
            })?;

        let exists: bool = row.get(0);
        Ok(exists)
    }

    async fn delete(&self, response_id: &str) -> Result<(), StateStorageError> {
        self.ensure_ready().await?;

        let rows_affected = self
            .client
            .execute(
                "DELETE FROM conversation_states WHERE response_id = $1",
                &[&response_id],
            )
            .await
            .map_err(|e| {
                StateStorageError::StorageError(format!(
                    "Failed to delete conversation state for {}: {}",
                    response_id, e
                ))
            })?;

        if rows_affected == 0 {
            return Err(StateStorageError::NotFound(format!(
                "Conversation state not found for response_id: {}",
                response_id
            )));
        }

        debug!("Deleted conversation state for {}", response_id);
        Ok(())
    }
}

/*
PostgreSQL schema is maintained in docs/db_setup/conversation_states.sql
Run that SQL file against your database before using this storage backend.
*/

#[cfg(test)]
mod tests {
    use super::*;
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
            model: "gpt-4".to_string(),
            provider: "openai".to_string(),
        }
    }

    // Note: These tests require a running PostgreSQL database
    // Set TEST_DATABASE_URL environment variable to run integration tests
    // Example: TEST_DATABASE_URL=postgresql://user:pass@localhost/test_db

    async fn get_test_storage() -> Option<PostgreSQLConversationStorage> {
        if let Ok(db_url) = std::env::var("TEST_DATABASE_URL") {
            match PostgreSQLConversationStorage::new(db_url).await {
                Ok(storage) => Some(storage),
                Err(e) => {
                    eprintln!("Failed to create test storage: {}", e);
                    None
                }
            }
        } else {
            eprintln!("TEST_DATABASE_URL not set, skipping Supabase integration tests");
            None
        }
    }

    #[tokio::test]
    async fn test_supabase_put_and_get_success() {
        let Some(storage) = get_test_storage().await else {
            return;
        };

        let state = create_test_state("test_resp_001");
        storage.put(state.clone()).await.unwrap();

        let retrieved = storage.get("test_resp_001").await.unwrap();
        assert_eq!(retrieved.response_id, "test_resp_001");
        assert_eq!(retrieved.input_items.len(), 1);
        assert_eq!(retrieved.model, "gpt-4");
        assert_eq!(retrieved.provider, "openai");

        // Cleanup
        let _ = storage.delete("test_resp_001").await;
    }

    #[tokio::test]
    async fn test_supabase_put_overwrites_existing() {
        let Some(storage) = get_test_storage().await else {
            return;
        };

        let state1 = create_test_state("test_resp_002");
        storage.put(state1).await.unwrap();

        let mut state2 = create_test_state("test_resp_002");
        state2.model = "gpt-4-turbo".to_string();
        state2.input_items.push(InputItem::Message(InputMessage {
            role: MessageRole::Assistant,
            content: MessageContent::Items(vec![InputContent::InputText {
                text: "Response".to_string(),
            }]),
        }));
        storage.put(state2).await.unwrap();

        let retrieved = storage.get("test_resp_002").await.unwrap();
        assert_eq!(retrieved.model, "gpt-4-turbo");
        assert_eq!(retrieved.input_items.len(), 2);

        // Cleanup
        let _ = storage.delete("test_resp_002").await;
    }

    #[tokio::test]
    async fn test_supabase_get_not_found() {
        let Some(storage) = get_test_storage().await else {
            return;
        };

        let result = storage.get("nonexistent_id").await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            StateStorageError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn test_supabase_exists_returns_false() {
        let Some(storage) = get_test_storage().await else {
            return;
        };

        let exists = storage.exists("nonexistent_id").await.unwrap();
        assert!(!exists);
    }

    #[tokio::test]
    async fn test_supabase_exists_returns_true_after_put() {
        let Some(storage) = get_test_storage().await else {
            return;
        };

        let state = create_test_state("test_resp_003");
        storage.put(state).await.unwrap();

        let exists = storage.exists("test_resp_003").await.unwrap();
        assert!(exists);

        // Cleanup
        let _ = storage.delete("test_resp_003").await;
    }

    #[tokio::test]
    async fn test_supabase_delete_success() {
        let Some(storage) = get_test_storage().await else {
            return;
        };

        let state = create_test_state("test_resp_004");
        storage.put(state).await.unwrap();

        storage.delete("test_resp_004").await.unwrap();

        let exists = storage.exists("test_resp_004").await.unwrap();
        assert!(!exists);
    }

    #[tokio::test]
    async fn test_supabase_delete_not_found() {
        let Some(storage) = get_test_storage().await else {
            return;
        };

        let result = storage.delete("nonexistent_id").await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            StateStorageError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn test_supabase_merge_works() {
        let Some(storage) = get_test_storage().await else {
            return;
        };

        let prev_state = create_test_state("test_resp_005");
        let current_input = vec![InputItem::Message(InputMessage {
            role: MessageRole::User,
            content: MessageContent::Items(vec![InputContent::InputText {
                text: "New message".to_string(),
            }]),
        })];

        let merged = storage.merge(&prev_state, current_input);

        // Should have 2 messages (1 from prev + 1 current)
        assert_eq!(merged.len(), 2);
    }

    #[tokio::test]
    async fn test_supabase_table_verification() {
        let Some(storage) = get_test_storage().await else {
            return;
        };

        // This should trigger table verification
        let result = storage.ensure_ready().await;
        assert!(result.is_ok(), "Table verification should succeed");

        // Second call should use cached result
        let result2 = storage.ensure_ready().await;
        assert!(result2.is_ok(), "Cached verification should succeed");
    }

    #[tokio::test]
    #[ignore] // Run manually with: cargo test test_verify_data_in_supabase -- --ignored
    async fn test_verify_data_in_supabase() {
        let Some(storage) = get_test_storage().await else {
            return;
        };

        // Create a test record that persists
        let state = create_test_state("manual_test_verification");
        storage.put(state).await.unwrap();

        println!("✅ Data written to Supabase!");
        println!("Check your Supabase dashboard:");
        println!(
            "  SELECT * FROM conversation_states WHERE response_id = 'manual_test_verification';"
        );
        println!("\nTo cleanup, run:");
        println!(
            "  DELETE FROM conversation_states WHERE response_id = 'manual_test_verification';"
        );

        // DON'T cleanup - leave it for manual verification
    }
}
