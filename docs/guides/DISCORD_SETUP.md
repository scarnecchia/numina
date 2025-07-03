# Pattern Discord Bot

The Pattern Discord bot provides a natural language interface to the multi-agent system through Discord.

## Setup

1. Create a Discord application and bot at https://discord.com/developers/applications
2. Copy `.env.discord` to `.env` and add your bot token
3. Invite the bot to your server with appropriate permissions (Send Messages, Read Messages, Use Slash Commands)

## Running the Bot

```bash
# With local Letta server
cargo run --bin discord_bot

# Or with environment variables
DISCORD_TOKEN=your_token LETTA_BASE_URL=http://localhost:8000 cargo run --bin discord_bot
```

## Usage

### Slash Commands

- `/chat [message]` - Chat with the agents
- `/vibe` - Check current system state/vibe
- `/tasks` - List current tasks (coming soon)
- `/memory` - Show shared memory state

### Natural Language Routing

The bot supports flexible agent routing in DMs and channels:

#### Default Agents (configurable)
- `pattern` - Orchestrator/sleeptime agent
- `entropy` - Task management
- `flux` - Time management  
- `archive` - Memory/search
- `momentum` - Energy tracking
- `anchor` - Habits

#### Routing Formats

1. **@ mentions**: `@entropy help me break down this task`
2. **Colon prefix**: `flux: when should I schedule this?`
3. **Slash agent**: `/agent momentum how's my energy?`
4. **Default**: Messages without routing go to the orchestrator

### Customizing Agents

You can customize which agents are available by modifying the `MultiAgentSystemBuilder` in `src/bin/discord_bot.rs`:

```rust
// Add custom agents
builder.add_agent(
    "chaos".to_string(),     // Internal ID (used for routing)
    "Chaos".to_string(),     // Display name
    "Task breaker".to_string(), // Description
    "You are Chaos, the task breakdown specialist.".to_string(),
    false, // Not a sleeptime agent
);
```

Agent names are automatically detected for routing - no hardcoded lists!

## Configuration

### Environment Variables

- `DISCORD_TOKEN` (required) - Bot token
- `DISCORD_CHANNEL_ID` (optional) - Limit to specific channel
- `LETTA_BASE_URL` - Letta server URL (default: http://localhost:8000)
- `LETTA_API_KEY` - For Letta cloud instead of local server
- `PATTERN_DB_PATH` - Database location (default: pattern.db)

### Config File

Create `pattern.toml` for persistent configuration:

```toml
[discord]
token = "your_bot_token"
respond_to_dms = true
respond_to_mentions = true

[letta]
base_url = "http://localhost:8000"
# Or for cloud:
# api_key = "your_api_key"

[database]
path = "pattern.db"
```

## Security Notes

- Never commit `.env` or `pattern.toml` with real tokens
- Use environment variables for sensitive data in production
- The bot requires MESSAGE_CONTENT intent for natural language routing