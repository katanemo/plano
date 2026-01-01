use std::collections::HashMap;
use std::sync::Arc;

use common::configuration::{
    Agent, AgentFilterChain, AgentUsagePreference, Listener, OrchestrationPreference,
};
use hermesllm::apis::openai::Message;
use tracing::{debug, warn};

use crate::router::plano_orchestrator::OrchestratorService;

/// Errors that can occur during agent selection
#[derive(Debug, thiserror::Error)]
pub enum AgentSelectionError {
    #[error("Listener not found for name: {0}")]
    ListenerNotFound(String),
    #[error("No agents configured for listener: {0}")]
    NoAgentsConfigured(String),
    #[error("Default agent not found for listener: {0}")]
    DefaultAgentNotFound(String),
    #[error("MCP client error: {0}")]
    McpError(String),
    #[error("Orchestration service error: {0}")]
    OrchestrationError(String),
}

/// Service for selecting agents based on orchestration preferences and listener configuration
pub struct AgentSelector {
    orchestrator_service: Arc<OrchestratorService>,
}

impl AgentSelector {
    pub fn new(orchestrator_service: Arc<OrchestratorService>) -> Self {
        Self {
            orchestrator_service,
        }
    }

    /// Find listener by name from the request headers
    pub async fn find_listener(
        &self,
        listener_name: Option<&str>,
        listeners: &[common::configuration::Listener],
    ) -> Result<Listener, AgentSelectionError> {
        let listener = listeners
            .iter()
            .find(|l| listener_name.map(|name| l.name == name).unwrap_or(false))
            .cloned()
            .ok_or_else(|| {
                AgentSelectionError::ListenerNotFound(
                    listener_name.unwrap_or("unknown").to_string(),
                )
            })?;

        Ok(listener)
    }

    /// Create agent name to agent mapping for efficient lookup
    pub fn create_agent_map(&self, agents: &[Agent]) -> HashMap<String, Agent> {
        agents
            .iter()
            .map(|agent| (agent.id.clone(), agent.clone()))
            .collect()
    }

    /// Get the default agent or the first agent if no default is specified
    fn get_default_agent(
        &self,
        agents: &[AgentFilterChain],
        listener_name: &str,
    ) -> Result<AgentFilterChain, AgentSelectionError> {
        agents
            .iter()
            .find(|a| a.default.unwrap_or(false))
            .cloned()
            .or_else(|| {
                warn!(
                    "No default agent found, routing request to first agent: {}",
                    agents[0].id
                );
                Some(agents[0].clone())
            })
            .ok_or_else(|| AgentSelectionError::DefaultAgentNotFound(listener_name.to_string()))
    }

    /// Convert agent descriptions to orchestration preferences
    async fn convert_agent_description_to_orchestration_preferences(
        &self,
        agents: &[AgentFilterChain],
    ) -> Vec<AgentUsagePreference> {
        let mut preferences = Vec::new();

        for agent_chain in agents {
            preferences.push(AgentUsagePreference {
                model: agent_chain.id.clone(),
                orchestration_preferences: vec![OrchestrationPreference {
                    name: agent_chain.id.clone(),
                    description: agent_chain.description.clone().unwrap_or_default(),
                }],
            });
        }

        preferences
    }

    /// Select multiple agents using orchestration
    pub async fn select_agents(
        &self,
        messages: &[Message],
        listener: &Listener,
        trace_parent: Option<String>,
    ) -> Result<Vec<AgentFilterChain>, AgentSelectionError> {
        let agents = listener
            .agents
            .as_ref()
            .ok_or_else(|| AgentSelectionError::NoAgentsConfigured(listener.name.clone()))?;

        // If only one agent, skip orchestration
        if agents.len() == 1 {
            debug!("Only one agent available, skipping orchestration");
            return Ok(vec![agents[0].clone()]);
        }

        let usage_preferences = self
            .convert_agent_description_to_orchestration_preferences(agents)
            .await;
        debug!(
            "Agents usage preferences for orchestration: {}",
            serde_json::to_string(&usage_preferences).unwrap_or_default()
        );

        match self
            .orchestrator_service
            .determine_orchestration(messages, trace_parent, Some(usage_preferences))
            .await
        {
            Ok(Some(routes)) => {
                debug!("Determined {} agent(s) via orchestration", routes.len());
                let mut selected_agents = Vec::new();

                for (route_name, agent_name) in routes {
                    debug!("Processing route: {}, agent: {}", route_name, agent_name);
                    let selected_agent = agents
                        .iter()
                        .find(|a| a.id == agent_name)
                        .cloned()
                        .ok_or_else(|| {
                            AgentSelectionError::OrchestrationError(format!(
                                "Selected agent '{}' not found in listener agents",
                                agent_name
                            ))
                        })?;
                    selected_agents.push(selected_agent);
                }

                if selected_agents.is_empty() {
                    debug!("No agents determined using orchestration, using default agent");
                    Ok(vec![self.get_default_agent(agents, &listener.name)?])
                } else {
                    Ok(selected_agents)
                }
            }
            Ok(None) => {
                debug!("No agents determined using orchestration, using default agent");
                Ok(vec![self.get_default_agent(agents, &listener.name)?])
            }
            Err(err) => Err(AgentSelectionError::OrchestrationError(err.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::configuration::{AgentFilterChain, Listener};

    fn create_test_orchestrator_service() -> Arc<OrchestratorService> {
        Arc::new(OrchestratorService::new(
            "http://localhost:8080".to_string(),
            "test-model".to_string(),
        ))
    }

    fn create_test_agent(name: &str, description: &str, is_default: bool) -> AgentFilterChain {
        AgentFilterChain {
            id: name.to_string(),
            description: Some(description.to_string()),
            default: Some(is_default),
            filter_chain: Some(vec![name.to_string()]),
        }
    }

    fn create_test_listener(name: &str, agents: Vec<AgentFilterChain>) -> Listener {
        Listener {
            name: name.to_string(),
            agents: Some(agents),
            port: 8080,
            router: None,
        }
    }

    fn create_test_agent_struct(name: &str) -> Agent {
        Agent {
            id: name.to_string(),
            agent_type: Some("test".to_string()),
            url: "http://localhost:8080".to_string(),
            tool: None,
            transport: None,
        }
    }

    #[tokio::test]
    async fn test_find_listener_success() {
        let orchestrator_service = create_test_orchestrator_service();
        let selector = AgentSelector::new(orchestrator_service);

        let listener1 = create_test_listener("test-listener", vec![]);
        let listener2 = create_test_listener("other-listener", vec![]);
        let listeners = vec![listener1.clone(), listener2];

        let result = selector
            .find_listener(Some("test-listener"), &listeners)
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap().name, "test-listener");
    }

    #[tokio::test]
    async fn test_find_listener_not_found() {
        let orchestrator_service = create_test_orchestrator_service();
        let selector = AgentSelector::new(orchestrator_service);

        let listeners = vec![create_test_listener("other-listener", vec![])];

        let result = selector
            .find_listener(Some("nonexistent"), &listeners)
            .await;

        assert!(result.is_err());
        matches!(
            result.unwrap_err(),
            AgentSelectionError::ListenerNotFound(_)
        );
    }

    #[test]
    fn test_create_agent_map() {
        let orchestrator_service = create_test_orchestrator_service();
        let selector = AgentSelector::new(orchestrator_service);

        let agents = vec![
            create_test_agent_struct("agent1"),
            create_test_agent_struct("agent2"),
        ];

        let agent_map = selector.create_agent_map(&agents);

        assert_eq!(agent_map.len(), 2);
        assert!(agent_map.contains_key("agent1"));
        assert!(agent_map.contains_key("agent2"));
    }

    #[test]
    fn test_get_default_agent() {
        let orchestrator_service = create_test_orchestrator_service();
        let selector = AgentSelector::new(orchestrator_service);

        let agents = vec![
            create_test_agent("agent1", "First agent", false),
            create_test_agent("agent2", "Default agent", true),
            create_test_agent("agent3", "Third agent", false),
        ];

        let result = selector.get_default_agent(&agents, "test-listener");

        assert!(result.is_ok());
        assert_eq!(result.unwrap().id, "agent2");
    }

    #[test]
    fn test_get_default_agent_fallback_to_first() {
        let orchestrator_service = create_test_orchestrator_service();
        let selector = AgentSelector::new(orchestrator_service);

        let agents = vec![
            create_test_agent("agent1", "First agent", false),
            create_test_agent("agent2", "Second agent", false),
        ];

        let result = selector.get_default_agent(&agents, "test-listener");

        assert!(result.is_ok());
        assert_eq!(result.unwrap().id, "agent1");
    }
}
