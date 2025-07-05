# Cache Refactor Plan: SQLite KV → Foyer

## Overview

Refactor the current SQLite-based caching to use Foyer, a hybrid in-memory/disk cache library. This will improve performance, reduce database load, and provide better cache management.

## Current State

Currently using SQLite as a key-value store:
- **Agents**: `agents` table with `letta_agent_id` column
- **Groups**: `groups` table with `group_data` JSON column (full Letta Group struct)
- **Memory blocks**: `shared_memory` table with block values

### Problems with Current Approach
- No automatic eviction - cache grows indefinitely
- Every operation hits the database
- Manual cache invalidation required
- No memory/disk tiering
- Limited observability into cache performance

## Target Architecture

### Dependencies
```toml
# Cargo.toml
foyer = { version = "0.12", features = ["serde"] }
```

### Cache Module Structure
```rust
// src/cache.rs
use foyer::{DirectFsDeviceOptions, Engine, HybridCache, HybridCacheBuilder};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedAgent {
    pub letta_id: String,
    pub name: String,
    pub system_prompt_hash: String, // For detecting changes
    pub tools: Vec<String>,          // For quick tool checks
    pub last_accessed: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedGroup {
    pub letta_group: letta::types::Group,
    pub agent_ids: Vec<String>,
    pub last_accessed: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedMemoryBlock {
    pub block_value: String,
    pub letta_block_id: Option<String>,
    pub max_length: usize,
    pub last_modified: chrono::DateTime<Utc>,
}

pub struct PatternCache {
    // user_id + agent_type -> CachedAgent
    agents: HybridCache<String, CachedAgent>,
    
    // user_id + group_name -> CachedGroup  
    groups: HybridCache<String, CachedGroup>,
    
    // user_id + block_name -> CachedMemoryBlock
    memory_blocks: HybridCache<String, CachedMemoryBlock>,
    
    // Optional: Message history cache
    // agent_id -> last N messages
    message_history: HybridCache<String, Vec<LettaMessage>>,
}
```

### Cache Configuration
```rust
impl PatternCache {
    pub async fn new(cache_dir: &Path) -> Result<Self> {
        // Agents: Hot cache, accessed on every message
        let agents = HybridCacheBuilder::new()
            .memory(64 * 1024 * 1024)  // 64MB in-memory
            .storage(Engine::Large)     // Optimized for larger values
            .with_device_options(
                DirectFsDeviceOptions::new(cache_dir.join("agents"))
                    .with_capacity(256 * 1024 * 1024)  // 256MB on disk
            )
            .build()
            .await?;
            
        // Groups: Medium access frequency
        let groups = HybridCacheBuilder::new()
            .memory(32 * 1024 * 1024)  // 32MB in-memory
            .storage(Engine::Small)     // Smaller values
            .with_device_options(
                DirectFsDeviceOptions::new(cache_dir.join("groups"))
                    .with_capacity(128 * 1024 * 1024)  // 128MB on disk
            )
            .build()
            .await?;
            
        // Memory blocks: Frequently updated
        let memory_blocks = HybridCacheBuilder::new()
            .memory(128 * 1024 * 1024)  // 128MB in-memory
            .storage(Engine::Large)
            .with_device_options(
                DirectFsDeviceOptions::new(cache_dir.join("memory"))
                    .with_capacity(512 * 1024 * 1024)  // 512MB on disk
            )
            .build()
            .await?;
            
        Ok(Self { agents, groups, memory_blocks })
    }
}
```

### Key Generation Strategy
```rust
// Consistent, collision-free keys
fn agent_key(user_id: i64, agent_type: &str) -> String {
    format!("u{}:a:{}", user_id, agent_type)
}

fn group_key(user_id: i64, group_name: &str) -> String {
    format!("u{}:g:{}", user_id, group_name)
}

fn memory_key(user_id: i64, block_name: &str) -> String {
    format!("u{}:m:{}", user_id, block_name)
}

fn message_history_key(agent_id: &str) -> String {
    format!("msg:{}", agent_id)
}
```

## Migration Plan

### Phase 1: Add Foyer Infrastructure (No Breaking Changes) ✅
1. ✅ Add foyer dependency (v0.17 with serde feature)
2. ✅ Create `src/cache.rs` module with hybrid caches
3. ✅ Add to lib.rs exports
4. ❌ Add cache initialization to `PatternService` (next phase)
5. ❌ Add feature flag `foyer-cache` for gradual rollout (decided not needed)

### Phase 2: Agent Cache Migration (NEXT)
1. Add `cache: Arc<PatternCache>` to `MultiAgentSystem`
2. Initialize cache in `MultiAgentSystemBuilder::build()`
3. Update `create_or_get_agent_with_list()` to:
   - Check cache first using `cache.get_agent()`
   - Verify system prompt hash matches
   - Update cache on miss with `cache.insert_agent()`
   - Store agent in DB only if not cached
4. Update `update_agent()` to invalidate cache
5. Add `cache.hash_string()` usage for system prompt comparison

### Phase 3: Group Cache Migration
1. Update `get_or_create_group()`:
   - Check `cache.get_group()` first
   - On miss, try DB then Letta API
   - Update cache with `cache.insert_group()`
2. Update `delete_group()` to invalidate cache
3. Remove `group_data` JSON caching from DB writes

### Phase 4: Memory Block Cache
1. Add cache checks to `get_shared_memory_block()`
2. Update `upsert_shared_memory_with_letta()` to update cache
3. Implement write-through with `cache.update_memory_block_value()`

### Phase 5: SQLite Cleanup
1. Remove `upsert_agent()` calls (cache handles tracking)
2. Stop writing to `group_data` column
3. Create migration to drop cache columns
4. Keep only core relationships in SQLite

## Benefits

1. **Performance**
   - Zero-copy in-memory access
   - Automatic hot/cold tiering
   - Parallel cache operations

2. **Resource Management**
   - Configurable memory limits
   - Automatic eviction (LRU/LFU)
   - Disk space bounds

3. **Observability**
   - Built-in metrics (hits/misses/evictions)
   - Prometheus/OpenTelemetry support
   - Cache efficiency tracking

4. **Operational**
   - No manual cache cleanup
   - Graceful degradation on cache miss
   - Persistent cache across restarts

## Future Enhancements

### Aggressive Caching Opportunities
- **Message History**: Cache last N messages per agent (reduce Letta API calls)
- **Tool Results**: Cache deterministic tool outputs (e.g., `list_knowledge_files`)
- **Query Patterns**: Cache common query results (e.g., "get all agents for user")
- **Computed Values**: Cache expensive computations (e.g., agent capability checks)

### Advanced Features
- **Cache Warming**: Pre-load active users on startup
- **TTL Support**: Time-based expiration for certain data
- **Cache Hierarchies**: L1 (process memory) → L2 (foyer) → L3 (SQLite)
- **Distributed Cache**: Redis/KeyDB for multi-instance deployments

## Implementation Notes

### Key API Differences from Plan
- `HybridCache::get()` returns `Result<Option<CacheEntry>>`, not just the value
- Need to call `entry.value()` to get the actual cached item
- `remove()` doesn't return the old value - need to get() first if needed
- Use `Engine::large()` and `Engine::small()` for storage configuration

### Next Session TODOs
1. Add cache field to MultiAgentSystem
2. Pass cache directory path through config
3. Update agent creation/lookup methods
4. Add metrics tracking for cache hit/miss rates

## Success Metrics
- Reduce Letta API calls by 80%+ for existing agents
- Sub-millisecond agent lookups for cached entries
- Reduce SQLite load for read operations
- Maintain cache hit rate >90% for active users