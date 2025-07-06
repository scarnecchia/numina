//! Pattern ND - Neurodivergent Support Tools
//!
//! This crate provides ADHD-specific tools, agent personalities,
//! and cognitive support features tailored for neurodivergent users.

pub mod agents;
pub mod sleeptime;
pub mod tools;

pub use agents::{ADHDAgent, AgentPersonality};
pub use sleeptime::{SleeptimeMonitor, SleeptimeTrigger};
pub use tools::{ADHDTool, EnergyState, TaskBreakdown};

/// ADHD-specific agent types
pub mod agent_types {
    pub const PATTERN: &str = "pattern";
    pub const ENTROPY: &str = "entropy";
    pub const FLUX: &str = "flux";
    pub const ARCHIVE: &str = "archive";
    pub const MOMENTUM: &str = "momentum";
    pub const ANCHOR: &str = "anchor";
}

/// Re-export commonly used types
pub mod prelude {
    pub use crate::{
        agent_types, ADHDAgent, ADHDTool, AgentPersonality, EnergyState, SleeptimeMonitor,
        SleeptimeTrigger, TaskBreakdown,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        // Basic smoke test
        assert_eq!(2 + 2, 4);
    }
}
