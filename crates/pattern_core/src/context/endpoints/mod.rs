//! Message delivery endpoints for routing agent messages to various destinations

mod group;

pub use group::GroupEndpoint;

// Re-export the trait from message_router
pub use super::message_router::{MessageEndpoint, MessageOrigin};
