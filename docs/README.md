# Pattern Documentation

Pattern is a multi-agent cognitive support system designed specifically for ADHD brains, inspired by MemGPT's stateful agent architecture.

## Current State

✅ **Working**: Agent groups, Bluesky integration, Discord bot, data sources, CLI  
🚧 **In Progress**: MCP client, API server, Discord slash commands  
📋 **Planned**: Task management, MCP server

## Documentation Structure

### 📐 Core Architecture
- [Pattern Agent Architecture](architecture/pattern-agent-architecture.md) - Agent framework design
- [Database Backend](architecture/database-backend.md) - SurrealDB patterns and entities
- [Memory System](architecture/memory-and-groups.md) - MemGPT-style memory blocks
- [Tool System](architecture/tool-system.md) - Multi-operation tool architecture
- [Context Building](architecture/context-building.md) - How agent context is constructed

### 🔧 Implementation Guides
- [Data Sources](data-sources.md) - How to integrate data sources with agents
- [Group Coordination](group-coordination-guide.md) - Using agent groups and patterns
- [Tool System Guide](tool-system-guide.md) - Creating and registering tools
- [Config Design](config-design.md) - Configuration system architecture

### 🔌 Integration Guides
- [Discord Setup](guides/discord-setup.md) - Discord bot configuration
- [Bluesky Integration](bluesky-integration-plan.md) - ATProto firehose setup
- [MCP Integration](guides/mcp-integration.md) - Model Context Protocol (planned)

### 📚 API References
- [Database API](api/database-api.md) - Database operations and patterns
- [Quick Reference](quick-reference.md) - Common patterns and code snippets

### 🐛 Troubleshooting
- [Known API Issues](known-api-issues.md) - Provider-specific workarounds
- [Agent Loops](troubleshooting/agent-loops.md) - Preventing message loops
- [Discord Issues](troubleshooting/discord-issues.md) - Discord integration issues
- [SurrealDB Patterns](troubleshooting/surrealdb-patterns.md) - Database gotchas

## Quick Start

### CLI Usage
```bash
# Chat with a single agent
pattern-cli chat

# Chat with an agent group
pattern-cli chat --group main

# Use Discord integration
pattern-cli chat --discord

# Manage agent groups
pattern-cli group create MyGroup --description "Test group" --pattern round-robin
pattern-cli group add-member MyGroup agent-name --role member
pattern-cli group status MyGroup
```

### Creating an Agent
```rust
use pattern_core::agent::DatabaseAgent;

let agent = DatabaseAgent::new(
    agent_id,
    user_id,
    agent_type,
    name,
    system_prompt,
    memory,
    db.clone(),
    model_provider,
    tool_registry,
    embedding_provider,
    heartbeat_sender,
).await?;
```

### Adding a Data Source
```rust
use pattern_core::data_source::{FileDataSource, FileStorageMode};

let source = FileDataSource::new(
    "docs".to_string(),
    PathBuf::from("./docs"),
    FileStorageMode::Indexed,
    Some(embedding_provider),
)?;

coordinator.add_source("docs", source).await?;
```

## Development

See [CLAUDE.md](../CLAUDE.md) in the project root for:
- Current development priorities
- Implementation guidelines
- Known issues and workarounds
- Build commands

## Key Concepts

### Agent Groups
Groups allow multiple agents to collaborate using coordination patterns:
- **Round-robin**: Fair distribution
- **Dynamic**: Capability-based routing
- **Pipeline**: Sequential processing
- **Supervisor**: Hierarchical delegation
- **Voting**: Consensus decisions
- **Sleeptime**: Background monitoring

### Memory System
MemGPT-style memory with:
- **Core memory**: Always in context (persona, human)
- **Archival memory**: Searchable long-term storage
- **Conversation history**: Recent messages
- Thread-safe with Arc<DashMap>

### Data Sources
Flexible data ingestion:
- File watching with indexing
- Discord message streams
- Bluesky firehose
- Custom sources via trait implementation