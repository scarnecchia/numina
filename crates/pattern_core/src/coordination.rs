use dashmap::DashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Agent, AgentId, Result};

/// A constellation represents a collection of agents working together for a specific user
#[derive(Debug)]
pub struct Constellation {
    pub id: String,
    pub owner_id: String,
    pub agents: DashMap<AgentId, Box<dyn Agent>>,
    pub groups: DashMap<String, AgentGroup>,
}

/// A group of agents that coordinate together
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentGroup {
    pub id: String,
    pub name: String,
    pub coordination_pattern: CoordinationPattern,
    pub agent_ids: Vec<AgentId>,
    pub metadata: serde_json::Value,
}

/// Defines how agents in a group coordinate their actions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CoordinationPattern {
    /// One agent leads, others follow
    Supervisor {
        leader_id: AgentId,
        delegation_rules: serde_json::Value,
    },

    /// Agents take turns in order
    RoundRobin {
        current_index: usize,
        skip_unavailable: bool,
    },

    /// Agents vote on decisions
    Voting {
        quorum: usize,
        voting_rules: VotingRules,
    },

    /// Sequential processing pipeline
    Pipeline {
        stages: Vec<PipelineStage>,
        parallel_stages: bool,
    },

    /// Dynamic selection based on context
    Dynamic {
        selector_name: String,
        selector_config: serde_json::Value,
    },

    /// Background monitoring with intervention triggers
    Sleeptime {
        check_interval: Duration,
        triggers: Vec<SleeptimeTrigger>,
        intervention_agent_id: AgentId,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VotingRules {
    pub voting_timeout: Duration,
    pub tie_breaker: TieBreaker,
    pub weight_by_expertise: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TieBreaker {
    Random,
    FirstVote,
    SpecificAgent(AgentId),
    NoDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStage {
    pub name: String,
    pub agent_ids: Vec<AgentId>,
    pub timeout: Duration,
    pub on_failure: StageFailureAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StageFailureAction {
    Skip,
    Retry { max_attempts: usize },
    Abort,
    Fallback { agent_id: AgentId },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SleeptimeTrigger {
    pub name: String,
    pub condition: TriggerCondition,
    pub priority: TriggerPriority,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TriggerCondition {
    TimeElapsed { duration: Duration },
    PatternDetected { pattern_name: String },
    ThresholdExceeded { metric: String, threshold: f64 },
    Custom { evaluator: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum TriggerPriority {
    Low,
    Medium,
    High,
    Critical,
}

/// Trait for implementing agent selection strategies
#[async_trait]
pub trait AgentSelector: Send + Sync {
    /// Select an agent based on the current context
    async fn select_agent(
        &self,
        agents: &[AgentId],
        context: &serde_json::Value,
    ) -> Result<AgentId>;

    /// Get the name of this selector
    fn name(&self) -> &str;
}

impl Constellation {
    /// Create a new constellation for a user
    pub fn new(owner_id: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            owner_id: owner_id.into(),
            agents: DashMap::new(),
            groups: DashMap::new(),
        }
    }

    /// Add an agent to the constellation
    pub fn add_agent(&mut self, agent_id: AgentId, agent: Box<dyn Agent>) {
        self.agents.insert(agent_id, agent);
    }

    /// Create a new agent group
    pub fn create_group(
        &mut self,
        name: impl Into<String>,
        pattern: CoordinationPattern,
        agent_ids: Vec<AgentId>,
    ) -> String {
        let group = AgentGroup {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            coordination_pattern: pattern,
            agent_ids,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        };

        let group_id = group.id.clone();
        self.groups.insert(group_id.clone(), group);
        group_id
    }
}
