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

    // Generate the standard CRUD tests via macro
    generate_storage_tests!({
        let Some(storage) = get_test_storage().await else {
            return;
        };
        storage
    });

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

        println!("Data written to Supabase!");
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
