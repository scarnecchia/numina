# Pattern Quick Reference Guide

Quick overview of Pattern's current implementation and key patterns.

## Project Structure

```
pattern/
├── crates/
│   ├── pattern_api/      # Shared API types
│   ├── pattern_cli/      # Command-line interface
│   ├── pattern_core/     # Core framework
│   ├── pattern_nd/       # ADHD-specific tools
│   ├── pattern_mcp/      # MCP server (stub)
│   ├── pattern_macros/   # Entity derive macros
│   ├── pattern_discord/  # Discord bot
│   ├── pattern_server/   # Backend API (in dev)
│   └── pattern_main/     # Main orchestrator
├── docs/                 # Documentation
└── pattern.toml          # Configuration
```

## CLI Commands

```bash
# Single agent chat
pattern-cli chat

# Group chat
pattern-cli chat --group main

# Discord mode
pattern-cli chat --discord
pattern-cli chat --discord --group main

# Agent management
pattern-cli agent create <name> --type assistant
pattern-cli agent list
pattern-cli agent status <name>

# Group management
pattern-cli group create <name> --description "desc" --pattern round-robin
pattern-cli group add-member <group> <agent> --role member
pattern-cli group list
pattern-cli group status <name>

# Memory inspection
pattern-cli memory list <agent-name>
pattern-cli memory show <agent-name> <block-label>
```

## Agent Creation

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

## Built-in Tools

### context (Memory Operations)
```json
{
  "operation": "append",
  "label": "human",
  "value": "User prefers dark mode"
}
```
Operations: `append`, `replace`, `archive`, `load_from_archival`, `swap`

### recall (Archival Memory)
```json
{
  "operation": "insert",
  "label": "meeting_notes_2024",
  "value": "Discussed project timeline"
}
```
Operations: `insert`, `append`, `read`, `delete`

### search (Unified Search)
```json
{
  "domain": "archival_memory",
  "query": "project timeline",
  "limit": 10
}
```
Domains: `archival_memory`, `conversations`, `all`

### send_message (Agent Communication)
```json
{
  "target": {
    "type": "agent",
    "id": "agent-123"
  },
  "content": "Please review this"
}
```
Target types: `user`, `agent`, `group`, `discord`, `bluesky`

## Memory System

```rust
// Thread-safe with Arc<DashMap>
let memory = Memory::with_owner(user_id);

## ID Tips (to_key vs to_string)

- Use `id.to_key()` for persistence (raw record key) and `RecordId::from(&id)` for query binds.
- Use `id.to_string()` only for human-readable output (often includes a table prefix like `agent:` or `group:`).
- Example: deriving a `MemoryId` or tracker key from a `GroupId` should use `group_id.to_key()`, not `group_id.to_string()`.

Caveat (rare acceptable `to_string()`): some IDs (e.g., `Did`, `MessageId`) display as their raw key without a table prefix. In those cases `to_string()` can be acceptable if you explicitly need the raw key string. Prefer `to_key()` when unsure.

// Core memory blocks
memory.create_block("persona", "I am Pattern")?;
memory.create_block("human", "User info here")?;

// Update operations
memory.update_block_value("human", "New info")?;
memory.append_to_block("human", "\nAdditional info")?;

// Archival operations (via recall tool)
agent.execute_tool("recall", json!({
    "operation": "insert",
    "label": "notes_2024",
    "value": "Important information"
})).await?;
```

## Data Sources

```rust
use pattern_core::data_source::{
    DataIngestionCoordinator,
    FileDataSource,
    FileStorageMode
};

// Create coordinator
let coordinator = DataIngestionCoordinator::new(
    message_router,
    agent.embedding_provider(),
)?;

// Add file source
let source = FileDataSource::new(
    "docs",
    PathBuf::from("./docs"),
    FileStorageMode::Indexed,
    Some(embedding_provider),
)?;
coordinator.add_source("docs", source).await?;

// Add Bluesky source
let bsky_source = BlueskyFirehoseSource::new(
    jetstream_url,
    filter,
    agent_did,
)?;
coordinator.add_source("bluesky", bsky_source).await?;
```

## Group Coordination Patterns

```rust
use pattern_core::coordination::{
    CoordinationPattern,
    GroupManager,
    RoundRobinManager,
    DynamicManager
};

// Round-robin distribution
let manager = RoundRobinManager::new(agents);

// Dynamic routing
let manager = DynamicManager::new(
    agents,
    DynamicSelector::Capability("task_breakdown"),
);

// Pipeline processing
let manager = PipelineManager::new(agents);

// Send to group
let response = manager.send_message(
    message,
    Some(metadata),
).await?;
```

## Database Patterns

```rust
// ALWAYS use parameter binding
let result = db.query(
    "SELECT * FROM agent WHERE user_id = $user_id"
).bind(("user_id", user_id)).await?;

// Entity with RELATE edges
#[derive(Entity)]
#[entity(entity_type = "user")]
struct User {
    pub id: UserId,
    #[entity(relation = "owns")]
    pub agents: Vec<Agent>,
}

// Create RELATE edge
db.query(
    "RELATE $user->owns->$agent"
).bind(("user", user_id))
 .bind(("agent", agent_id))
 .await?;
```

## Agent Naming, Roles, and Defaults

- Agent names are arbitrary: features no longer depend on specific names like "Pattern", "Anchor", or "Archive".
- Group roles drive behavior:
  - Supervisor: acts as orchestrator; data sources (e.g., Bluesky/Jetstream) route through the Supervisor by default.
  - Specialist with domain "system_integrity": gets the SystemIntegrityTool.
  - Specialist with domain "memory_management": gets the ConstellationSearchTool.
- Sleeptime prompts are role/domain-based:
  - Supervisor uses the former orchestrator prompt; specialists use prompts matching their domain.
- Discord defaults:
  - Default agent selection prefers the Supervisor when an agent isn’t specified.
  - Bot self-mentions resolve to the Supervisor agent’s name when available.
- CLI sender labels by origin:
  - Agent: agent name
  - Bluesky: `@handle`
  - Discord: `Discord`
  - DataSource: `source_id`
  - CLI: `CLI`
  - API: `API`
  - Other: `origin_type`
  - None/unknown: `Runtime`

## Error Handling

```rust
use pattern_core::error::CoreError;

// Specific error variants with context
return Err(CoreError::tool_not_found(
    name,
    available_tools
));

return Err(CoreError::memory_not_found(
    &agent_id,
    &block_name,
    available_blocks
));
```

## Environment Variables

```bash
# Database
DATABASE_URL=ws://localhost:8000

# Discord
DISCORD_TOKEN=your-bot-token
DISCORD_BATCH_DELAY_MS=1500

# Bluesky
BLUESKY_IDENTIFIER=your.handle.bsky.social
BLUESKY_PASSWORD=app-password
JETSTREAM_URL=wss://jetstream.atproto.tools

# Model providers
ANTHROPIC_API_KEY=sk-ant-...
OPENAI_API_KEY=sk-...
GOOGLE_API_KEY=...
```

## Common Patterns

### Creating Custom Tool
```rust
#[derive(Debug, Clone)]
struct MyTool;

#[async_trait]
impl AiTool for MyTool {
    type Input = MyInput;
    type Output = MyOutput;

    fn name(&self) -> &str { "my_tool" }
    fn description(&self) -> &str { "Does something" }
    
    async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
        // Implementation
    }
}

// Register
tool_registry.register(Box::new(MyTool));
```

### Anti-looping Protection
```rust
// Router has built-in cooldown
router.set_cooldown_duration(Duration::from_secs(30));

// Messages between agents throttled automatically
```

## Performance Notes

- CompactString inlines ≤24 byte strings
- DashMap shards for concurrent access
- AgentHandle provides cheap cloning
- Memory uses Arc internally
- Database ops are non-blocking
