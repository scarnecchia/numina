# Pattern - Multi-Agent ADHD Support System

Pattern is an MCP (Model Context Protocol) server that extends letta-rs to build an autonomous multi-agent system designed for ADHD cognitive support. It provides a flexible framework for building personalized AI agent systems with shared memory and coordination.

## Features

- **Multi-Agent Architecture**: Configurable agents with specialized roles
- **Shared Memory System**: Agents share context through memory blocks
- **Discord Integration**: Natural language interface through Discord
- **Task Management**: ADHD-aware task breakdown and tracking
- **Time Management**: Smart scheduling with time multiplication for ADHD
- **MCP Server**: Expose agent capabilities as MCP tools
- **Flexible & Generic**: Build your own agent configurations

## Quick Start

### Prerequisites

- Rust 1.75+
- SQLite
- Letta server running locally or Letta cloud API key
- Discord bot token (for Discord integration)

### Basic Setup

1. Clone and build:
```bash
cd pattern
cargo build
```

2. Set up environment:
```bash
cp .env.example .env
# Edit .env with your configuration
```

3. Run Discord bot:
```bash
cargo run --bin discord_bot
```

## Architecture

### Multi-Agent System

Pattern implements a flexible multi-agent architecture inspired by Brandon Sanderson's Stormlight Archive:

- **Pattern (Orchestrator)**: Sleeptime agent that coordinates others
- **Entropy**: Task breakdown and management
- **Flux**: Time estimation and scheduling
- **Archive**: Memory and information retrieval
- **Momentum**: Energy and attention tracking
- **Anchor**: Habits and routine management

### Shared Memory Blocks

Agents communicate through shared memory:
- `current_state`: Overall system state/vibe
- `active_context`: Current task and context
- `bond_evolution`: Relationship progression

## Configuration

### Environment Variables

```bash
# Discord
DISCORD_TOKEN=your_bot_token

# Letta
LETTA_BASE_URL=http://localhost:8000
# Or for cloud:
LETTA_API_KEY=your_api_key

# Database
PATTERN_DB_PATH=pattern.db
```

### Custom Agents

Create custom agent configurations in `src/bin/discord_bot.rs`:

```rust
builder.add_agent(
    "myagent".to_string(),          // ID for routing
    "MyAgent".to_string(),          // Display name
    "My custom agent".to_string(),  // Description
    "You are MyAgent...".to_string(), // System prompt
    false,                          // Not sleeptime
);
```

## Usage

### Discord Commands

- `/chat [message]` - Chat with agents
- `/vibe` - Check system state
- `/memory` - View shared memory

### Agent Routing

Route messages to specific agents:
- `@entropy break down this task`
- `flux: when should I do this?`
- `/agent archive find that thing`

## Development

### Project Structure

```
pattern/
├── src/
│   ├── lib.rs          # Core library
│   ├── agents.rs       # Multi-agent system
│   ├── agent.rs        # Letta integration
│   ├── discord.rs      # Discord bot
│   ├── db.rs           # Database layer
│   └── bin/
│       ├── mcp.rs      # MCP server
│       └── discord_bot.rs
├── migrations/         # SQL migrations
└── CLAUDE.md          # Development guide
```

### Running Tests

```bash
cargo test
```

### Building with Features

```bash
# With MCP server
cargo build --features mcp

# Just the library
cargo build --no-default-features
```

## MCP Integration

When built with `--features mcp`, Pattern exposes MCP tools:

- `chat_with_agent` - Send messages to agents
- `get_agent_memory` - Retrieve agent memory
- `update_agent_memory` - Update memory blocks
- `schedule_event` - Smart scheduling (planned)
- `check_activity_state` - Interruption detection (planned)

## Contributing

See [CLAUDE.md](CLAUDE.md) for development guidelines and architecture details.

## License

[Your license here]

## Acknowledgments

- Inspired by Brandon Sanderson's cognitive multiplicity model in Stormlight Archive
- Built on [letta-rs](https://github.com/letta-ai/letta-rs) and [rmcp](https://github.com/modelcontextprotocol/rust-sdk)