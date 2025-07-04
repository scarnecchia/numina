# Testing Pattern

This guide explains how to test the Pattern ADHD support system.

## Prerequisites

1. **Letta Server**: Make sure Letta is running in Docker
   ```bash
   # In the letta-rs directory
   cd ../letta-rs
   docker compose up -d
   
   # Check if it's running
   curl http://localhost:8283/v1/health
   
   # View logs if needed
   docker compose logs -f
   ```

2. **Environment Variables**: Create a `.env` file with:
   ```env
   # Letta Configuration
   LETTA_BASE_URL=http://localhost:8283

   # Discord Bot Configuration
   DISCORD_TOKEN=your_discord_bot_token_here
   # Optional: limit bot to specific channel
   DISCORD_CHANNEL_ID=your_channel_id_here
   ```

## Running Pattern

### Full Mode (Discord + MCP + Background Tasks)
```bash
cargo run --features full
```

### Just Discord Bot
```bash
cargo run --features binary,discord
```

### Just MCP Server
```bash
cargo run --features binary,mcp
```

## Testing Discord Bot

### 1. First Time Setup
- The bot will register slash commands on startup
- **Global commands can take up to 1 hour to appear**
- Check console for "Pattern is connected!" message

### 2. Available Commands
- `/chat message:"your message" agent:"optional agent name"` - Chat with agents
- `/vibe` - Check current state
- `/tasks` - List tasks (not implemented yet)
- `/memory` - Show shared memory state

### 3. Natural Language Chat
You can also chat naturally in channels or DMs:
- `@pattern hello!` - Mentions work if bot has permissions
- `entropy: help me break down this task` - Agent routing
- `@flux when should I schedule this?` - Direct agent chat
- Regular messages in the configured channel

### 4. Agent Routing
Pattern will route messages to specific agents based on:
- `@agent_name message` - Route to specific agent
- `agent_name: message` - Alternative routing format
- `/agent agent_name message` - Another routing option
- No prefix = routes to Pattern (default orchestrator)

## Testing MCP Server

### 1. With Claude Desktop
Add to your Claude Desktop config:
```json
{
  "mcpServers": {
    "pattern": {
      "command": "/path/to/pattern",
      "args": ["--features", "binary,mcp"],
      "env": {
        "LETTA_BASE_URL": "http://localhost:8283"
      }
    }
  }
}
```

### 2. Available MCP Tools
- `chat_with_agent` - Send messages to agents
- `get_agent_memory` - Retrieve agent memory
- `update_agent_memory` - Update memory blocks
- `send_discord_message` - Send to Discord channels
- `send_discord_embed` - Send rich embeds
- `get_discord_channel_info` - Get channel details
- `send_discord_dm` - Send direct messages
- `schedule_event` - Schedule with ADHD time buffers (not implemented)
- `send_message` - Generic message sending (not implemented)
- `check_activity_state` - Check interruption timing (not implemented)

## Debugging Tips

### Check Logs
Pattern uses extensive logging:
```bash
# Set log level
RUST_LOG=pattern=debug cargo run --features full
```

### Database
SQLite database is created at `pattern.db` by default:
```bash
# View database
sqlite3 pattern.db
.tables
.schema users
```

### Common Issues

1. **Slash commands not appearing**: 
   - Wait up to 1 hour for global commands
   - Check bot has `applications.commands` scope
   - Verify bot is in your server with proper permissions

2. **Bot not responding**:
   - Check DISCORD_CHANNEL_ID if set
   - Verify bot has message read permissions
   - Check logs for errors

3. **Letta connection errors**:
   - Ensure Letta server is running
   - Verify LETTA_BASE_URL is correct
   - Check Letta server logs

## Quick Test Script

```bash
#!/usr/bin/env bash
# test_pattern.sh

# 1. Start Letta Docker container
cd ../letta-rs && docker compose up -d

# 2. Wait for Letta to be ready
echo "Waiting for Letta..."
until curl -s http://localhost:8283/v1/health > /dev/null; do
    sleep 1
done

# 3. Run Pattern
cd ../pattern
RUST_LOG=pattern=info cargo run --features full

# Note: Remember to stop Letta when done
# cd ../letta-rs && docker compose down
```

## Development Testing

### Unit Tests
```bash
cargo test --lib
```

### Integration Tests (requires Letta running)
```bash
cargo test --features full
```

### Specific Module Tests
```bash
cargo test --lib -- db::
cargo test --lib -- agent::
cargo test --lib -- mcp::
cargo test --lib -- discord::
```