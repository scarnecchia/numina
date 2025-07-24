# Pattern - Agent Platform and Support Constellation

Pattern is two things.

## Pattern Platform:

The first is a platform for building stateful agents, based on the MemGPT paper, similar to Letta. It's flexible and extensible.

- **Flexible data backend**: Based on SurrealDB, which can be used as an embedded or external database.
- **Memory Tools**: Implements the MemGPT/Letta architecture, with versatile tools for agent context management and recall.
- **Agent Protection Tools**: Agent memory and context sections can be protected to stabilize the agent, or set to require consent before alteration.
- **Agent Coordination**: Multiple specialized agents can collaborate and coordinate in a variety of configurations.
- **Multi-user support**: Agents can be configured to have a primary "partner" that they support while interacting with others.
- **Easy to self-host**: The embedded database option plus (nearly) pure rust design makes the platform and tools easy to set up.

### Current Status

**Core Library Framework Complete**:
- Entity system with proc macros and ops functions to make surrealDB simple
- Agent state persistence and recovery
- Built-in tools (context, recall, search, send_message)
- Message compression strategies (truncation, summarization, importance-based)
- Agent groups with coordination patterns (round-robin, dynamic, pipeline)

**In Progress**:
- Vector embeddings and semantic search
- MCP server refactor
- Discord bot integration
- Additional coordination patterns

## The `Pattern` agent constellation:

The second is a multi-agent cognitive support system designed for the neurodivergent. It uses a multi-agent architecture with shared memory to provide external executive function through specialized cognitive agents.

- **Pattern** (Orchestrator) - Runs background checks every 20-30 minutes for attention drift and physical needs
- **Entropy** - Breaks down overwhelming tasks into manageable atomic units
- **Flux** - Translates between ADHD time and clock time (5 minutes = 30 minutes)
- **Archive** - External memory bank for context recovery and pattern finding
- **Momentum** - Tracks energy patterns and protects flow states
- **Anchor** - Manages habits, meds, water, and basic needs without nagging

### Constellation Features:

- **Three-Tier Memory**: Core blocks, searchable sources, and archival storage
- **Discord Integration**: Natural language interface through Discord bot
- **MCP Server**: Expose agent capabilities via Model Context Protocol
- **Cost-Optimized Sleeptime**: Two-tier monitoring (rules-based + AI intervention)
- **Flexible Group Patterns**: Create any coordination style you need
- **Task Management**: ADHD-aware task breakdown with time multiplication
- **Passive Knowledge Sharing**: Agents share insights via embedded documents

## Documentation

All documentation is organized in the [`docs/`](docs/) directory:

- **[Architecture](docs/architecture/)** - System design and technical details
  - [Entity System](docs/architecture/entity-system.md) - Zero-boilerplate database entities
  - [Context Building](docs/architecture/context-building.md) - Stateful agent context management
  - [Tool System](docs/architecture/tool-system.md) - Type-safe tool implementation
  - [Built-in Tools](docs/architecture/builtin-tools.md) - Memory and communication tools
  - [Database Backend](docs/architecture/database-backend.md) - SurrealDB integration
- **[Guides](docs/guides/)** - Setup and integration instructions
  - [MCP Integration](docs/guides/mcp-integration.md) - Model Context Protocol setup (somewhat outdated)
  - [Discord Setup](docs/guides/discord-setup.md) - Discord bot configuration (somewhat outdated)
- **[API Reference](docs/api/)** - API documentation
  - [Database API](docs/api/database-api.md) - Direct database operations
- **[Troubleshooting](docs/troubleshooting/)** - Common issues and solutions
- **[Quick Reference](docs/quick-reference.md)** - Handy command and code snippets

## Neurodivergent-specific Design

Pattern understands that neurodivergent brains are different, not broken:

- **Time Translation**: Automatic multipliers (1.5x-3x) for all time estimates
- **Hidden Complexity**: Recognizes that "simple" tasks are never simple
- **No Shame Spirals**: Validates struggles as logical responses, never suggests "try harder"
- **Energy Awareness**: Tracks attention as finite resource that depletes non-linearly
- **Flow Protection**: Distinguishes productive hyperfocus from harmful burnout
- **Context Recovery**: External memory for "what was I doing?" moments

### Custom Agents

Create custom agent configurations through the builder API or configuration files. See [Architecture docs](docs/architecture/) for details.

## Quick Start

### Prerequisites
- Rust 1.85+ (required for 2024 edition) (or use the Nix flake)
- An LLM API key (Anthropic, OpenAI, Google, etc.)
  - I currently recommend Gemini and OpenAI API keys, because it defaults to using OpenAI for embedding, and I've tested most extensively with Gemini

### Using as a Library

Add `pattern-core` to your `Cargo.toml`:

```toml
[dependencies]
pattern-core = { path = "path/to/pattern/crates/pattern_core" }
# or once published:
# pattern-core = "0.1.0"
```

Create a basic agent:

```rust
use pattern_core::{
    agent::{DatabaseAgent, AgentType},
    config::ModelConfig,
    model::{ModelProvider, providers::GeminiProvider},
    db::SurrealEmbedded,
    memory::Memory,
    tool::ToolRegistry,
    id::{AgentId, UserId},
};
use std::sync::Arc;
use tokio::sync::RwLock;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize database
    let db = SurrealEmbedded::new("./my_pattern.db").await?;
    
    // Create model provider
    let model_config = ModelConfig {
        provider: ModelProvider::Gemini,
        model_name: "gemini-2.0-flash".to_string(),
        api_key: std::env::var("GEMINI_API_KEY")?,
        ..Default::default()
    };
    let provider = Arc::new(RwLock::new(
        GeminiProvider::from_config(&model_config).await?
    ));
    
    // Create agent
    let agent_id = AgentId::generate();
    let user_id = UserId::generate();
    let memory = Memory::with_owner(user_id);
    let tools = ToolRegistry::new();
    
    let agent = DatabaseAgent::new(
        agent_id,
        user_id,
        AgentType::Generic,
        "MyAssistant".to_string(),
        "You are a helpful AI assistant.".to_string(),
        memory,
        db.clone(),
        provider,
        tools,
        None, // No embeddings provider
    );
    
    // Send a message
    use pattern_core::message::{Message, ChatRole};
    let message = Message::new(ChatRole::User, "Hello! How can you help me today?");
    let response = agent.process_message(message).await?;
    println!("Agent: {:?}", response);
    
    Ok(())
}
```

### Using with Groups

```rust
use pattern_core::{
    db::ops::{create_group_for_user, add_agent_to_group},
    coordination::{GroupCoordinationPattern, patterns::RoundRobinManager},
};

// Create a group
let group = create_group_for_user(
    &db,
    user_id,
    "support_team",
    Some("My support agents"),
    GroupCoordinationPattern::RoundRobin,
).await?;

// Add agents to the group
add_agent_to_group(&db, group.id, entropy_agent.id(), "task_breakdown").await?;
add_agent_to_group(&db, group.id, flux_agent.id(), "time_management").await?;

// Use the group - the CLI provides group chat functionality
// Or implement your own using GroupManager trait
let manager = RoundRobinManager::new();
// ... coordinate messages through the group
```

### CLI Tool

The `pattern-cli` tool lets you interact with agents directly:

```bash
# Build the CLI
cargo build --bin pattern-cli

# Create a basic config file (optional)
cp pattern.toml.example pattern.toml
# Edit pattern.toml with your preferences

# Create a .env file for API keys (optional)
echo "GEMINI_API_KEY=your-key-here\nOPENAI_API_KEY=your-key-here" > .env

# Or use environment variables directly
export GEMINI_API_KEY=your-key-here
export OPENAI_API_KEY=your-key-here

# List agents
cargo run --bin pattern-cli -- agent list

# Create an agent
cargo run --bin pattern-cli -- agent create "Entropy"
# Chat with an agent
cargo run --bin pattern-cli -- chat --agent Archive
# or with the default from the config file
cargo run --bin pattern-cli -- chat


# Show agent status
cargo run --bin pattern-cli -- agent status Pattern

# Search conversation history
cargo run --bin pattern-cli -- debug search-conversations --agent Flux "previous conversation"

# Raw database queries for debugging
cargo run --bin pattern-cli -- db query "SELECT * from mem"

# Or run from the crate directory
cd ./crates/pattern-cli
cargo run -- chat
```

The CLI stores its database in `./pattern.db` by default. You can override this with `--db-path` or in the config file.

### Configuration

Pattern looks for configuration in these locations (first found wins):
1. `pattern.toml` in the current directory
2. `~/Library/Application Support/pattern/config.toml` (macOS)
3. `~/.config/pattern/config.toml` (Linux)
4. `~/.pattern/config.toml` (fallback)

See `pattern.toml.example` for all available options.

## Development

### Building

```bash
# Check compilation
cargo check

# Run tests
cargo test --lib

# Full validation (required before commits)
just pre-commit-all

# Build with all features
cargo build --features full
```

### Project Structure

```
pattern/
├── crates/
│   ├── pattern_cli/      # Command-line testing tool
│   ├── pattern_core/     # Agent framework, memory, tools, coordination
│   ├── pattern_nd/       # Tools and agent personalities specific to the neurodivergent support constellation
│   ├── pattern_mcp/      # MCP server implementation
│   ├── pattern_discord/  # Discord bot integration
│   └── pattern_main/     # Main orchestrator binary (mostly legacy as of yet)
├── docs/                 # Architecture and integration guides
└── CLAUDE.md          # Development reference
```

## Roadmap

### In Progress
- Build-out of the core framework
  - Vector search
  - MCP refactor
  - Discord re-integration
- Re-implementation of the core Pattern constellation
- Command-line tool for chat and debugging

### Planned
- Webapp-based playground environment for platform
- Contract/client tracking for freelancers
- Social memory for birthdays and follow-ups
- Activity monitoring for interruption timing
- Bluesky integration for public accountability

## Acknowledgments

- Inspired by Brandon Sanderson's cognitive multiplicity model in Stormlight Archive
- Designed by someone who gets it - time is fake but deadlines aren't

## License

Pattern is dual-licensed:

- **AGPL-3.0** for open source use - see [LICENSE](LICENSE)
- **Commercial License** available for proprietary applications - contact for details

This dual licensing ensures Pattern remains open for the neurodivergent community while supporting sustainable development. Any use of Pattern in a network service or application requires either compliance with AGPL-3.0 (sharing source code) or a commercial license.
