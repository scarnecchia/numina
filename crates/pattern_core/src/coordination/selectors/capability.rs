//! Capability-based agent selection

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use super::SelectionContext;
use crate::coordination::AgentSelector;
use crate::coordination::groups::AgentWithMembership;
use crate::{Result, agent::Agent};

/// Selects agents based on their capabilities
#[derive(Debug, Clone)]
pub struct CapabilitySelector;

#[async_trait]
impl AgentSelector for CapabilitySelector {
    async fn select_agents<'a>(
        &'a self,
        agents: &'a [AgentWithMembership<Arc<dyn Agent>>],
        _context: &SelectionContext,
        config: &HashMap<String, String>,
    ) -> Result<Vec<&'a AgentWithMembership<Arc<dyn Agent>>>> {
        // Get required capabilities from config
        let required_capabilities: Vec<String> = config
            .get("capabilities")
            .map(|s| s.split(',').map(|c| c.trim().to_string()).collect())
            .unwrap_or_default();

        // Get match mode (all or any)
        let require_all = config
            .get("require_all")
            .map(|s| s == "true")
            .unwrap_or(false);

        // Filter agents by capabilities
        let mut selected = Vec::new();

        for awm in agents {
            // Only consider active agents
            if !awm.membership.is_active {
                continue;
            }

            let matches = if require_all {
                // Agent must have all required capabilities
                required_capabilities
                    .iter()
                    .all(|req| awm.membership.capabilities.iter().any(|cap| cap == req))
            } else {
                // Agent must have at least one required capability
                required_capabilities.is_empty()
                    || required_capabilities
                        .iter()
                        .any(|req| awm.membership.capabilities.iter().any(|cap| cap == req))
            };

            if matches {
                selected.push(awm);
            }
        }

        // Limit results if max_agents is specified
        if let Some(max) = config
            .get("max_agents")
            .and_then(|s| s.parse::<usize>().ok())
        {
            selected.truncate(max);
        }

        Ok(selected)
    }

    fn name(&self) -> &str {
        "capability"
    }

    fn description(&self) -> &str {
        "Selects agents based on their capabilities matching requirements"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AgentId,
        coordination::{
            groups::GroupMembership,
            test_utils::test::{TestAgent, create_test_message},
            types::GroupMemberRole,
        },
    };
    use chrono::Utc;

    #[tokio::test]
    async fn test_capability_selector() {
        let selector = CapabilitySelector;

        let agent1_id = AgentId::generate();
        let agent2_id = AgentId::generate();
        let agent3_id = AgentId::generate();

        // Create agents with different capabilities
        let agents: Vec<AgentWithMembership<Arc<dyn crate::agent::Agent>>> = vec![
            AgentWithMembership {
                agent: Arc::new(TestAgent {
                    id: agent1_id.clone(),
                    name: "agent1".to_string(),
                }) as Arc<dyn crate::agent::Agent>,
                membership: GroupMembership {
                    joined_at: Utc::now(),
                    role: GroupMemberRole::Regular,
                    is_active: true,
                    capabilities: vec!["technical".to_string(), "coding".to_string()],
                },
            },
            AgentWithMembership {
                agent: Arc::new(TestAgent {
                    id: agent2_id.clone(),
                    name: "agent2".to_string(),
                }) as Arc<dyn crate::agent::Agent>,
                membership: GroupMembership {
                    joined_at: Utc::now(),
                    role: GroupMemberRole::Regular,
                    is_active: true,
                    capabilities: vec!["creative".to_string(), "writing".to_string()],
                },
            },
            AgentWithMembership {
                agent: Arc::new(TestAgent {
                    id: agent3_id.clone(),
                    name: "agent3".to_string(),
                }) as Arc<dyn crate::agent::Agent>,
                membership: GroupMembership {
                    joined_at: Utc::now(),
                    role: GroupMemberRole::Regular,
                    is_active: true,
                    capabilities: vec!["technical".to_string(), "analysis".to_string()],
                },
            },
        ];

        let context = SelectionContext {
            message: create_test_message("Need technical help"),
            recent_selections: vec![],
            available_agents: vec![], // Not used in new implementation
            agent_capabilities: HashMap::new(), // Not used in new implementation
        };

        // Select agents with 'technical' capability
        let mut config = HashMap::new();
        config.insert("capabilities".to_string(), "technical".to_string());

        let selected = selector
            .select_agents(&agents, &context, &config)
            .await
            .unwrap();
        assert_eq!(selected.len(), 2);
        let selected_ids: Vec<_> = selected.iter().map(|awm| awm.agent.id()).collect();
        assert!(selected_ids.contains(&agent1_id));
        assert!(selected_ids.contains(&agent3_id));
        assert!(!selected_ids.contains(&agent2_id));

        // Select agents with multiple capabilities (any match)
        config.insert("capabilities".to_string(), "creative,coding".to_string());
        config.insert("require_all".to_string(), "false".to_string());

        let selected = selector
            .select_agents(&agents, &context, &config)
            .await
            .unwrap();
        assert_eq!(selected.len(), 2);
        let selected_ids: Vec<_> = selected.iter().map(|awm| awm.agent.id()).collect();
        assert!(selected_ids.contains(&agent1_id)); // has coding
        assert!(selected_ids.contains(&agent2_id)); // has creative

        // Select agents with all capabilities
        config.insert("capabilities".to_string(), "technical,analysis".to_string());
        config.insert("require_all".to_string(), "true".to_string());

        let selected = selector
            .select_agents(&agents, &context, &config)
            .await
            .unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].agent.id(), agent3_id);
    }
}
