use std::sync::Arc;

use hermesllm::apis::openai::{ChatCompletionsRequest, Message, MessageContent, Role};
use hyper::header::HeaderMap;

use crate::handlers::agent_selector::{AgentSelectionError, AgentSelector};
use crate::handlers::pipeline_processor::PipelineProcessor;
use crate::handlers::response_handler::ResponseHandler;
use crate::router::plano_orchestrator::OrchestratorService;

/// Integration test that demonstrates the modular agent chat flow
/// This test shows how the three main components work together:
/// 1. AgentSelector - selects the appropriate agents based on orchestration
/// 2. PipelineProcessor - executes the agent pipeline
/// 3. ResponseHandler - handles response streaming
#[cfg(test)]
mod tests {
    use super::*;
    use common::configuration::{Agent, AgentFilterChain, Listener};

    fn create_test_orchestrator_service() -> Arc<OrchestratorService> {
        Arc::new(OrchestratorService::new(
            "http://localhost:8080".to_string(),
            "test-model".to_string(),
        ))
    }

    fn create_test_message(role: Role, content: &str) -> Message {
        Message {
            role,
            content: Some(MessageContent::Text(content.to_string())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    #[tokio::test]
    async fn test_modular_agent_chat_flow() {
        // Setup services
        let orchestrator_service = create_test_orchestrator_service();
        let agent_selector = AgentSelector::new(orchestrator_service);
        let mut pipeline_processor = PipelineProcessor::default();

        // Create test data
        let agents = vec![
            Agent {
                id: "filter-agent".to_string(),
                agent_type: Some("filter".to_string()),
                url: "http://localhost:8081".to_string(),
                tool: None,
                transport: None,
            },
            Agent {
                id: "terminal-agent".to_string(),
                agent_type: Some("terminal".to_string()),
                url: "http://localhost:8082".to_string(),
                tool: None,
                transport: None,
            },
        ];

        let agent_pipeline = AgentFilterChain {
            id: "terminal-agent".to_string(),
            filter_chain: Some(vec![
                "filter-agent".to_string(),
                "terminal-agent".to_string(),
            ]),
            description: Some("Test pipeline".to_string()),
            default: Some(true),
        };

        let listener = Listener {
            name: "test-listener".to_string(),
            agents: Some(vec![agent_pipeline.clone()]),
            port: 8080,
            router: None,
        };

        let listeners = vec![listener];
        let messages = vec![create_test_message(Role::User, "Hello world!")];

        // Test 1: Agent Selection
        let selected_listener = agent_selector
            .find_listener(Some("test-listener"), &listeners)
            .await;

        assert!(selected_listener.is_ok());
        let listener = selected_listener.unwrap();
        assert_eq!(listener.name, "test-listener");

        // Test 2: Agent Map Creation
        let agent_map = agent_selector.create_agent_map(&agents);
        assert_eq!(agent_map.len(), 2);
        assert!(agent_map.contains_key("filter-agent"));
        assert!(agent_map.contains_key("terminal-agent"));

        // Test 3: Pipeline Processing (empty filter chain for testing)
        let request = ChatCompletionsRequest {
            messages: messages.clone(),
            model: "test-model".to_string(),
            ..Default::default()
        };

        // Create a pipeline with empty filter chain to avoid network calls
        let test_pipeline = AgentFilterChain {
            id: "terminal-agent".to_string(),
            filter_chain: Some(vec![]), // Empty filter chain - no network calls needed
            description: None,
            default: None,
        };

        let headers = HeaderMap::new();
        let result = pipeline_processor
            .process_filter_chain(
                &request.messages,
                &test_pipeline,
                &agent_map,
                &headers,
                None,
                String::new(),
                String::new(),
            )
            .await;

        println!("Pipeline processing result: {:?}", result);

        assert!(result.is_ok());
        let processed_messages = result.unwrap();
        // With empty filter chain, should return the original messages unchanged
        assert_eq!(processed_messages.len(), 1);
        if let Some(MessageContent::Text(content)) = &processed_messages[0].content {
            assert_eq!(content, "Hello world!");
        } else {
            panic!("Expected text content");
        }

        // Test 4: Error Response Creation
        let error_response = ResponseHandler::create_bad_request("Test error");
        assert_eq!(error_response.status(), hyper::StatusCode::BAD_REQUEST);

        println!("✅ All modular components working correctly!");
    }

    #[tokio::test]
    async fn test_error_handling_flow() {
        let router_service = create_test_orchestrator_service();
        let agent_selector = AgentSelector::new(router_service);

        // Test listener not found
        let result = agent_selector.find_listener(Some("nonexistent"), &[]).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            AgentSelectionError::ListenerNotFound(_)
        ));

        // Test error response creation
        let error_response = ResponseHandler::create_internal_error("Pipeline failed");
        assert_eq!(
            error_response.status(),
            hyper::StatusCode::INTERNAL_SERVER_ERROR
        );

        println!("✅ Error handling working correctly!");
    }
}
