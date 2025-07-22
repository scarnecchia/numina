//! Agent selection strategies for dynamic coordination

use std::collections::HashMap;

use super::{groups::AgentWithMembership, types::SelectionContext};
use crate::{Result, agent::Agent};

mod capability;
mod load_balancing;
mod random;

pub use capability::CapabilitySelector;
pub use load_balancing::LoadBalancingSelector;
pub use random::RandomSelector;

/// Enum of all available agent selectors
#[derive(Debug, Clone)]
pub enum AgentSelector {
    Random(RandomSelector),
    Capability(CapabilitySelector),
    LoadBalancing(LoadBalancingSelector),
}

impl AgentSelector {
    /// Select one or more agents based on the context and configuration
    pub async fn select_agents<'a, A, T>(
        &self,
        agents: &'a [AgentWithMembership<T>],
        context: &SelectionContext,
        config: &HashMap<String, String>,
    ) -> Result<Vec<&'a AgentWithMembership<T>>>
    where
        A: Agent,
        T: AsRef<A>,
    {
        match self {
            AgentSelector::Random(selector) => {
                selector.select_agents(agents, context, config).await
            }
            AgentSelector::Capability(selector) => {
                selector.select_agents(agents, context, config).await
            }
            AgentSelector::LoadBalancing(selector) => {
                selector.select_agents(agents, context, config).await
            }
        }
    }

    /// Human-readable name for this selector
    pub fn name(&self) -> &str {
        match self {
            AgentSelector::Random(selector) => selector.name(),
            AgentSelector::Capability(selector) => selector.name(),
            AgentSelector::LoadBalancing(selector) => selector.name(),
        }
    }

    /// Description of how this selector works
    pub fn description(&self) -> &str {
        match self {
            AgentSelector::Random(selector) => selector.description(),
            AgentSelector::Capability(selector) => selector.description(),
            AgentSelector::LoadBalancing(selector) => selector.description(),
        }
    }
}

/// Registry for agent selectors
pub trait SelectorRegistry: Send + Sync {
    /// Get a selector by name
    fn get(&self, name: &str) -> Option<&AgentSelector>;

    /// Register a new selector
    fn register(&mut self, name: String, selector: AgentSelector);

    /// List all available selectors
    fn list(&self) -> Vec<&str>;
}

/// Default implementation of SelectorRegistry
pub struct DefaultSelectorRegistry {
    selectors: HashMap<String, AgentSelector>,
}

impl DefaultSelectorRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            selectors: HashMap::new(),
        };

        // Register default selectors
        registry.register("random".to_string(), AgentSelector::Random(RandomSelector));
        registry.register(
            "capability".to_string(),
            AgentSelector::Capability(CapabilitySelector),
        );
        registry.register(
            "load_balancing".to_string(),
            AgentSelector::LoadBalancing(LoadBalancingSelector),
        );

        registry
    }
}

impl SelectorRegistry for DefaultSelectorRegistry {
    fn get(&self, name: &str) -> Option<&AgentSelector> {
        self.selectors.get(name)
    }

    fn register(&mut self, name: String, selector: AgentSelector) {
        self.selectors.insert(name, selector);
    }

    fn list(&self) -> Vec<&str> {
        self.selectors.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for DefaultSelectorRegistry {
    fn default() -> Self {
        Self::new()
    }
}
