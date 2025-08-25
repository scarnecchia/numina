# Bluesky Cache & Deduplication Design

## Goals

1. **Reduce notification spam** - When multiple related posts arrive within seconds, send one comprehensive notification instead of many
2. **Minimize API calls** - Cache thread context and post data to avoid redundant fetches
3. **Preserve completeness** - Ensure notifications include full context even when batching

## Current State

### Already Implemented
- **Batching Layer**: `PendingBatch` with DashMap collections for posts by thread/author
- **Basic Cache Structure**: `thread_cache: Arc<DashMap<String, CachedThreadContext>>`
- **TTL Fields**: `thread_cache_ttl: Duration` configured at 5 minutes
- **Processing Tracking**: `processed_uris: DashMap<String, Instant>` for deduplication

### Needs Implementation
- **Cache utilization**: Thread cache exists but isn't being populated or checked
- **Type conversions**: Ad-hoc `extract_*` functions need consolidation
- **Embed improvements**: BlueskyPost uses raw atrium types for embeds
- **Batch formatting**: Still sending individual posts instead of consolidated notifications

## Type System Cleanup

### Current Issues

1. **BlueskyPost embed field**: Uses raw `Union<atrium_api::app::bsky::feed::post::RecordEmbedRefs>` 
   - Should use our cleaner `EmbedInfo` enum instead
   - Already have `extract_embed_info()` that produces it

2. **Scattered conversion functions**:
   - `extract_post_from_record()` - Converts RecordData to parts
   - `extract_post_data()` - Extracts from PostView
   - `extract_embed()` - Complex embed handling
   - `extract_embed_info()` - Creates EmbedInfo from PostView
   - `extract_record_embed()` - Handles RecordEmbedRefs
   - `extract_image_alt_texts()` - Gets alt text from embeds
   - `extract_post_relative_time()` - Formats time

3. **Missing standard conversions**: No From/TryFrom impls for common conversions

### Proposed BlueskyPost Improvements

```rust
pub struct BlueskyPost {
    pub uri: String,
    pub did: String,
    pub cid: Cid,
    pub handle: String,
    pub display_name: Option<String>,
    pub text: String,
    pub created_at: DateTime<Utc>,
    pub reply: Option<ReplyRef>,
    pub embed: Option<EmbedInfo>,  // Changed from raw Union type
    pub langs: Vec<String>,
    pub labels: Vec<String>,
    pub facets: Vec<Facet>,
    // New fields for cache efficiency:
    pub thread_root: Option<String>,  // Cached thread root
    pub indexed_at: Option<DateTime<Utc>>,  // When Bluesky indexed it
}

impl BlueskyPost {
    // Convert from firehose event (minimal data)
    pub fn from_record(record: &RecordData, uri: &str, did: &str) -> Self { }
    
    // Convert from API response (full data)
    pub fn from_post_view(post: &PostView) -> Self { }
    
    // Enrich minimal post with API data
    pub async fn enrich(&mut self, post_view: &PostView) { }
    
    // Get thread root (compute if needed, cache result)
    pub fn thread_root(&self) -> String { }
}
```

## Cache Utilization

### Existing Infrastructure

```rust
// Already in BlueskyFirehoseSource:
pending_batch: Arc<PendingBatch>,  // Batching posts
thread_cache: Arc<DashMap<String, CachedThreadContext>>,  // Ready to use
thread_cache_ttl: Duration,  // 5 minute TTL

// CachedThreadContext already defined:
struct CachedThreadContext {
    context: ThreadContext,  // Full thread with siblings, replies, engagement
    cached_at: Instant,
    thread_root: String,
}
```

### What Needs Implementation

1. **Populate cache on fetch**:
   - When `build_thread_context()` fetches from constellation
   - Store result in `thread_cache` with current Instant
   
2. **Check cache before fetch**:
   - In `format_notification()`, check `thread_cache` first
   - If exists and fresh (< 5 min), use cached context
   - If stale or missing, fetch and update cache

3. **Cache invalidation**:
   - Check TTL on read: `if now - cached_at > thread_cache_ttl`
   - Optional: Periodic cleanup task for expired entries

### Cache Operations

1. **On Jetstream Event Arrival**
   ```
   - Convert RecordData → basic BlueskyPost
   - Check if thread_root exists in cache
   - If cached and fresh: use it
   - If not: add to batch for later fetch
   ```

2. **On Batch Flush**
   ```
   - Collect all unique thread_roots in batch
   - Fetch missing contexts in parallel
   - Cache all fetched data
   - Build consolidated notification
   ```

3. **Cache Invalidation**
   ```
   - TTL expiration (5 min for threads, 10 min for posts)
   - Manual invalidation on user actions
   - Memory pressure limits
   ```

## Deduplication Strategy

### Within Batch
- Posts with same URI are merged (keep latest)
- Multiple posts from same thread are grouped
- Author's posts are kept in chronological order

### Across Batches
- Check `processed_uris` before creating notifications
- Skip if recently processed (within 30 seconds)
- But refresh cache if context is stale

### Notification Consolidation

For a batch containing multiple posts:

```
Thread: [root post]
├─ @user1: "first reply"
├─ @user2: "second reply"
└─ @user1: "third reply"

Becomes one notification:
"Thread activity on [topic]:
 - user1 replied (2 messages)
 - user2 replied
 [Show full thread context]"
```

## Implementation Plan

### Phase 1: Type System Cleanup ✨ Priority
1. Change `BlueskyPost.embed` from raw Union to `Option<EmbedInfo>`
2. Add `thread_root` and `indexed_at` fields to BlueskyPost
3. Consolidate `extract_*` functions into BlueskyPost methods:
   - `from_record()` - Create from firehose RecordData
   - `from_post_view()` - Create from API PostView 
   - `thread_root()` - Compute/cache thread root

### Phase 2: Cache Integration 
1. Modify `build_thread_context()` to populate `thread_cache`
2. Update `format_notification()` to check cache before fetching
3. Add TTL checking logic for cache entries

### Phase 3: Batch Consolidation
1. Enhance `format_batch_notification()` to actually consolidate posts
2. Create thread summary from multiple posts in batch
3. Aggregate engagement metrics across batch

### Phase 4: Testing & Metrics
1. Add tests for conversions and cache operations
2. Add cache hit/miss logging
3. Monitor API call reduction

## Benefits

1. **Performance**: 
   - 70-90% reduction in constellation API calls
   - Faster notification generation
   - Lower memory usage from deduplication

2. **User Experience**:
   - Cleaner notifications (one per thread burst)
   - More context in each notification
   - Consistent thread views

3. **System Health**:
   - Reduced load on external APIs
   - Better rate limit compliance
   - More predictable resource usage

## Risks & Mitigations

1. **Stale Cache**: Mitigated by reasonable TTLs and refresh on important events
2. **Memory Growth**: Bounded cache size with LRU eviction
3. **Race Conditions**: DashMap handles concurrent access safely
4. **Missing Context**: Fallback to direct fetch if cache miss