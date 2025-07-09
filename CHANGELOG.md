# Changelog

All notable changes to Pattern will be documented in this file.

## 2025-07-09

### Added
- Built-in tools system for agents (update_memory, send_message)
  - Type-safe `AiTool` trait with generics instead of `serde_json::Value`
  - `AgentHandle` provides cheap, cloneable access to agent internals
  - `BuiltinTools::builder()` allows customization of default implementations
  - Tools are registered in the same ToolRegistry as external tools
- Generic `DatabaseAgent<Connection, ModelProvider, EmbeddingProvider>`
  - Allows using different model providers (OpenAI, Anthropic, local, etc.)
  - Supports various embedding providers for semantic search
  - Added `MockModelProvider` and `MockEmbeddingProvider` for testing
- Thread-safe memory system improvements
  - Added `Arc<DashMap>` to Memory for efficient concurrent access
  - Memory blocks can be shared across threads without cloning data

### Changed
- Refactored `AgentContext` to separate concerns:
  - `AgentHandle`: Cheap, frequently accessed data (agent_id, memory)
  - `MessageHistory`: Large message vectors behind `Arc<RwLock<_>>`
  - Metadata behind `Arc<tokio::sync::RwLock<_>>` for concurrent updates
- ID format consistency: All TypedId implementations now use underscore separator
  - Format: `prefix_uuid` (e.g., `agent_12345678-...`, `user_abcdef12-...`)
  - MemoryIdType uses "mem" prefix, not "memory"
- Tool schemas are automatically inlined for MCP compatibility (no `$ref` fields)

### Fixed
- All test failures related to ID format changes
- Memory sharing issues in built-in tools
- Thread safety issues with agent metadata updates
- MCP schema generation to be fully compliant

## 2025-07-06 (Late Night Session)

### Added
- Multi-crate workspace structure for better modularity
  - `pattern-core`: Core agent framework, memory system, and tool registry
  - `pattern-nd`: ADHD-specific tools and agent personalities
  - `pattern-mcp`: MCP server implementation with multiple transports
  - `pattern-discord`: Discord bot functionality
  - `pattern-main`: Main orchestrator and integration
- SurrealDB embedded database with vector search capabilities
- Type-safe database operations with custom ID types (UserId, AgentId, etc.)
- Comprehensive error handling using miette for rich diagnostics
- Embedding providers with feature flags:
  - Candle (local) - Jina models only (BERT has dtype issues)
  - OpenAI - text-embedding-3-small/large
  - Cohere - embed-english-v3.0
  - Ollama - Stub implementation
- Automatic handling of SurrealDB's nested value format
- Database schema migrations with version tracking
- Custom serialization for AgentType enum with `custom:` prefix for custom variants
- Integration test suite for slow tests (Candle embeddings)
- Dummy embeddings (384-dim zeros) when no provider configured

### Changed
- Migrated from SQLite to SurrealDB for persistence
- Replaced SQLite KV cache with Foyer library (disk-backed async cache)
- Moved from monolithic structure to multi-crate workspace
- Removed all C dependencies (using SurrealKV instead of RocksDB, rustls instead of native TLS)
- Optimized test suite - unit tests now run in <1 second (moved slow tests to integration)
- Updated AgentType serialization to handle Custom variants as strings
- Improved database operations to be more direct (no repository pattern)
- Enhanced error messages with contextual help

### Fixed
- MCP tool schema issues (removed references, enums, nested structs, unsigned ints)
- Agent infinite loop issue when prefixing messages
- MCP tool name conflicts with Letta defaults
- SurrealDB value unwrapping for all wrapped types (Datetime, Thing, Strand, Bool, Number)
- BERT embedding models dtype issues (now documented, Jina models recommended)

### Security
- All database IDs now use strongly-typed wrappers to prevent ID mixing
- Removed hardcoded API keys from examples

## 2025-07-05

### Added
- Custom tiered sleeptime monitor with rules-based checks and Pattern intervention
- Knowledge sharing tools (write_agent_knowledge, read_agent_knowledge, sync_knowledge_to_letta)
- MCP tools for ADHD support:
  - `schedule_event` with database storage
  - `check_activity_state` with energy state tracking
  - `record_energy_state` for tracking ADHD energy/attention states
- Model capability abstraction system (Routine/Interactive/Investigative/Critical)
- Configurable model mappings in pattern.toml (global and per-agent)
- External agent configuration via AGENT_CONFIG_PATH

### Changed
- Optimized agent initialization to eliminate API calls for existing agents
- Database caching now uses multi-tier approach with agent IDs

### Fixed
- Agent update functionality (no more delete/recreate for cloud limits)

## 2025-07-04

### Added
- Native Letta groups with dynamic, supervisor, round-robin, and sleeptime managers
- Passive knowledge sharing via Letta sources
- Partner/conversant architecture for privacy
- Unified `send_message` tool for all communication

### Changed
- Discord bot now routes to groups based on message content
- Default groups created on user init: main, crisis, planning, memory

## 2025-07-03

### Added
- Multi-agent system with Pattern orchestrator + 5 specialist agents
- Discord bot with natural language and slash commands
- MCP server with 10+ tools
- Comprehensive documentation structure
- Library restructure with feature flags

### Changed
- Migrated from single agent to constellation architecture
- Improved error handling throughout

## 2025-07-01 - 2025-07-02

### Added
- Initial Pattern ADHD support system
- Basic Letta integration
- Simple Discord bot
- Core memory system