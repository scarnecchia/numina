# Search Improvements Using SurrealDB Full-Text Search

## Current Issues

### 1. Inefficient Content Search
- Current implementation uses `content @@ $search_query` (basic pattern matching)
- No ranking or relevance scoring
- Limited to exact substring matches
- No support for fuzzy matching or typos
- Returns results in chronological order rather than relevance

### 2. Complex Multi-Query Approach
- Conversation search with content requires filtering agent messages in code
- Constellation search requires multiple nested queries
- No database-level optimization for common search patterns

### 3. Limited Search Features
- No highlighting of matched terms
- No support for advanced search operators
- No phonetic or semantic matching
- No configurable analyzers for different use cases

## SurrealDB Full-Text Search Features

### Available Capabilities
1. **Analyzers**: Tokenization, stemming, stop words removal
2. **BM25 Scoring**: Relevance-based ranking
3. **Highlighting**: Extract snippets with matched terms
4. **Fuzzy Matching**: Handle typos and variations
5. **Phonetic Search**: Match similar-sounding words
6. **Custom Analyzers**: Define domain-specific search behavior

## Proposed Improvements

### Phase 1: Define Search Indexes

#### 1.1 Message Content Index
```sql
-- Create analyzer for message content
DEFINE ANALYZER message_analyzer TOKENIZERS blank, class, punct 
    FILTERS snowball(english), lowercase, ascii;

-- Define full-text index on messages
DEFINE INDEX idx_message_content ON TABLE msg 
    COLUMNS content 
    SEARCH ANALYZER message_analyzer 
    BM25 HIGHLIGHTS;
```

#### 1.2 Memory Block Index
```sql
-- Create analyzer for memory blocks
DEFINE ANALYZER memory_analyzer TOKENIZERS blank, class 
    FILTERS snowball(english), lowercase, ascii, edgengram(2,10);

-- Define index for archival memory
DEFINE INDEX idx_memory_value ON TABLE memory_block 
    COLUMNS value 
    SEARCH ANALYZER memory_analyzer 
    BM25 HIGHLIGHTS;
```

#### 1.3 Compound Indexes for Common Queries
```sql
-- Agent-specific message search
DEFINE INDEX idx_agent_messages ON TABLE agent_messages 
    COLUMNS in, out.content 
    SEARCH ANALYZER message_analyzer;

-- Constellation message search  
DEFINE INDEX idx_constellation_messages ON TABLE msg
    COLUMNS content, role, created_at
    SEARCH ANALYZER message_analyzer;
```

### Phase 2: Implement Enhanced Search Functions

#### 2.1 Conversation Search Improvements

```rust
pub async fn search_conversations_enhanced(
    &self,
    query: Option<&str>,
    role_filter: Option<ChatRole>,
    start_time: Option<DateTime<Utc>>,
    end_time: Option<DateTime<Utc>>,
    limit: usize,
    fuzzy: bool,           // New: Enable fuzzy matching
    highlights: bool,      // New: Return highlighted snippets
    min_score: Option<f32> // New: Minimum relevance score
) -> Result<Vec<SearchResult>> {
    let mut sql = String::from(
        "SELECT *, 
         search::score(1) AS score,
         search::highlight('<mark>', '</mark>', 1) AS snippet
         FROM msg"
    );
    
    let mut conditions = vec![];
    
    if let Some(q) = query {
        if fuzzy {
            conditions.push("content @1@ $search_query"); // Fuzzy search
        } else {
            conditions.push("content @0@ $search_query"); // Exact search
        }
    }
    
    // Add other filters...
    
    sql.push_str(" WHERE ");
    sql.push_str(&conditions.join(" AND "));
    sql.push_str(" ORDER BY score DESC, created_at DESC");
    sql.push_str(&format!(" LIMIT {}", limit));
    
    // Execute and process results with scores and snippets
}
```

#### 2.2 New SearchResult Type

```rust
#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub message: Message,
    pub score: f32,           // BM25 relevance score
    pub snippet: Option<String>, // Highlighted text snippet
    pub context: SearchContext,  // Surrounding messages/metadata
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchContext {
    pub batch_id: Option<SnowflakeId>,
    pub thread_depth: usize,
    pub agent_name: String,
    pub surrounding_messages: Vec<MessagePreview>,
}
```

### Phase 3: Advanced Search Features

#### 3.1 Multi-Field Search
```rust
// Search across multiple fields with different weights
pub async fn search_multi_field(
    &self,
    query: &str,
    fields: Vec<(&str, f32)>, // (field_name, weight)
    limit: usize
) -> Result<Vec<SearchResult>>
```

#### 3.2 Search Suggestions
```rust  
// Provide search suggestions based on partial input
pub async fn search_suggestions(
    &self,
    partial_query: &str,
    limit: usize
) -> Result<Vec<String>>
```

#### 3.3 Faceted Search
```rust
// Group results by categories
pub async fn search_with_facets(
    &self,
    query: &str,
    facets: Vec<String>, // e.g., ["role", "agent", "date_range"]
) -> Result<FacetedSearchResults>
```

### Phase 4: Constellation Search Optimization

#### 4.1 Materialized View for Constellation Messages
```sql
-- Create view that pre-joins constellation messages
DEFINE TABLE constellation_message_view AS 
    SELECT 
        msg.*,
        agent.name as agent_name,
        constellation.id as constellation_id
    FROM msg
    JOIN agent_messages ON msg.id = agent_messages.out
    JOIN agent ON agent.id = agent_messages.in
    JOIN constellation_agents ON agent.id = constellation_agents.out
    JOIN constellation ON constellation.id = constellation_agents.in
    GROUP BY msg.id;

-- Index the view for fast searching
DEFINE INDEX idx_constellation_view ON TABLE constellation_message_view
    COLUMNS content, agent_name
    SEARCH ANALYZER message_analyzer
    BM25 HIGHLIGHTS;
```

#### 4.2 Simplified Constellation Search
```rust
pub async fn search_constellation_messages_optimized(
    &self,
    query: &str,
    filters: SearchFilters,
    limit: usize
) -> Result<Vec<ConstellationSearchResult>> {
    // Single query against materialized view
    let sql = r#"
        SELECT *,
               search::score(1) AS score,
               search::highlight('<mark>', '</mark>', 1) AS snippet
        FROM constellation_message_view
        WHERE constellation_id = $constellation_id
        AND content @@ $query
        ORDER BY score DESC
        LIMIT $limit
    "#;
    
    // Direct execution with relevance ranking
}
```

### Phase 5: Search Analytics

#### 5.1 Track Search Performance
```rust
pub struct SearchMetrics {
    pub query: String,
    pub execution_time: Duration,
    pub results_count: usize,
    pub avg_score: f32,
    pub filters_used: Vec<String>,
}
```

#### 5.2 Popular Searches Cache
```rust
// Cache frequently searched queries
pub struct SearchCache {
    popular_queries: DashMap<String, CachedResults>,
    ttl: Duration,
}
```

## Implementation Plan

### Step 1: Database Schema Updates (2-3 hours)
- [ ] Add search indexes to migration
- [ ] Create analyzers for different content types
- [ ] Test index performance with sample data

### Step 2: Core Search Refactor (4-6 hours)
- [ ] Create SearchResult and SearchContext types
- [ ] Update AgentHandle search methods
- [ ] Implement BM25 scoring and highlighting
- [ ] Add fuzzy matching support

### Step 3: Tool Updates (2-3 hours)
- [ ] Update SearchTool with new parameters
- [ ] Add search configuration options
- [ ] Update tool examples and documentation

### Step 4: Constellation Optimization (3-4 hours)
- [ ] Create materialized views or optimized queries
- [ ] Implement single-query constellation search
- [ ] Test performance with large constellations

### Step 5: Testing & Documentation (2-3 hours)
- [ ] Write comprehensive tests
- [ ] Benchmark search performance
- [ ] Document new search features
- [ ] Update CLI help text

## Expected Benefits

1. **Performance**: 5-10x faster search with indexes
2. **Relevance**: Results ranked by relevance, not just time
3. **User Experience**: Typo tolerance, highlighting, suggestions
4. **Scalability**: Efficient search even with millions of messages
5. **Flexibility**: Support for complex queries and filters

## Migration Considerations

1. **Backward Compatibility**: Keep old search methods temporarily
2. **Index Building**: May take time for large existing databases
3. **Testing**: Extensive testing needed before production
4. **Monitoring**: Track search performance and usage patterns

## Success Metrics

- Search latency < 100ms for 95% of queries
- Relevant results in top 3 for 80% of searches  
- Support for databases with 1M+ messages
- Zero downtime migration from old search system