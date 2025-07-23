use miette::Diagnostic;
use thiserror::Error;

#[derive(Error, Diagnostic, Debug)]
pub enum DiscordError {
    #[error("Discord authentication failed")]
    #[diagnostic(
        code(pattern::discord::auth_failed),
        help("Check that your Discord bot token is valid and has not been regenerated")
    )]
    AuthenticationFailed {
        #[source]
        cause: serenity::Error,
        token_preview: String, // First/last few chars of token for debugging
    },

    #[error("Missing required intents")]
    #[diagnostic(
        code(pattern::discord::missing_intents),
        help("Enable the following intents in Discord Developer Portal: {}", required_intents.join(", "))
    )]
    MissingIntents {
        required_intents: Vec<String>,
        current_intents: Vec<String>,
        bot_id: Option<String>,
    },

    #[error("Channel not found")]
    #[diagnostic(
        code(pattern::discord::channel_not_found),
        help("Channel ID {channel_id} not found or bot doesn't have access")
    )]
    ChannelNotFound {
        channel_id: String,
        guild_id: Option<String>,
        accessible_channels: Vec<String>,
    },

    #[error("User not found")]
    #[diagnostic(
        code(pattern::discord::user_not_found),
        help("User ID {user_id} not found in accessible guilds")
    )]
    UserNotFound {
        user_id: String,
        searched_guilds: Vec<String>,
    },

    #[error("Permission denied")]
    #[diagnostic(
        code(pattern::discord::permission_denied),
        help("Bot lacks permission '{required_permission}' in {location}")
    )]
    PermissionDenied {
        required_permission: String,
        location: String, // e.g., "channel #general", "guild MyServer"
        current_permissions: Vec<String>,
        bot_role: Option<String>,
    },

    #[error("Message send failed")]
    #[diagnostic(
        code(pattern::discord::message_send_failed),
        help("Failed to send message to {destination}")
    )]
    MessageSendFailed {
        destination: String, // Channel name/ID or user mention
        message_length: usize,
        #[source]
        cause: serenity::Error,
        rate_limited: bool,
    },

    #[error("Command registration failed")]
    #[diagnostic(
        code(pattern::discord::command_registration_failed),
        help("Failed to register slash command '{command_name}'")
    )]
    CommandRegistrationFailed {
        command_name: String,
        #[source]
        cause: serenity::Error,
        existing_commands: Vec<String>,
    },

    #[error("Invalid command syntax")]
    #[diagnostic(
        code(pattern::discord::invalid_command_syntax),
        help("Command syntax error: {reason}")
    )]
    InvalidCommandSyntax {
        command: String,
        reason: String,
        expected_format: String,
        #[source_code]
        provided_input: String,
        #[label("error here")]
        error_span: (usize, usize),
    },

    #[error("Handler not found")]
    #[diagnostic(
        code(pattern::discord::handler_not_found),
        help("No handler registered for {handler_type} '{handler_name}'")
    )]
    HandlerNotFound {
        handler_type: String, // "command", "event", etc.
        handler_name: String,
        available_handlers: Vec<String>,
    },

    #[error("Rate limit exceeded")]
    #[diagnostic(
        code(pattern::discord::rate_limit_exceeded),
        help("Discord API rate limit hit. Retry after {retry_after_seconds} seconds")
    )]
    RateLimitExceeded {
        endpoint: String,
        retry_after_seconds: u64,
        requests_made: usize,
        rate_limit_scope: RateLimitScope,
    },

    #[error("Webhook error")]
    #[diagnostic(
        code(pattern::discord::webhook_error),
        help("Webhook operation failed for {webhook_url}")
    )]
    WebhookError {
        webhook_url: String,
        operation: String,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Interaction failed")]
    #[diagnostic(
        code(pattern::discord::interaction_failed),
        help("Failed to handle Discord interaction of type '{interaction_type}'")
    )]
    InteractionFailed {
        interaction_type: String,
        interaction_id: String,
        user_id: String,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
        responded: bool,
    },

    #[error("Voice connection failed")]
    #[diagnostic(
        code(pattern::discord::voice_connection_failed),
        help("Failed to establish voice connection to {channel_name}")
    )]
    VoiceConnectionFailed {
        channel_name: String,
        guild_id: String,
        #[source]
        cause: serenity::Error,
        voice_states: Vec<String>,
    },

    #[error("Embed build failed")]
    #[diagnostic(
        code(pattern::discord::embed_build_failed),
        help("Failed to build Discord embed: {reason}")
    )]
    EmbedBuildFailed {
        reason: String,
        field_count: usize,
        total_length: usize,
        limits_exceeded: Vec<EmbedLimit>,
    },

    #[error("Gateway connection lost")]
    #[diagnostic(
        code(pattern::discord::gateway_connection_lost),
        help("Lost connection to Discord gateway. Attempting reconnection...")
    )]
    GatewayConnectionLost {
        last_heartbeat: Option<std::time::SystemTime>,
        reconnect_attempts: usize,
        #[source]
        cause: serenity::Error,
    },

    #[error("Invalid bot configuration")]
    #[diagnostic(
        code(pattern::discord::invalid_bot_config),
        help("Bot configuration error: {issues}")
    )]
    InvalidBotConfiguration {
        issues: String,
        config_path: Option<String>,
        missing_fields: Vec<String>,
    },

    #[error("Message too long")]
    #[diagnostic(
        code(pattern::discord::message_too_long),
        help("Message length ({length} chars) exceeds Discord's limit of {limit} characters")
    )]
    MessageTooLong {
        length: usize,
        limit: usize,
        truncated_preview: String,
        suggestion: MessageSplitSuggestion,
    },

    #[error("Attachment error")]
    #[diagnostic(
        code(pattern::discord::attachment_error),
        help("Failed to process attachment '{filename}'")
    )]
    AttachmentError {
        filename: String,
        size_bytes: Option<usize>,
        mime_type: Option<String>,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Context extraction failed")]
    #[diagnostic(
        code(pattern::discord::context_extraction_failed),
        help("Failed to extract {context_type} from Discord message")
    )]
    ContextExtractionFailed {
        context_type: String, // "user", "channel", "guild", etc.
        message_id: String,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Routing failed")]
    #[diagnostic(
        code(pattern::discord::routing_failed),
        help("Failed to route message to appropriate handler")
    )]
    RoutingFailed {
        message_content: String,
        author_id: String,
        channel_type: String,
        #[source]
        cause: pattern_core::CoreError,
        attempted_routes: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum RateLimitScope {
    Global,
    Channel,
    Guild,
    User,
}

impl std::fmt::Display for RateLimitScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Global => write!(f, "global"),
            Self::Channel => write!(f, "per-channel"),
            Self::Guild => write!(f, "per-guild"),
            Self::User => write!(f, "per-user"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct EmbedLimit {
    pub field: String,
    pub current: usize,
    pub maximum: usize,
}

#[derive(Debug, Clone)]
pub enum MessageSplitSuggestion {
    SplitIntoMultiple { parts: usize },
    UseFile { filename: String },
    Truncate { safe_length: usize },
}

impl std::fmt::Display for MessageSplitSuggestion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SplitIntoMultiple { parts } => {
                write!(f, "Split message into {} parts", parts)
            }
            Self::UseFile { filename } => {
                write!(f, "Send as file attachment: {}", filename)
            }
            Self::Truncate { safe_length } => {
                write!(f, "Truncate to {} characters", safe_length)
            }
        }
    }
}

pub type Result<T> = std::result::Result<T, DiscordError>;

// Helper functions for creating common errors
impl DiscordError {
    pub fn auth_failed(cause: serenity::Error, token: &str) -> Self {
        // Show first 6 and last 4 characters of token for debugging
        let token_preview = if token.len() > 10 {
            format!("{}...{}", &token[..6], &token[token.len() - 4..])
        } else {
            "***".to_string()
        };

        Self::AuthenticationFailed {
            cause,
            token_preview,
        }
    }

    pub fn missing_permission(
        permission: impl Into<String>,
        location: impl Into<String>,
        current: Vec<String>,
    ) -> Self {
        Self::PermissionDenied {
            required_permission: permission.into(),
            location: location.into(),
            current_permissions: current,
            bot_role: None,
        }
    }

    pub fn message_too_long(content: &str) -> Self {
        const DISCORD_LIMIT: usize = 2000;
        let length = content.len();

        let suggestion = if length <= DISCORD_LIMIT * 3 {
            MessageSplitSuggestion::SplitIntoMultiple {
                parts: (length / DISCORD_LIMIT) + 1,
            }
        } else if length <= 8_000_000 {
            // Discord file size limit is 8MB
            MessageSplitSuggestion::UseFile {
                filename: "message.txt".to_string(),
            }
        } else {
            MessageSplitSuggestion::Truncate {
                safe_length: DISCORD_LIMIT - 100, // Leave room for truncation indicator
            }
        };

        let truncated = if content.len() > 100 {
            format!("{}...", &content[..100])
        } else {
            content.to_string()
        };

        Self::MessageTooLong {
            length,
            limit: DISCORD_LIMIT,
            truncated_preview: truncated,
            suggestion,
        }
    }

    pub fn invalid_command(
        command: impl Into<String>,
        input: impl Into<String>,
        reason: impl Into<String>,
        expected: impl Into<String>,
    ) -> Self {
        let input = input.into();
        let reason = reason.into();

        // Try to find where the error might be in the input
        let error_span = if let Some(pos) = input.find(' ') {
            (pos, input.len())
        } else {
            (0, input.len())
        };

        Self::InvalidCommandSyntax {
            command: command.into(),
            reason,
            expected_format: expected.into(),
            provided_input: input,
            error_span,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miette::Report;

    #[test]
    fn test_auth_error_hides_token() {
        let fake_error = serenity::Error::Other("test");
        let error = DiscordError::auth_failed(
            fake_error,
            "MTE2MzU5NzE0MjQ5NzI1NTQyNA.GqvKfH.verysecrettoken",
        );

        if let DiscordError::AuthenticationFailed { token_preview, .. } = &error {
            assert_eq!(token_preview, "MTE2Mz...oken");
            assert!(!token_preview.contains("secret"));
        }
    }

    #[test]
    fn test_message_too_long_suggestions() {
        // Test split suggestion
        let error = DiscordError::message_too_long(&"a".repeat(3500));
        if let DiscordError::MessageTooLong { suggestion, .. } = &error {
            matches!(
                suggestion,
                MessageSplitSuggestion::SplitIntoMultiple { parts: 2 }
            );
        }

        // Test file suggestion
        let error = DiscordError::message_too_long(&"a".repeat(10_000));
        if let DiscordError::MessageTooLong { suggestion, .. } = &error {
            matches!(suggestion, MessageSplitSuggestion::UseFile { .. });
        }
    }

    #[test]
    fn test_embed_limits() {
        let limits = vec![
            EmbedLimit {
                field: "title".to_string(),
                current: 300,
                maximum: 256,
            },
            EmbedLimit {
                field: "description".to_string(),
                current: 5000,
                maximum: 4096,
            },
        ];

        let error = DiscordError::EmbedBuildFailed {
            reason: "Multiple field limits exceeded".to_string(),
            field_count: 26,
            total_length: 7000,
            limits_exceeded: limits,
        };

        let report = Report::new(error);
        let output = format!("{:?}", report);
        assert!(output.contains("embed_build_failed"));
    }
}
