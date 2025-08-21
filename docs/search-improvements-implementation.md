# Search Improvements Implementation Plan

## Phase 1: Improve Search Without Index Rebuilds

### 1.1 Add BM25 Scoring to Existing Queries

Since our indexes already have BM25, we can use `search::score()` without rebuilding:

```rust
// Current query (no scoring)
"SELECT * FROM msg WHERE content @@ $search_query"

// Improved query with BM25 scoring
"SELECT *, search::score(1) AS relevance_score 
 FROM msg 
 WHERE content @@ $search_query 
 ORDER BY relevance_score DESC"
```

### 1.2 Add Fuzzy Search Options

Use SurrealDB's search operators for fuzzy matching:
- `@0@` - Exact match (default behavior of `@@`)
- `@1@` - Fuzzy with edit distance 1 (typos)
- `@2@` - Fuzzy with edit distance 2 (more typos)

```rust
// Add fuzzy parameter to search functions
pub async fn search_conversations(
    &self,
    query: Option<&str>,
    role_filter: Option<ChatRole>,
    // ... other params ...
    fuzzy_level: Option<u8>, // 0=exact, 1=fuzzy, 2=very fuzzy
) -> Result<Vec<Message>>
```

### 1.3 Implement Search Result Ranking

Create a new result type that includes relevance scores:

```rust
#[derive(Debug, Clone, Serialize)]
pub struct ScoredMessage {
    pub message: Message,
    pub score: f32,
    // Future: pub snippet: Option<String>, when HIGHLIGHTS enabled
}
```

## Phase 2: Future Enhancements (Requires Index Rebuild)

### 2.1 Enable Highlighting

When ready to rebuild indexes, add HIGHLIGHTS:

```sql
-- Future index with highlights
DEFINE INDEX msg_content_search ON msg 
    FIELDS content 
    SEARCH ANALYZER msg_content_analyzer 
    BM25 HIGHLIGHTS;
```

### 2.2 Add Snippet Extraction

Once HIGHLIGHTS is enabled:

```rust
"SELECT *, 
    search::score(1) AS relevance_score,
    search::highlight('<mark>', '</mark>', 1) AS snippet
 FROM msg 
 WHERE content @@ $search_query"
```

### 2.3 Migration Strategy for Adding HIGHLIGHTS

```rust
// Add to future migration (v3)
async fn migrate_v3_search_highlights<C: Connection>(db: &Surreal<C>) -> Result<()> {
    // Drop old indexes
    db.query("REMOVE INDEX msg_content_search ON msg").await?;
    db.query("REMOVE INDEX mem_value_search ON mem").await?;
    
    // Recreate with HIGHLIGHTS
    db.query("DEFINE INDEX msg_content_search ON msg 
              FIELDS content 
              SEARCH ANALYZER msg_content_analyzer 
              BM25 HIGHLIGHTS").await?;
    
    // ... recreate other indexes
}
```

## Phase 3: Implementation Code

### 3.1 Update Context State Search Methods

```rust
// In context/state.rs
impl AgentHandle {
    pub async fn search_conversations_ranked(
        &self,
        query: Option<&str>,
        role_filter: Option<ChatRole>,
        start_time: Option<DateTime<Utc>>,
        end_time: Option<DateTime<Utc>>,
        limit: usize,
        fuzzy_level: Option<u8>,
    ) -> Result<Vec<ScoredMessage>> {
        let db = self.db.as_ref().ok_or_else(/* ... */)?;
        
        // Build the search operator based on fuzzy level
        let search_op = match fuzzy_level.unwrap_or(0) {
            0 => "@@",     // Exact match (default)
            1 => "@1@",    // Fuzzy with edit distance 1
            2 => "@2@",    // Fuzzy with edit distance 2
            _ => "@@",
        };
        
        // Build query with scoring
        let mut conditions = vec![format!("content {} $search_query", search_op)];
        
        if role_filter.is_some() {
            conditions.push("role = $role".to_string());
        }
        // ... other conditions
        
        let sql = format!(
            "SELECT *, search::score(1) AS relevance_score
             FROM msg
             WHERE {}
             ORDER BY relevance_score DESC, created_at DESC
             LIMIT {}",
            conditions.join(" AND "),
            limit
        );
        
        // Execute and map results
        let mut result = db.query(&sql)
            .bind(("search_query", query))
            // ... other bindings
            .await?;
            
        #[derive(Deserialize)]
        struct ScoredResult {
            #[serde(flatten)]
            message: Message,
            relevance_score: f32,
        }
        
        let scored_results: Vec<ScoredResult> = result.take(0)?;
        
        // Convert to ScoredMessage
        Ok(scored_results.into_iter()
            .map(|r| ScoredMessage {
                message: r.message,
                score: r.relevance_score,
            })
            .collect())
    }
}
```

### 3.2 Update Search Tool

```rust
// In tool/builtin/search.rs
impl SearchTool {
    async fn search_conversations(
        &self,
        query: &str,
        role: Option<ChatRole>,
        start_time: Option<DateTime<Utc>>,
        end_time: Option<DateTime<Utc>>,
        limit: usize,
        fuzzy: bool, // New parameter
    ) -> Result<SearchOutput> {
        if self.handle.has_db_connection() {
            // Use fuzzy level 1 if fuzzy=true, 0 otherwise
            let fuzzy_level = if fuzzy { Some(1) } else { None };
            
            match self.handle
                .search_conversations_ranked(
                    Some(query), 
                    role, 
                    start_time, 
                    end_time, 
                    limit,
                    fuzzy_level
                )
                .await
            {
                Ok(scored_messages) => {
                    let results: Vec<_> = scored_messages
                        .into_iter()
                        .map(|sm| {
                            json!({
                                "id": sm.message.id,
                                "role": sm.message.role.to_string(),
                                "content": sm.message.display_content(),
                                "created_at": sm.message.created_at,
                                "relevance_score": sm.score,
                                // Future: "snippet": sm.snippet,
                            })
                        })
                        .collect();
                    
                    Ok(SearchOutput {
                        success: true,
                        message: Some(format!(
                            "Found {} messages matching '{}' (ranked by relevance)",
                            results.len(),
                            query
                        )),
                        results: json!(results),
                    })
                }
                Err(e) => Ok(SearchOutput {
                    success: false,
                    message: Some(format!("Search failed: {}", e)),
                    results: json!([]),
                }),
            }
        } else {
            // ... fallback
        }
    }
}
```

### 3.3 Add Search Configuration

```rust
// Add to SearchInput
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct SearchInput {
    pub domain: SearchDomain,
    pub query: String,
    pub limit: Option<i64>,
    pub role: Option<String>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    
    // New search options
    #[serde(default)]
    pub fuzzy: bool,  // Enable fuzzy matching
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_score: Option<f32>,  // Minimum relevance score
    
    #[serde(default)]
    pub request_heartbeat: bool,
}
```

## Phase 4: Constellation Search Optimization

### 4.1 Improved Constellation Search Query

Instead of complex nested queries, use a more efficient approach:

```rust
pub async fn search_constellation_messages_optimized(
    &self,
    query: Option<&str>,
    role_filter: Option<ChatRole>,
    limit: usize,
    fuzzy_level: Option<u8>,
) -> Result<Vec<(String, ScoredMessage)>> {
    // Get constellation agents in one query
    let agents_sql = r#"
        SELECT id, name FROM agent WHERE id IN (
            SELECT VALUE out FROM constellation_agents
            WHERE in = (
                SELECT VALUE <-constellation_agents<-constellation 
                FROM agent:$agent_id 
                LIMIT 1
            )
        )
    "#;
    
    // Search messages with scoring
    let search_op = match fuzzy_level.unwrap_or(0) {
        0 => "@@",
        1 => "@1@",
        2 => "@2@",
        _ => "@@",
    };
    
    let messages_sql = format!(r#"
        SELECT *, 
               search::score(1) AS relevance_score,
               in AS agent_id
        FROM agent_messages
        WHERE in IN (
            SELECT VALUE id FROM agent WHERE id IN (
                SELECT VALUE out FROM constellation_agents
                WHERE in = (
                    SELECT VALUE <-constellation_agents<-constellation 
                    FROM agent:$agent_id 
                    LIMIT 1
                )
            )
        )
        AND out.content {} $search_query
        ORDER BY relevance_score DESC, out.created_at DESC
        LIMIT $limit
    "#, search_op);
    
    // Execute and combine results
}
```

## Phase 5: Testing Strategy

### 5.1 Performance Benchmarks

```rust
#[cfg(test)]
mod bench {
    #[test]
    async fn benchmark_search_performance() {
        // Test with 10k messages
        let start = Instant::now();
        let results = handle.search_conversations_ranked(
            Some("test query"),
            None, None, None,
            100,
            Some(1), // fuzzy
        ).await.unwrap();
        
        assert!(start.elapsed() < Duration::from_millis(100));
        assert!(results[0].score > results[1].score); // Verify ranking
    }
}
```

### 5.2 Fuzzy Search Tests

```rust
#[test]
async fn test_fuzzy_search() {
    // Insert message with "configuration"
    // Search for "configration" (typo)
    let results = handle.search_conversations_ranked(
        Some("configration"),
        None, None, None, 10,
        Some(1), // fuzzy level 1
    ).await.unwrap();
    
    assert!(!results.is_empty());
    assert!(results[0].message.content.contains("configuration"));
}
```

## Implementation Priority

1. **Immediate** (No index changes needed):
   - Add BM25 scoring to queries
   - Implement fuzzy search operators
   - Return relevance scores with results

2. **Short-term** (Minor changes):
   - Add search configuration options
   - Optimize constellation search queries
   - Add performance benchmarks

3. **Future** (Requires index rebuild):
   - Add HIGHLIGHTS to indexes
   - Implement snippet extraction
   - Add search suggestions

## Benefits Without Index Rebuild

Even without HIGHLIGHTS, we get:
- **Relevance ranking** - Results sorted by BM25 score
- **Fuzzy matching** - Handle typos and variations  
- **Better performance** - Optimized queries
- **Configurable search** - Exact vs fuzzy modes
- **Score visibility** - Users see relevance scores

## Migration Path for HIGHLIGHTS

When ready to add highlighting:

1. Schedule maintenance window
2. Run migration to drop/recreate indexes with HIGHLIGHTS
3. Update queries to use `search::highlight()`
4. Add snippet field to result types
5. Update UI to show highlighted snippets

This approach gives immediate improvements while keeping the door open for highlighting later.