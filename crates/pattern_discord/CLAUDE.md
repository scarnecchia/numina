# CLAUDE.md - Pattern Discord

This crate provides Discord bot integration for the Pattern system, enabling multi-agent support through Discord's chat interface.

## Core Principles

- **Natural Conversation Flow**: Bot should feel like chatting with a helpful friend, not a command interface
- **Context Awareness**: Understand channel vs DM contexts, user relationships, and conversation history
- **Non-Intrusive**: Support without spamming or overwhelming channels
- **Privacy First**: DM content never leaks to public channels
- **Graceful Degradation**: Handle Discord API issues without losing user data

## Architecture Overview

### Key Components

1. **Bot Core** (`bot.rs`)
   - Serenity framework integration
   - Event handling (messages, reactions, joins)
   - Command routing (slash and natural language)
   - Connection resilience

2. **Agent Routing** (`routing.rs`)
   - Message analysis for agent selection
   - Group-based routing (main, crisis, planning, memory)
   - DM vs channel context handling
   - Multi-agent conversation coordination

3. **Context Management** (`context.rs`)
   - User identity tracking
   - Channel history access
   - Conversation threading
   - Privacy boundaries

4. **Command System** (`commands/`)
   - Slash commands for explicit interactions
   - Natural language command detection
   - Help system that's actually helpful
   - Debug commands for troubleshooting

## Discord-Specific Patterns

### Message Handling
```rust
// Always check context
match message.channel_id.to_channel(&ctx).await? {
    Channel::Private(_) => handle_dm(message),
    Channel::Guild(channel) => handle_guild_message(message, channel),
    _ => Ok(()), // Ignore other channel types
}

// Respect rate limits
if let Err(why) = message.reply(&ctx, response).await {
    if why.to_string().contains("rate limit") {
        // Queue for later or drop gracefully
    }
}
```

### User Context
```rust
pub struct DiscordContext {
    pub user_id: UserId,
    pub username: String,
    pub guild_id: Option<GuildId>,
    pub channel_id: ChannelId,
    pub is_dm: bool,
    pub recent_messages: Vec<Message>,
    pub user_timezone: Option<Tz>,
}
```

### Agent Selection
```rust
// Keywords for routing
const CRISIS_KEYWORDS: &[&str] = &["emergency", "crisis", "help", "panic"];
const PLANNING_KEYWORDS: &[&str] = &["plan", "schedule", "organize", "todo"];
const MEMORY_KEYWORDS: &[&str] = &["remember", "recall", "forgot", "memory"];

// Route based on content analysis
fn select_agent_group(content: &str) -> AgentGroup {
    let lower = content.to_lowercase();
    
    if CRISIS_KEYWORDS.iter().any(|&kw| lower.contains(kw)) {
        return AgentGroup::Crisis;
    }
    // ... other checks
    
    AgentGroup::Main // default
}
```

## Command Guidelines

### Slash Commands
```rust
// Clear, action-oriented names
/pattern help     - Show available commands
/pattern status   - Check bot and agent status
/pattern memory   - Interact with memory system
/pattern task     - Task management
/pattern energy   - Check/set energy state

// Admin/Debug commands
/pattern reload   - Reload configuration
/pattern debug    - Show debug information
/pattern agents   - List active agents
```

### Natural Language Detection
- "Hey Pattern" - Wake word for explicit attention
- "Pattern:" - Alternative wake word
- @mention - Always responds when mentioned
- DMs - Always active, no wake word needed

## Privacy & Security

### DM Isolation
```rust
// Never mix DM and guild contexts
struct MessageContext {
    source: MessageSource,
    // ...
}

enum MessageSource {
    DirectMessage { user_id: UserId },
    GuildChannel { guild_id: GuildId, channel_id: ChannelId },
}

// Enforce at type level
impl MessageContext {
    fn can_access_history(&self, other: &MessageContext) -> bool {
        match (&self.source, &other.source) {
            (DirectMessage { user_id: a }, DirectMessage { user_id: b }) => a == b,
            (GuildChannel { guild_id: a, .. }, GuildChannel { guild_id: b, .. }) => a == b,
            _ => false,
        }
    }
}
```

### Permission Checks
- Check bot permissions before operations
- Respect user privacy settings
- Don't store messages without consent
- Clear data on user request

## Error Handling

### Discord API Errors
```rust
match result {
    Err(SerenityError::Http(e)) => {
        match e {
            HttpError::UnsuccessfulRequest(ErrorResponse { status_code, .. }) => {
                match status_code {
                    429 => // Rate limited, back off
                    403 => // No permission, notify user gently
                    404 => // Message/channel deleted, clean up
                    _ => // Log and continue
                }
            }
        }
    }
}
```

### Connection Resilience
- Auto-reconnect on disconnect
- Queue important messages during outages
- Graceful degradation of features
- Status updates in designated channel

## Performance Considerations

- Cache user preferences to reduce DB queries
- Batch Discord API requests when possible
- Use intents to reduce event overhead
- Implement message queuing for high-volume channels
- Background task for cleanup and maintenance

## Testing Discord Features

```bash
# Test with a local bot token
DISCORD_TOKEN=your_test_token cargo run --features discord

# Test specific handlers
cargo test -p pattern-discord test_message_routing

# Integration tests with test server
cargo test -p pattern-discord --features integration_tests
```

## Common Issues

1. **"Bot not responding"**
   - Check intents configuration
   - Verify bot has channel permissions
   - Check for rate limiting

2. **"Commands not showing"**
   - Ensure commands are registered globally/per-guild
   - Check bot has applications.commands scope
   - Wait for command propagation (up to 1 hour)

3. **"Memory not persisting"**
   - Verify user ID mapping is consistent
   - Check database connection
   - Ensure proper shutdown handling

## Future Enhancements

- Voice channel support for body doubling
- Thread auto-creation for task tracking
- Reaction-based quick actions
- Scheduled reminders via Discord events
- Multi-modal support (images, files)