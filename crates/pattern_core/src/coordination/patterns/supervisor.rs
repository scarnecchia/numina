//! Supervisor coordination pattern implementation

use chrono::Utc;
use std::collections::HashMap;

use crate::{
    AgentId, CoreError, Result,
    agent::Agent,
    coordination::{
        groups::{AgentResponse, AgentWithMembership, GroupManager, GroupResponse},
        types::{CoordinationPattern, DelegationStrategy, FallbackBehavior, GroupState},
        utils::text_response,
    },
    message::Message,
};

pub struct SupervisorManager;

impl GroupManager for SupervisorManager {
    async fn route_message<A>(
        &self,
        group: &crate::coordination::groups::AgentGroup,
        agents: &[AgentWithMembership<impl AsRef<A>>],
        message: Message,
    ) -> Result<GroupResponse>
    where
        A: Agent,
    {
        let start_time = std::time::Instant::now();

        // Extract supervisor config
        let (leader_id, delegation_rules) = match &group.coordination_pattern {
            CoordinationPattern::Supervisor {
                leader_id,
                delegation_rules,
            } => (leader_id, delegation_rules),
            _ => {
                return Err(CoreError::AgentGroupError {
                    group_name: group.name.clone(),
                    operation: "route_message".to_string(),
                    cause: Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "Invalid pattern for SupervisorManager",
                    )),
                });
            }
        };

        // Extract current delegations from state
        let current_delegations = match &group.state {
            GroupState::Supervisor {
                current_delegations,
            } => current_delegations.clone(),
            _ => HashMap::new(),
        };

        // Find the leader agent
        let leader = agents
            .iter()
            .find(|awm| awm.agent.as_ref().id() == *leader_id)
            .ok_or_else(|| CoreError::agent_not_found(leader_id.to_string()))?;

        // Decide if leader should delegate
        let should_delegate = self.should_delegate(
            &message,
            &current_delegations,
            leader_id,
            delegation_rules.max_delegations_per_agent,
        );

        let responses = if should_delegate {
            // Select delegate based on strategy
            let delegate = self.select_delegate(
                agents,
                leader_id,
                &delegation_rules.delegation_strategy,
                &current_delegations,
                delegation_rules.max_delegations_per_agent,
            )?;

            if let Some(delegate_awm) = delegate {
                // Delegate handles the message
                let agent_response = delegate_awm
                    .agent
                    .as_ref()
                    .process_message(message.clone())
                    .await?;
                vec![AgentResponse {
                    agent_id: delegate_awm.agent.as_ref().id(),
                    response: agent_response,
                    responded_at: Utc::now(),
                }]
            } else {
                // No delegate available, use fallback behavior
                match &delegation_rules.fallback_behavior {
                    FallbackBehavior::HandleSelf => {
                        let agent_response = leader
                            .agent
                            .as_ref()
                            .process_message(message.clone())
                            .await?;
                        vec![AgentResponse {
                            agent_id: leader_id.clone(),
                            response: agent_response,
                            responded_at: Utc::now(),
                        }]
                    }
                    FallbackBehavior::Queue => {
                        vec![AgentResponse {
                            agent_id: leader_id.clone(),
                            response: text_response(
                                "[Supervisor] Message queued for later processing",
                            ),
                            responded_at: Utc::now(),
                        }]
                    }
                    FallbackBehavior::Fail => {
                        return Err(CoreError::AgentGroupError {
                            group_name: group.name.clone(),
                            operation: "delegate".to_string(),
                            cause: Box::new(std::io::Error::new(
                                std::io::ErrorKind::Other,
                                "No delegates available and fallback is set to fail",
                            )),
                        });
                    }
                }
            }
        } else {
            // Leader handles directly
            let agent_response = leader
                .agent
                .as_ref()
                .process_message(message.clone())
                .await?;
            vec![AgentResponse {
                agent_id: leader_id.clone(),
                response: agent_response,
                responded_at: Utc::now(),
            }]
        };

        // Update state with new delegation counts
        let new_state = if should_delegate {
            let mut new_delegations = current_delegations.clone();
            if let Some(delegate_id) = responses.first().map(|r| &r.agent_id) {
                *new_delegations.entry(delegate_id.clone()).or_insert(0) += 1;
            }
            Some(GroupState::Supervisor {
                current_delegations: new_delegations,
            })
        } else {
            None
        };

        Ok(GroupResponse {
            group_id: group.id.clone(),
            pattern: "supervisor".to_string(),
            responses,
            execution_time: start_time.elapsed(),
            state_changes: new_state,
        })
    }

    async fn update_state(
        &self,
        _current_state: &GroupState,
        response: &GroupResponse,
    ) -> Result<Option<GroupState>> {
        // State is already updated in route_message for supervisor pattern
        Ok(response.state_changes.clone())
    }
}

impl SupervisorManager {
    fn should_delegate(
        &self,
        _message: &Message,
        current_delegations: &HashMap<AgentId, usize>,
        leader_id: &AgentId,
        max_delegations: Option<usize>,
    ) -> bool {
        // Simple heuristic: delegate if leader has too many active delegations
        if let Some(max) = max_delegations {
            let leader_count = current_delegations.get(leader_id).copied().unwrap_or(0);
            leader_count >= max
        } else {
            // Could add more sophisticated logic here based on message content
            false
        }
    }

    fn select_delegate<'a, A>(
        &self,
        agents: &'a [AgentWithMembership<impl AsRef<A>>],
        leader_id: &AgentId,
        strategy: &DelegationStrategy,
        current_delegations: &HashMap<AgentId, usize>,
        max_delegations: Option<usize>,
    ) -> Result<Option<&'a AgentWithMembership<impl AsRef<A>>>>
    where
        A: Agent,
    {
        // Filter out leader and unavailable agents
        let available_agents: Vec<_> = agents
            .iter()
            .filter(|awm| {
                let agent_id = awm.agent.as_ref().id();
                agent_id != *leader_id
                    && awm.membership.is_active
                    && self.can_accept_delegation(&agent_id, current_delegations, max_delegations)
            })
            .collect();

        if available_agents.is_empty() {
            return Ok(None);
        }

        match strategy {
            DelegationStrategy::RoundRobin => {
                // Simple round-robin: pick the agent with fewest delegations
                Ok(available_agents.into_iter().min_by_key(|&awm| {
                    current_delegations
                        .get(&awm.agent.as_ref().id())
                        .copied()
                        .unwrap_or(0)
                }))
            }
            DelegationStrategy::LeastBusy => {
                // Same as round-robin for now (would check actual workload in real impl)
                Ok(available_agents.into_iter().min_by_key(|&awm| {
                    current_delegations
                        .get(&awm.agent.as_ref().id())
                        .copied()
                        .unwrap_or(0)
                }))
            }
            DelegationStrategy::Capability => {
                // Check capabilities stored in membership
                Ok(available_agents
                    .into_iter()
                    .find(|&awm| !awm.membership.capabilities.is_empty()))
            }
            DelegationStrategy::Random => {
                let mut rng = rand::rng();
                let index = rand::Rng::random_range(&mut rng, 0..available_agents.len());
                Ok(available_agents.get(index).copied())
            }
        }
    }

    fn can_accept_delegation(
        &self,
        agent_id: &AgentId,
        current_delegations: &HashMap<AgentId, usize>,
        max_delegations: Option<usize>,
    ) -> bool {
        if let Some(max) = max_delegations {
            let current = current_delegations.get(agent_id).copied().unwrap_or(0);
            current < max
        } else {
            true
        }
    }
}
