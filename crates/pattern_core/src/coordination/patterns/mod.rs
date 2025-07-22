//! Coordination pattern implementations

mod dynamic;
mod pipeline;
mod round_robin;
mod sleeptime;
mod supervisor;
mod voting;

pub use dynamic::DynamicManager;
pub use pipeline::PipelineManager;
pub use round_robin::RoundRobinManager;
pub use sleeptime::SleeptimeManager;
pub use supervisor::SupervisorManager;
pub use voting::VotingManager;
