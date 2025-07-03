# Pattern - Multi-Agent Cognitive Support System

Pattern is a multi-agent cognitive support system designed for the neurodivergent. It uses Letta's multi-agent architecture with shared memory to provide external executive function through specialized cognitive agents.

## What is Pattern?

Pattern implements a constellation of AI agents that work together to support cognition in those with executive function problems:

- **Pattern** (Orchestrator) - Runs background checks every 20-30 minutes for attention drift and physical needs
- **Entropy** - Breaks down overwhelming tasks into manageable atomic units
- **Flux** - Translates between ADHD time and clock time (5 minutes = 30 minutes)
- **Archive** - External memory bank for context recovery and pattern finding
- **Momentum** - Tracks energy patterns and protects flow states
- **Anchor** - Manages habits, meds, water, and basic needs without nagging

## Features

- **Multi-Agent Architecture**: Configurable agents with specialized roles and shared memory
- **Discord Integration**: Natural language interface through Discord bot
- **MCP Server**: Expose agent capabilities via Model Context Protocol
- **HTTP Transport**: Persistent MCP connections for Letta integration
- **Background Tasks**: Sleeptime orchestrator for proactive support
- **Task Management**: ADHD-aware task breakdown with time multiplication
- **Flexible Framework**: Build your own agent configurations

## Quick Start

### Prerequisites

- Rust 1.75+
- SQLite
- Letta server running locally or Letta cloud API key
- Discord bot token (optional, for Discord integration)

### Basic Setup

```bash
# Clone and build
git clone [repository]
cd pattern
cargo build --release

# Set up environment
cp .env.example .env
# Edit .env with your configuration

# Run Pattern with all features
cargo run --features full

# Or run specific components
cargo run --features binary,discord  # Just Discord bot
cargo run --features binary,mcp      # Just MCP server
```

## Documentation

All documentation is organized in the `docs/` directory:

- **[Documentation Index](docs/README.md)** - Start here for comprehensive guides
- **[Architecture](docs/architecture/)** - System design and agent details
- **[Setup Guides](docs/guides/)** - Discord, MCP, and usage instructions
- **[Testing Guide](docs/guides/TESTING.md)** - How to test Pattern
- **[Development](CLAUDE.md)** - Development guide, TODOs, and progress

## ADHD-Specific Design

Pattern understands that ADHD brains are different, not broken:

- **Time Translation**: Automatic multipliers (1.5x-3x) for all time estimates
- **Hidden Complexity**: Recognizes that "simple" tasks are never simple
- **No Shame Spirals**: Validates struggles as logical responses, never suggests "try harder"
- **Energy Awareness**: Tracks attention as finite resource that depletes non-linearly
- **Flow Protection**: Distinguishes productive hyperfocus from harmful burnout
- **Context Recovery**: External memory for "what was I doing?" moments

## Usage Examples

### Discord Bot

```
# Chat with default orchestrator
@pattern hey, I'm feeling overwhelmed

# Route to specific agent
@entropy help me break down this project
flux: when should I schedule this task?
/agent archive what did we discuss yesterday?

# Check system state
/vibe
/memory
```

### MCP Tools

When integrated with Claude or other MCP clients:
- `chat_with_agent` - Send messages to any agent
- `get_agent_memory` - Retrieve shared memory state
- `update_agent_memory` - Update memory blocks
- Discord integration tools for sending messages
- More tools in development

## Configuration

### Environment Variables

```bash
# Discord (optional)
DISCORD_TOKEN=your_bot_token
DISCORD_CHANNEL_ID=optional_channel_limit

# Letta
LETTA_BASE_URL=http://localhost:8283
# Or for cloud:
LETTA_API_KEY=your_api_key

# Database
PATTERN_DB_PATH=pattern.db

# MCP
MCP_ENABLED=true
MCP_TRANSPORT=http
MCP_PORT=8080
```

### Custom Agents

Create custom agent configurations through the builder API or configuration files. See [Architecture docs](docs/architecture/) for details.

## Development

### Project Structure

```
pattern/
├── src/
│   ├── lib.rs          # Core library
│   ├── agents.rs       # Multi-agent system
│   ├── agent.rs        # Letta integration
│   ├── discord.rs      # Discord bot
│   ├── server.rs       # MCP server
│   ├── service.rs      # Service orchestrator
│   └── db.rs           # Database layer
├── docs/               # Documentation
├── scripts/            # Utility scripts
├── migrations/         # SQL migrations
└── CLAUDE.md          # Development reference
```

### Running Tests

```bash
# Unit tests
cargo test --lib

# Integration tests (requires Letta)
cargo test --features full

# Specific modules
cargo test --lib -- agents::
```

### Building with Features

- `discord` - Discord bot support
- `mcp` - MCP server support
- `mcp-sse` - Server-sent events transport
- `binary` - Build executables
- `full` - All features enabled

## Roadmap

### In Progress
- Meaningful sleeptime background checks
- Task CRUD operations
- Shared agent tools implementation

### Planned
- Contract/client tracking for freelancers
- Social memory for birthdays and follow-ups
- Activity monitoring for interruption timing
- Bluesky integration for public accountability

## Acknowledgments

- Inspired by Brandon Sanderson's cognitive multiplicity model in Stormlight Archive
- Built on [letta-rs](https://github.com/letta-ai/letta-rs) and [MCP Rust SDK](https://github.com/modelcontextprotocol/rust-sdk)
- Designed by someone who gets it - time is fake but deadlines aren't

## License

Pattern is dual-licensed:

- **AGPL-3.0** for open source use - see [LICENSE](LICENSE)
- **Commercial License** available for proprietary applications - contact for details

This dual licensing ensures Pattern remains open for the neurodivergent community while supporting sustainable development. Any use of Pattern in a network service or application requires either compliance with AGPL-3.0 (sharing source code) or a commercial license.
