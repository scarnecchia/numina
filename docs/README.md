# Pattern Documentation

Pattern is a multi-agent cognitive support system designed specifically for ADHD brains, using Letta's multi-agent architecture with shared memory.

## Documentation Structure

### 📐 Architecture
- [Agent Architecture](architecture/pattern-agent-architecture.md) - Multi-agent system design
- [System Prompts](architecture/pattern-system-prompts.md) - Agent personality definitions
- [Agent Routing](architecture/AGENT-ROUTING.md) - How messages are routed to agents

### 📚 Setup Guides
- [Discord Setup](guides/DISCORD_SETUP.md) - Setting up the Discord bot
- [MCP HTTP Setup](guides/MCP_HTTP_SETUP.md) - Configuring HTTP transport for MCP
- [Usage Guide](guides/USAGE.md) - General usage instructions
- [Testing Guide](guides/TESTING.md) - How to test Pattern

### 🔧 Development Guides
- [MCP SDK Guide](guides/MCP_SDK_GUIDE.md) - Working with the MCP Rust SDK
- [LSP Edit Guide](guides/LSP_EDIT_GUIDE.md) - Using language server protocol tools
- [CLAUDE.md](../CLAUDE.md) - Main development reference and TODOs

### 📖 API References
- [Letta API Reference](api/LETTA_API_REFERENCE.md) - Common Letta API patterns and gotchas

### 🎯 Feature Concepts
- [Bluesky Shame Feature](BLUESKY_SHAME_FEATURE.md) - Public accountability posts concept

## Quick Start

1. Copy `.env.example` to `.env` and configure
2. Run `cargo run --features full` to start Pattern with all features
3. See [Discord Setup](guides/DISCORD_SETUP.md) for bot configuration
4. See [MCP HTTP Setup](guides/MCP_HTTP_SETUP.md) for MCP server setup

## Scripts

Utility scripts are in the `scripts/` directory:
- `cleanup_mcp.sh` - Remove MCP server registration from Letta
- `register_mcp_manually.sh` - Manually register MCP server when Letta is ready
- `test_bot.sh` - Test the Discord bot
- `test_mcp_connection.sh` - Test MCP HTTP connectivity