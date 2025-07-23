# Pattern Workspace Structure

This is a multi-crate Rust workspace for the Pattern ADHD cognitive support system.

## Crate Overview

### pattern-core
The foundational crate providing:
- Agent framework (trait definitions, base implementations)
- Memory system with vector search capabilities
- Tool execution framework
- Model integration via genai crate
- Multi-agent coordination patterns
- SurrealDB integration for persistence

### pattern-nd
Neurodivergent-specific features:
- ADHD agent personalities (Pattern, Entropy, Flux, Archive, Momentum, Anchor)
- ADHD-aware tools (task breakdown, time multiplication, energy tracking)
- Sleeptime monitoring system
- Executive function support utilities

### pattern-mcp
Model Context Protocol server implementation:
- Multiple transport support (stdio, HTTP, SSE)
- Tool registration and discovery
- Protocol message handling
- Client session management

### pattern-discord
Discord bot integration:
- Natural language chat interface
- Slash command support
- Message routing to agents
- User context management
- Discord-specific tools

### pattern-main
Main orchestrator that ties everything together:
- Service coordination
- Configuration management
- Database migrations
- Binary entry points
- Feature flag coordination

## Building

```bash
# Build all crates
cargo build --workspace

# Build specific crate
cargo build -p pattern-core

# Build with all features
cargo build --workspace --all-features

# Run the main binary
cargo run -p pattern-main
```

## Development

```bash
# Run all tests
cargo test --workspace

# Run tests for specific crate
cargo test -p pattern-core

# Check compilation
cargo check --workspace

# Format code
cargo fmt --all

# Run clippy
cargo clippy --workspace -- -D warnings
```

## Feature Flags

The pattern-main crate supports these features:
- `discord` - Enable Discord bot
- `mcp` - Enable MCP server
- `nd` - Enable neurodivergent features
- `binary` - Build as executable with CLI
- `full` - All features enabled (default)

## Migration Status

We are currently migrating from a monolithic structure with Letta dependency to this multi-crate architecture. During the transition:
- Legacy code remains in pattern-main/src
- New implementations go in their respective crates
- The goal is to completely remove the Letta dependency

## Database

We're using SurrealDB as our primary database, providing:
- Multi-model support (document, graph, relational, vector)
- Built-in vector search for semantic memory
- Real-time subscriptions for agent coordination
- Graph relationships for agent constellations
- Time-series support for ADHD tracking