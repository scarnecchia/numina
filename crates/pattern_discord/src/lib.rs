//! Pattern Discord - Discord Bot Integration
//!
//! This crate provides Discord bot functionality for Pattern,
//! enabling natural language interaction with the multi-agent system.

pub mod bot;
pub mod commands;
pub mod context;
pub mod endpoints;
pub mod error;
pub mod routing;

pub use bot::{DiscordBot, DiscordBotConfig};
pub use commands::{Command, CommandHandler, SlashCommand};
pub use context::{DiscordContext, MessageContext, UserContext};
pub use error::{DiscordError, Result};
pub use routing::{MessageRouter, RoutingStrategy};

// Re-export serenity for convenience
pub use serenity;

/// Discord-specific configuration
pub mod config {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct DiscordConfig {
        pub token: String,
        pub prefix: String,
        pub allowed_channels: Option<Vec<String>>,
        pub admin_users: Option<Vec<String>>,
    }

    impl Default for DiscordConfig {
        fn default() -> Self {
            Self {
                token: String::new(),
                prefix: "!".to_string(),
                allowed_channels: None,
                admin_users: None,
            }
        }
    }
}

/// Re-export commonly used types
pub mod prelude {
    pub use crate::{
        Command, CommandHandler, DiscordBot, DiscordBotConfig, DiscordContext, DiscordError,
        MessageContext, MessageRouter, Result, RoutingStrategy, SlashCommand, UserContext,
        config::DiscordConfig,
    };
}

#[cfg(test)]
mod tests {

    #[test]
    fn it_works() {
        // Basic smoke test
        assert_eq!(2 + 2, 4);
    }
}
