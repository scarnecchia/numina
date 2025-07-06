# Pattern Refactor Plan: Multi-Crate Architecture

## Overview

We're restructuring Pattern from a monolithic crate into a workspace of focused crates, removing our dependency on Letta and building our own core agent framework using the `genai` crate for model abstraction.

## Crate Structure

### 1. `pattern-core` - Agent Framework & Core Tools
**Purpose**: Replace Letta with our own agent/memory/tool framework

**Key Components**:
- **Agent System**
  - Agent trait with state management
  - Message routing and conversation handling
  - Tool execution framework
  - Memory abstraction (working memory, long-term, archival)
  - Multi-agent coordination patterns (supervisor, round-robin, voting, pipeline)
  - Agent groups and constellation management
  - Inter-agent communication protocols
  
- **Model Integration**
  - Use `genai` crate for provider abstraction
  - Model capability tiers (Routine/Interactive/Investigative/Critical)
  - Streaming support for real-time responses
  - Token counting and cost tracking
  
- **Tool Framework**
  - Tool trait for extensibility
  - Tool registry and discovery
  - Parameter validation
  - Async execution support
  
- **Memory System**
  - Memory block abstraction
  - Vector search capabilities (later)
  - Persistence layer abstraction
  - Memory sharing between agents
  
- **Core Tools** (non-ADHD specific)
  - File operations
  - Web fetch
  - Time/date utilities
  - Basic task management
  - Note-taking
  
**Dependencies**:
- `genai` - AI model abstraction
- `serde` - Serialization
- `tokio` - Async runtime
- `surrealdb` - Multi-model database with vector search
- `uuid` - Agent/conversation IDs

### 2. `pattern-mcp` - MCP Server Implementation
**Purpose**: Clean separation of MCP protocol handling

**Key Components**:
- MCP server implementation (stdio, HTTP, SSE transports)
- Tool registration from pattern-core
- Protocol message handling
- Client session management
- Error handling and validation

**Dependencies**:
- `pattern-core` - For tool definitions
- `mcp-rs` - MCP protocol implementation
- Transport-specific deps (axum for HTTP, etc.)

### 3. `pattern-nd` - Neurodivergent Support Tools
**Purpose**: ADHD-specific tools and behaviors

**Key Components**:
- **ADHD Tools**
  - Task breakdown with time multiplication
  - Energy state tracking
  - Hyperfocus detection
  - Break reminders
  - Context recovery
  
- **Agent Personalities**
  - Pattern (orchestrator)
  - Entropy (task breakdown)
  - Flux (time management)
  - Archive (memory)
  - Momentum (energy tracking)
  - Anchor (habits/routines)
  
- **Sleeptime Monitor**
  - Two-tier monitoring system
  - ADHD-aware triggers
  - Intervention logic

**Dependencies**:
- `pattern-core` - Agent framework
- Domain-specific libs as needed

### 4. `pattern-discord` - Discord Bot
**Purpose**: Discord integration as a clean module

**Key Components**:
- Discord bot implementation
- Slash command handling
- Message routing to agents
- Channel/DM context management
- User session tracking
- Discord-specific tools

**Dependencies**:
- `pattern-core` - Agent system
- `pattern-nd` - ADHD tools (optional)
- `serenity` - Discord library

### 5. `pattern` - Main Orchestrator
**Purpose**: Unified backend that ties everything together

**Key Components**:
- Service orchestrator
- Configuration management
- Database migrations
- Binary entry points
- Feature flag coordination
- Background task scheduling

**Dependencies**:
- All pattern-* crates
- Configuration libs
- Database drivers

## Migration Strategy

### Phase 1: Core Framework (Week 1-2)
1. Create workspace structure
2. Build `pattern-core` with basic agent framework
3. Implement tool trait and registry
4. Create memory abstraction
5. Add genai integration for model calls
6. Unit tests for core functionality

### Phase 2: Extract Existing (Week 3)
1. Move MCP code to `pattern-mcp`
2. Extract ADHD tools to `pattern-nd`
3. Migrate Discord bot to `pattern-discord`
4. Update imports and dependencies
5. Ensure all tests still pass

### Phase 3: Remove Letta (Week 4)
1. Replace Letta agent calls with pattern-core
2. Migrate memory system
3. Implement agent coordination
4. Update persistence layer
5. Full integration testing

### Phase 4: Polish (Week 5)
1. Documentation updates
2. Example applications
3. Performance optimization
4. Error handling improvements

## Missing Components to Build

### From Letta functionality we need to replace:
1. **Agent State Management**
   - Conversation history
   - Memory updates
   - Tool call tracking
   
2. **Message Routing**
   - Agent selection logic
   - Group coordination
   - Partner/user context switching
   
3. **Memory Search**
   - Semantic search over memory blocks
   - Relevance scoring
   - Memory compression
   
4. **Tool Execution**
   - Parameter validation
   - Result formatting
   - Error propagation

### New functionality to add:
1. **Streaming Responses**
   - Real-time token streaming
   - Progress indicators
   - Partial results
   
2. **Cost Tracking**
   - Per-agent token usage
   - Model cost calculation
   - Budget limits
   
3. **Agent Lifecycle**
   - Creation/deletion
   - State persistence
   - Hot reload of prompts

## Database Schema with SurrealDB

SurrealDB's multi-model approach gives us incredible flexibility:

```surql
-- Users with multi-tenancy support
DEFINE TABLE users SCHEMAFULL;
DEFINE FIELD username ON TABLE users TYPE string;
DEFINE FIELD discord_id ON TABLE users TYPE string;
DEFINE INDEX idx_discord ON TABLE users COLUMNS discord_id UNIQUE;

-- Agents as graph nodes
DEFINE TABLE agents SCHEMAFULL;
DEFINE FIELD name ON TABLE agents TYPE string;
DEFINE FIELD type ON TABLE agents TYPE string;
DEFINE FIELD system_prompt ON TABLE agents TYPE string;
DEFINE FIELD model_tier ON TABLE agents TYPE string;
DEFINE FIELD embedding ON TABLE agents TYPE array<float>;
DEFINE FIELD owner ON TABLE agents TYPE record<users>;
DEFINE INDEX idx_embedding ON TABLE agents COLUMNS embedding VECTOR DIMENSION 1536;

-- Agent groups for coordination
DEFINE TABLE agent_groups SCHEMAFULL;
DEFINE FIELD name ON TABLE agent_groups TYPE string;
DEFINE FIELD coordination_pattern ON TABLE agent_groups TYPE string;
DEFINE FIELD owner ON TABLE agent_groups TYPE record<users>;

-- Relationships between agents
DEFINE TABLE belongs_to SCHEMAFULL;
DEFINE FIELD in ON TABLE belongs_to TYPE record<agents>;
DEFINE FIELD out ON TABLE belongs_to TYPE record<agent_groups>;
DEFINE FIELD role ON TABLE belongs_to TYPE string;

-- Conversations with real-time support
DEFINE TABLE conversations SCHEMAFULL;
DEFINE FIELD started_at ON TABLE conversations TYPE datetime DEFAULT time::now();
DEFINE FIELD context ON TABLE conversations TYPE object;
DEFINE EVENT conversation_activity ON TABLE conversations WHEN $event = "CREATE" OR $event = "UPDATE" THEN {};

-- Messages with vector embeddings
DEFINE TABLE messages SCHEMAFULL;
DEFINE FIELD conversation ON TABLE messages TYPE record<conversations>;
DEFINE FIELD agent ON TABLE messages TYPE record<agents>;
DEFINE FIELD role ON TABLE messages TYPE string;
DEFINE FIELD content ON TABLE messages TYPE string;
DEFINE FIELD embedding ON TABLE messages TYPE array<float>;
DEFINE FIELD tool_calls ON TABLE messages TYPE array<object>;
DEFINE FIELD created_at ON TABLE messages TYPE datetime DEFAULT time::now();
DEFINE INDEX idx_msg_embedding ON TABLE messages COLUMNS embedding VECTOR DIMENSION 1536;

-- Memory blocks with semantic search
DEFINE TABLE memories SCHEMAFULL;
DEFINE FIELD agent ON TABLE memories TYPE record<agents>;
DEFINE FIELD key ON TABLE memories TYPE string;
DEFINE FIELD content ON TABLE memories TYPE string;
DEFINE FIELD embedding ON TABLE memories TYPE array<float>;
DEFINE FIELD metadata ON TABLE memories TYPE object;
DEFINE FIELD updated_at ON TABLE memories TYPE datetime DEFAULT time::now();
DEFINE INDEX idx_mem_embedding ON TABLE memories COLUMNS embedding VECTOR DIMENSION 1536;
DEFINE INDEX idx_agent_key ON TABLE memories COLUMNS agent, key UNIQUE;

-- Tasks with graph relationships
DEFINE TABLE tasks SCHEMAFULL;
DEFINE FIELD title ON TABLE tasks TYPE string;
DEFINE FIELD description ON TABLE tasks TYPE string;
DEFINE FIELD owner ON TABLE tasks TYPE record<users>;
DEFINE FIELD assigned_agent ON TABLE tasks TYPE record<agents>;
DEFINE FIELD status ON TABLE tasks TYPE string;
DEFINE FIELD estimated_time ON TABLE tasks TYPE duration;
DEFINE FIELD actual_time ON TABLE tasks TYPE duration;

-- Parent-child task relationships
DEFINE TABLE subtask_of SCHEMAFULL;
DEFINE FIELD in ON TABLE subtask_of TYPE record<tasks>;
DEFINE FIELD out ON TABLE subtask_of TYPE record<tasks>;

-- Energy states as time-series data
DEFINE TABLE energy_states SCHEMAFULL;
DEFINE FIELD user ON TABLE energy_states TYPE record<users>;
DEFINE FIELD energy_level ON TABLE energy_states TYPE number;
DEFINE FIELD attention_state ON TABLE energy_states TYPE string;
DEFINE FIELD interruptible ON TABLE energy_states TYPE bool;
DEFINE FIELD timestamp ON TABLE energy_states TYPE datetime DEFAULT time::now();
```

## Configuration Structure

```toml
# pattern.toml
[workspace]
database_url = "sqlite://pattern.db"

[models]
default_provider = "anthropic"
default_model = "claude-3-haiku-20240307"

[models.providers.anthropic]
api_key_env = "ANTHROPIC_API_KEY"

[models.providers.openai]
api_key_env = "OPENAI_API_KEY"

[models.tiers]
routine = "claude-3-haiku-20240307"
interactive = "claude-3-5-sonnet-20241022"
investigative = "gpt-4"
critical = "claude-3-opus-20240229"

[discord]
token_env = "DISCORD_TOKEN"
prefix = "!"

[mcp]
transport = "sse"
port = 8080
```

## Nix Changes Required

1. Update `flake.nix` to handle workspace structure
2. Add separate derivations for each crate
3. Update development shell with all dependencies
4. Add build scripts for individual crates
5. Update CI/CD workflows

## Benefits of This Architecture

1. **Modularity**: Each crate has a single responsibility
2. **Testability**: Easier to unit test individual components
3. **Reusability**: Other projects can use pattern-core
4. **Flexibility**: Can swap out components (e.g., different Discord lib)
5. **Performance**: Only load what you need
6. **Maintainability**: Clear boundaries between systems

## Open Questions

1. Should we use `sqlx` or `diesel` for database abstraction?
2. Do we want to support multiple database backends or just SQLite?
3. Should pattern-core include a default memory implementation or just traits?
4. How do we handle agent personality configuration - in pattern-nd or pattern-core?
5. Do we want to support other chat platforms besides Discord in the future?
6. Should we implement our own vector search or use an existing solution?

## Implementation Details & Missing Components

### Agent Framework (pattern-core)

**Agent Trait Design**:
```rust
#[async_trait]
pub trait Agent: Send + Sync {
    async fn process_message(&self, message: Message) -> Result<Response>;
    async fn get_memory(&self, key: &str) -> Result<Option<String>>;
    async fn update_memory(&self, key: &str, value: String) -> Result<()>;
    async fn execute_tool(&self, tool: &str, params: Value) -> Result<Value>;
    fn get_system_prompt(&self) -> &str;
    fn get_capabilities(&self) -> Vec<String>;
}
```

**Missing from current implementation**:
- Conversation context window management
- Token counting for context limits
- Automatic summarization when context exceeds limits
- Tool result formatting for AI consumption
- Multi-turn conversation state tracking
- Agent "working memory" separate from long-term memory
- Graph-based agent relationships
- Real-time agent coordination via SurrealDB LIVE queries
- Vector similarity search for semantic memory

### Tool Framework Considerations

**Tool Registration Pattern** (inspired by luts):
```rust
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Value;
    async fn execute(&self, params: Value) -> Result<Value>;
}

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}
```

**Missing tools to implement**:
- Semantic search over memory/documents
- Multi-step planning tools
- Inter-agent communication tools
- State checkpoint/restore tools
- Batch operations for efficiency

### Memory System Architecture

**Three-tier memory hierarchy with SurrealDB**:
1. **Working Memory** (SurrealDB with TTL)
   - Current conversation context
   - Recent tool results (with expiry)
   - Active task state
   - Real-time updates via LIVE queries
   
2. **Long-term Memory** (SurrealDB persistent)
   - User facts and preferences
   - Historical conversations with embeddings
   - Learned patterns via graph relationships
   - Vector search for semantic retrieval
   
3. **Archival Storage** (SurrealDB + external)
   - Full conversation logs
   - Large documents (stored as references)
   - Embedded knowledge bases
   - Time-series data for pattern analysis

**Missing components**:
- Memory compression algorithms
- Relevance decay over time
- Cross-agent memory sharing protocol
- Memory conflict resolution
- Privacy controls for memory access

### Agent Coordination Without Letta

**Group Coordination Patterns in Core**:
```rust
// pattern-core/src/coordination.rs
pub enum CoordinationPattern {
    Supervisor { leader: AgentId },
    RoundRobin { agents: Vec<AgentId> },
    Voting { quorum: usize },
    Pipeline { stages: Vec<AgentId> },
    Dynamic { selector: Box<dyn AgentSelector> },
    Sleeptime { check_interval: Duration, triggers: Vec<Trigger> },
}

// Multi-agent constellation management
pub struct Constellation {
    id: String,
    owner: UserId,
    agents: HashMap<String, Agent>,
    groups: HashMap<String, AgentGroup>,
    memory_pool: SharedMemory,
    db: SurrealDB,
}

impl Constellation {
    pub async fn route_message(&self, msg: Message) -> Result<Response> {
        // Smart routing based on content, context, and agent capabilities
    }
    
    pub async fn coordinate(&self, pattern: CoordinationPattern) -> Result<()> {
        // Execute coordination pattern across agents
    }
}
```

**Missing from current system**:
- Agent election algorithms
- Consensus mechanisms
- Task delegation protocols
- Result aggregation strategies
- Failure recovery patterns

### Model Integration Details

**genai crate integration**:
- Need to map our capability tiers to genai's model abstraction
- Implement retry logic with exponential backoff
- Add request/response caching layer
- Token usage tracking per request
- Model fallback chains (if primary fails, try secondary)

**Missing model features**:
- Function/tool calling support validation
- Structured output parsing (JSON mode)
- Image/multimodal support detection
- Streaming response handling
- Rate limit management

### Persistence Layer Decisions

**Why SurrealDB is perfect for Pattern**:
- **Multi-model**: Documents for configs, graph for relationships, vectors for memory
- **Built-in vector search**: No separate vector DB needed
- **Real-time subscriptions**: LIVE queries for agent coordination
- **Graph traversal**: Perfect for agent relationships and memory connections
- **Time-series support**: Energy/attention tracking over time
- **Multi-tenancy**: Built-in user isolation
- **Unified query language**: Combine vector, graph, and relational in one query

**SurrealDB-specific features we'll use**:
```surql
-- Find similar memories across agent constellation
SELECT *, vector::similarity::cosine(embedding, $query_embedding) AS score
FROM memories
WHERE agent.owner = $user
ORDER BY score DESC
LIMIT 10
FETCH agent;

-- Real-time agent coordination
LIVE SELECT * FROM messages
WHERE conversation.agents CONTAINS $agent_id
AND created_at > time::now() - 5m;

-- Graph traversal for task breakdown
SELECT ->subtask_of->tasks AS subtasks
FROM $parent_task
FETCH subtasks.assigned_agent;
```

### Testing Strategy for Multi-Crate

**Test organization**:
- Unit tests in each crate
- Integration tests in pattern crate
- Mock implementations in pattern-core for testing
- Property-based testing for agent behaviors
- Benchmark suite for performance regression

**Missing test infrastructure**:
- Mock AI provider for deterministic tests
- Conversation replay system
- Load testing framework
- Chaos testing for agent failures

### Performance Considerations

**Caching layers needed**:
- Model response cache (with TTL)
- Tool result cache
- Compiled prompt template cache
- Database query result cache

**Async considerations**:
- Use tokio for all async runtime
- Careful about spawning too many tasks
- Connection pooling for database
- Rate limiting for external APIs

### Error Handling Philosophy

**Error types per crate**:
```rust
// pattern-core
pub enum CoreError {
    AgentError(String),
    ToolError(String),
    MemoryError(String),
    ModelError(String),
}

// Each crate has its own error type
// Main crate combines them
```

**Missing error handling**:
- Graceful degradation strategies
- Circuit breakers for failing services
- Error recovery workflows
- User-friendly error messages

## Next Steps

1. Review and refine this plan
2. Set up the workspace structure
3. Start with pattern-core implementation
4. Gradually migrate existing code
5. Remove Letta dependencies
6. Update documentation

## Workspace Structure

```
pattern/
├── Cargo.toml                 # Workspace root
├── pattern-core/              # Core agent framework
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs
│   │   ├── agent.rs           # Agent trait and base impl
│   │   ├── memory.rs          # Memory abstractions with vector search
│   │   ├── tool.rs            # Tool framework
│   │   ├── model.rs           # genai integration
│   │   ├── coordination.rs    # Multi-agent patterns & constellations
│   │   ├── db.rs              # SurrealDB integration
│   │   └── realtime.rs        # LIVE query subscriptions
│   └── tests/
├── pattern-nd/                # Neurodivergent tools
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs
│   │   ├── agents/            # ADHD agent personalities
│   │   ├── tools/             # ADHD-specific tools
│   │   └── sleeptime.rs       # Background monitoring
│   └── tests/
├── pattern-mcp/               # MCP server
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs
│   │   ├── server.rs          # MCP server impl
│   │   ├── transport/         # stdio, HTTP, SSE
│   │   └── registry.rs        # Tool registration
│   └── tests/
├── pattern-discord/           # Discord bot
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs
│   │   ├── bot.rs             # Discord bot impl
│   │   ├── commands.rs        # Slash commands
│   │   └── context.rs         # Discord context mgmt
│   └── tests/
└── pattern/                   # Main orchestrator
    ├── Cargo.toml
    ├── src/
    │   ├── lib.rs
    │   ├── main.rs            # Binary entry point
    │   ├── config.rs          # Configuration
    │   ├── db.rs              # Database setup
    │   └── service.rs         # Service orchestration
    ├── migrations/            # SQL migrations
    └── tests/
```

## Feature Flag Strategy

```toml
# pattern/Cargo.toml
[features]
default = ["sqlite", "discord", "mcp"]
full = ["sqlite", "postgres", "discord", "mcp", "nd"]
sqlite = ["pattern-core/sqlite"]
postgres = ["pattern-core/postgres"]
discord = ["pattern-discord"]
mcp = ["pattern-mcp"]
nd = ["pattern-nd"]
```

This allows users to compile only what they need while maintaining flexibility.

## Final Considerations

### Why Remove Letta?

1. **Cloud API Limitations**: Constant agent recreation hitting rate limits
2. **Inflexibility**: Hard to customize agent behaviors beyond their model
3. **Performance**: Extra network hop for every agent interaction
4. **Cost**: Paying for their cloud service when we can run models directly
5. **Control**: Need fine-grained control over memory, tools, and coordination

### Advantages of Our Approach

1. **Direct Model Access**: Use any model provider through genai
2. **Custom Memory**: Implement exactly the memory system we need
3. **Flexible Tools**: Easy to add new tools without their constraints
4. **Better Testing**: Can mock at any level for deterministic tests
5. **Cost Efficiency**: Only pay for model API calls, not middleware

### Risks to Mitigate

1. **Complexity**: Building our own agent framework is non-trivial
2. **Maintenance**: We own all the code and bugs
3. **Feature Parity**: Letta has features we might miss initially
4. **Time Investment**: Significant upfront development time

### Immediate Action Items

1. **Set up workspace structure**:
   ```bash
   mkdir -p pattern-{core,nd,mcp,discord}/{src,tests}
   ```

2. **Create workspace Cargo.toml**:
   ```toml
   [workspace]
   members = ["pattern-core", "pattern-nd", "pattern-mcp", "pattern-discord", "pattern"]
   resolver = "2"
   ```

3. **Start with pattern-core**:
   - Define Agent trait
   - Implement basic memory system
   - Create tool framework
   - Add genai integration

4. **Migrate incrementally**:
   - Keep existing code working
   - Replace Letta calls one by one
   - Run tests after each change

5. **Update Nix flake**:
   - Add workspace support
   - Update build commands
   - Fix development shell

### Success Metrics

- All existing tests pass after migration
- Response time improves by 30%+
- Token costs decrease by 20%+
- Agent coordination is more flexible
- Easier to add new agent types
- Better debugging and observability

### Timeline Estimate

- **Week 1-2**: Core framework and basic agents
- **Week 3**: Extract existing code to new crates
- **Week 4**: Remove Letta and integrate everything
- **Week 5**: Polish, optimize, and document

Total: ~5 weeks for full migration

### Decision Points

Before we start, we should decide:

1. **Database**: SurrealDB as primary backend (handles everything we need)
2. **Async Runtime**: Stick with tokio (SurrealDB SDK uses it)
3. **Model Providers**: Start with Anthropic/OpenAI via genai
4. **Memory Search**: Use SurrealDB's built-in vector search
5. **Configuration**: TOML for static config, SurrealDB for dynamic

### Let's Do This

This refactor will give us full control over Pattern's architecture and remove external dependencies that have been causing issues. The multi-crate structure will make the codebase more maintainable and allow others to use pattern-core for their own agent systems.

**Key advantages with SurrealDB**:
- Single database for all data types (no need for separate vector/graph/relational DBs)
- Real-time coordination between agents via LIVE queries
- Graph relationships make agent constellations natural
- Built-in vector search for semantic memory
- Time-series support for ADHD tracking
- Multi-tenancy for partner isolation

Ready to start with the workspace setup?