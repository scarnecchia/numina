# Pattern Usage Guide

Pattern is a unified binary that provides both MCP server and Discord bot functionality for the multi-agent ADHD support system.

## Running Modes

### 1. Full Mode (Discord + MCP + Background Tasks)

Run everything in a single process:

```bash
# Development
cargo run --features full

# Production
cargo build --release --features full
./target/release/pattern
```

### 2. Discord Bot Only

Run just the Discord bot with background tasks:

```bash
cargo run --features binary,discord
```

### 3. MCP Server Only

Run just the MCP server:

```bash
cargo run --features binary,mcp
```

### 4. Standalone Binaries

You can also run the legacy standalone binaries:

```bash
# Discord bot
cargo run --features discord --bin discord_bot

# MCP server  
cargo run --features mcp --bin mcp
```

## Configuration

### Environment Variables

Create a `.env` file:

```bash
# Discord
DISCORD_TOKEN=your_bot_token

# Letta
LETTA_BASE_URL=http://localhost:8000
# Or for cloud:
LETTA_API_KEY=your_api_key

# Database
PATTERN_DB_PATH=pattern.db

# MCP
PATTERN_MCP_ENABLED=true
PATTERN_MCP_TRANSPORT=stdio  # or sse
```

### Config File

Create `pattern.toml`:

```toml
[database]
path = "pattern.db"

[discord]
token = "your_bot_token"
respond_to_dms = true
respond_to_mentions = true

[letta]
base_url = "http://localhost:8000"
# api_key = "your_api_key"  # for cloud

[mcp]
enabled = true
transport = "stdio"
```

## Features

### Background Tasks

- **Sleeptime Orchestrator**: Pattern agent runs checks every 20-30 minutes
- **State Tracking**: Updates shared memory with system state
- **Future**: Task scheduling, energy monitoring, habit tracking

### Discord Features

- Natural language chat with agents
- Agent routing: `@entropy`, `flux:`, `/agent momentum`
- Slash commands: `/chat`, `/vibe`, `/tasks`, `/memory`
- DM support for private conversations

### MCP Tools

- `chat_with_agent` - Send messages to specific agents
- `get_agent_memory` - Retrieve agent memory state
- `update_agent_memory` - Update memory blocks
- `schedule_event` - Smart event scheduling (planned)
- `send_message` - Send Discord messages (planned)
- `check_activity_state` - Activity monitoring (planned)

## Development

### Building with Specific Features

```bash
# Just the library
cargo build --no-default-features

# Library + Discord
cargo build --features discord

# Library + MCP
cargo build --features mcp

# Everything
cargo build --features full
```

### Testing

```bash
# Run all tests
cargo test --features full

# Test specific feature
cargo test --features discord
```

## Architecture

The unified binary approach allows:
- Single process for all functionality
- Shared state between Discord and MCP
- Background tasks running alongside services
- Easier deployment and management
- Optional feature combinations

Services communicate through the shared `MultiAgentSystem` which manages:
- Agent instances and routing
- Shared memory blocks
- User state and preferences
- Task and event management