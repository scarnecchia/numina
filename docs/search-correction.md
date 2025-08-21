# Search Implementation Correction

## Misunderstanding About @N@ Operator

I incorrectly implemented the `@N@` operator as fuzzy search with edit distance. This is **WRONG**.

### What @N@ Actually Does

The number in `@N@` is a reference identifier for search functions:

```sql
-- Example with multiple search conditions
SELECT *, 
    search::score(1) AS title_score,
    search::score(2) AS body_score
FROM articles
WHERE title @1@ "machine learning"  -- Search #1
  AND body @2@ "neural networks"     -- Search #2
```

- `@1@` marks the first search condition
- `@2@` marks the second search condition  
- `search::score(1)` calculates BM25 score for condition #1
- `search::score(2)` calculates BM25 score for condition #2

### Current Implementation Issues

1. **Fuzzy search misimplementation**: Using `@1@`, `@2@` for fuzzy matching is incorrect
2. **Single search term**: We only have one search query, so we should use `@1@` consistently
3. **No actual fuzzy search**: SurrealDB fuzzy search requires different syntax

## Correct Implementation

### For Single Search Term (Our Case)

Since we only search one field at a time, we should use:

```sql
-- Correct: Use @1@ for the single search term
SELECT *, search::score(1) AS relevance_score
FROM msg  
WHERE content @1@ $search_query
ORDER BY relevance_score DESC
```

Or just use `@@` without a number since we only have one condition:

```sql
-- Also correct: Plain @@ when only one search term
SELECT *, search::score() AS relevance_score  
FROM msg
WHERE content @@ $search_query
ORDER BY relevance_score DESC
```

### For Actual Fuzzy Search

SurrealDB fuzzy search requires different approaches:

1. **Using MATCHES** (if available):
```sql
WHERE content MATCHES $search_query
```

2. **Using similarity functions** (if available):
```sql
WHERE string::similarity::fuzzy(content, $search_query) > 0.8
```

3. **Using regex for simple typos**:
```sql
WHERE content ~ $regex_pattern
```

## What Needs to Be Fixed

1. Remove the "fuzzy_level" parameter interpretation
2. Change all search queries to use `@1@` or just `@@`
3. Document that fuzzy search is not yet implemented
4. Research actual SurrealDB fuzzy search capabilities

## Impact

- BM25 scoring still works correctly
- Search will still be improved with relevance ranking
- But "fuzzy" parameter doesn't actually enable fuzzy matching
- Need different approach for typo tolerance