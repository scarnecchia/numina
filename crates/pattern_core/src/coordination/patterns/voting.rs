//! Voting coordination pattern implementation

use async_trait::async_trait;
use chrono::{Duration, Utc};
use std::{collections::HashMap, sync::Arc};
use uuid::Uuid;

use crate::{
    CoreError, Result,
    agent::Agent,
    coordination::{
        groups::{
            AgentResponse, AgentWithMembership, GroupManager, GroupResponse, GroupResponseEvent,
        },
        types::{
            CoordinationPattern, GroupState, TieBreaker, Vote, VoteOption, VotingProposal,
            VotingRules, VotingSession,
        },
        utils::text_response,
    },
    message::Message,
};

#[derive(Clone)]
pub struct VotingManager;

#[async_trait]
impl GroupManager for VotingManager {
    async fn route_message(
        &self,
        group: &crate::coordination::groups::AgentGroup,
        agents: &[AgentWithMembership<Arc<dyn Agent>>],
        message: Message,
    ) -> Result<Box<dyn futures::Stream<Item = GroupResponseEvent> + Send + Unpin>> {
        use tokio_stream::wrappers::ReceiverStream;
        let (tx, rx) = tokio::sync::mpsc::channel(100);

        let start_time = std::time::Instant::now();
        let group_id = group.id.clone();
        let _group_name = group.name.clone();

        // Do the full voting operation synchronously first
        let result = self.do_voting(group, agents, message).await;

        // Then send the result as a single Complete event
        tokio::spawn(async move {
            match result {
                Ok((agent_responses, state_changes)) => {
                    let _ = tx
                        .send(GroupResponseEvent::Complete {
                            group_id,
                            pattern: "voting".to_string(),
                            execution_time: start_time.elapsed(),
                            agent_responses,
                            state_changes,
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx
                        .send(GroupResponseEvent::Error {
                            agent_id: None,
                            message: e.to_string(),
                            recoverable: false,
                        })
                        .await;
                }
            }
        });

        Ok(Box::new(ReceiverStream::new(rx)))
    }

    async fn update_state(
        &self,
        _current_state: &GroupState,
        response: &GroupResponse,
    ) -> Result<Option<GroupState>> {
        // State is already updated in route_message for voting
        Ok(response.state_changes.clone())
    }
}

impl VotingManager {
    async fn do_voting(
        &self,
        group: &crate::coordination::groups::AgentGroup,
        agents: &[AgentWithMembership<Arc<dyn Agent>>],
        message: Message,
    ) -> Result<(Vec<AgentResponse>, Option<GroupState>)> {
        // Extract voting config
        let (quorum, voting_rules) = match &group.coordination_pattern {
            CoordinationPattern::Voting {
                quorum,
                voting_rules,
            } => (*quorum, voting_rules),
            _ => {
                return Err(CoreError::AgentGroupError {
                    group_name: group.name.clone(),
                    operation: "route_message".to_string(),
                    cause: "Invalid pattern for VotingManager".to_string(),
                });
            }
        };

        // Get active agents
        let active_agents: Vec<_> = agents
            .iter()
            .filter(|awm| awm.membership.is_active)
            .collect();

        if active_agents.is_empty() {
            return Err(CoreError::AgentGroupError {
                group_name: group.name.clone(),
                operation: "voting".to_string(),
                cause: "No active agents in voting group".to_string(),
            });
        }

        // Call all agents to vote
        let mut responses = Vec::new();
        let mut session = VotingSession {
            id: Uuid::new_v4(),
            proposal: self.create_proposal_from_message(&message),
            votes: HashMap::new(),
            started_at: Utc::now(),
            deadline: Utc::now()
                + Duration::from_std(voting_rules.voting_timeout)
                    .unwrap_or(Duration::seconds(30)),
        };

        // Call each agent to get their response/vote
        for awm in &active_agents {
            let agent_id = awm.agent.as_ref().id();

            // Process the voting message with the agent
            match awm
                .agent
                .clone()
                .process_message(message.clone())
                .await
            {
                Ok(response) => {
                    // Record agent response
                    responses.push(AgentResponse {
                        agent_id: agent_id.clone(),
                        response: response.clone(),
                        responded_at: Utc::now(),
                    });

                    // Extract text from response content
                    let response_text = response.content
                        .iter()
                        .filter_map(|c| c.text())
                        .collect::<Vec<_>>()
                        .join(" ");

                    // Parse response to determine vote
                    // Default: first option if no clear vote indicated
                    let vote_option = self.extract_vote_from_response(&response_text, &session.proposal)
                        .unwrap_or_else(|| {
                            session.proposal.options.first()
                                .map(|opt| opt.id.clone())
                                .unwrap_or_else(|| "option1".to_string())
                        });

                    // Record the vote
                    let vote = Vote {
                        option_id: vote_option,
                        weight: 1.0,
                        reasoning: Some(response_text.clone()),
                        timestamp: Utc::now(),
                    };
                    session.votes.insert(agent_id.clone(), vote);
                }
                Err(e) => {
                    tracing::warn!("Agent {} failed to process voting message: {}", agent_id, e);
                    // Continue with other agents on failure
                }
            }
        }

        // Check if we have quorum
        let has_quorum = session.votes.len() >= quorum;

        let new_state = if has_quorum || session.votes.len() == active_agents.len() {
            // Tally votes and determine winner
            let result = self.tally_votes(&session, voting_rules)?;

            // Add voting result summary
            responses.push(AgentResponse {
                agent_id: active_agents[0].agent.as_ref().id(), // Group response
                response: text_response(format!(
                    "[Voting Complete] Winner: {}. Votes: {}/{}",
                    result,
                    session.votes.len(),
                    active_agents.len()
                )),
                responded_at: Utc::now(),
            });

            Some(GroupState::Voting {
                active_session: None,
            })
        } else {
            // Not enough votes - keep session active
            responses.push(AgentResponse {
                agent_id: active_agents[0].agent.as_ref().id(),
                response: text_response(format!(
                    "[Voting] {}/{} agents have voted (quorum: {})",
                    session.votes.len(),
                    active_agents.len(),
                    quorum
                )),
                responded_at: Utc::now(),
            });

            Some(GroupState::Voting {
                active_session: Some(session),
            })
        };

        Ok((responses, new_state))
    }

    fn extract_vote_from_response(&self, response: &str, proposal: &VotingProposal) -> Option<String> {
        let lower = response.to_lowercase();

        // Try to match option descriptions in the response
        for option in &proposal.options {
            if lower.contains(&option.description.to_lowercase()) {
                return Some(option.id.clone());
            }
        }

        None
    }

    fn create_proposal_from_message(&self, message: &Message) -> VotingProposal {
        Self::create_proposal_from_message_impl(message)
    }

    fn create_proposal_from_message_impl(message: &Message) -> VotingProposal {
        // In a real implementation, this would parse the message to create options
        VotingProposal {
            content: format!("Proposal based on: {:?}", message.content),
            options: vec![
                VoteOption {
                    id: "option1".to_string(),
                    description: "Approve".to_string(),
                },
                VoteOption {
                    id: "option2".to_string(),
                    description: "Reject".to_string(),
                },
                VoteOption {
                    id: "option3".to_string(),
                    description: "Abstain".to_string(),
                },
            ],
            metadata: HashMap::new(),
        }
    }

    fn tally_votes(&self, session: &VotingSession, rules: &VotingRules) -> Result<String> {
        Self::tally_votes_impl(session, rules)
    }

    fn tally_votes_impl(session: &VotingSession, rules: &VotingRules) -> Result<String> {
        // Count votes by option
        let mut vote_counts: HashMap<String, f32> = HashMap::new();

        for vote in session.votes.values() {
            *vote_counts.entry(vote.option_id.clone()).or_insert(0.0) += vote.weight;
        }

        // Find the option(s) with the most votes
        let max_votes = vote_counts.values().cloned().fold(0.0, f32::max);
        let winners: Vec<_> = vote_counts
            .iter()
            .filter(|(_, count)| **count == max_votes)
            .map(|(option_id, _)| option_id.clone())
            .collect();

        if winners.len() == 1 {
            // Clear winner
            Ok(winners[0].clone())
        } else {
            // Tie - use tie breaker
            match &rules.tie_breaker {
                TieBreaker::Random => {
                    let mut rng = rand::rng();
                    let index = rand::Rng::random_range(&mut rng, 0..winners.len());
                    winners
                        .get(index)
                        .cloned()
                        .ok_or_else(|| CoreError::AgentGroupError {
                            group_name: "voting".to_string(),
                            operation: "tie_breaker".to_string(),
                            cause: "No winners to choose from".to_string(),
                        })
                }
                TieBreaker::FirstVote => {
                    // Find which tied option got its first vote earliest
                    let mut earliest_vote = None;
                    let mut winning_option = None;

                    for vote in session.votes.values() {
                        if winners.contains(&vote.option_id) {
                            if earliest_vote.is_none() || vote.timestamp < earliest_vote.unwrap() {
                                earliest_vote = Some(vote.timestamp);
                                winning_option = Some(vote.option_id.clone());
                            }
                        }
                    }

                    winning_option.ok_or_else(|| CoreError::AgentGroupError {
                        group_name: "voting".to_string(),
                        operation: "tie_breaker".to_string(),
                        cause: "Could not determine first vote".to_string(),
                    })
                }
                TieBreaker::SpecificAgent(agent_id) => {
                    // Find what the specific agent voted for
                    session
                        .votes
                        .get(agent_id)
                        .map(|vote| vote.option_id.clone())
                        .ok_or_else(|| CoreError::AgentGroupError {
                            group_name: "voting".to_string(),
                            operation: "tie_breaker".to_string(),
                            cause: format!("Tie-breaker agent {} did not vote", agent_id),
                        })
                }
                TieBreaker::NoDecision => Err(CoreError::AgentGroupError {
                    group_name: "voting".to_string(),
                    operation: "tie_breaker".to_string(),
                    cause: "Voting resulted in a tie with no tie-breaker".to_string(),
                }),
            }
        }
    }
}
