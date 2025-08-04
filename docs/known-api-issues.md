# Known API Issues

This document tracks known issues with various model provider APIs and their workarounds.

## Anthropic Claude Issues

### 1. Thinking Mode Message Compression Error

**Error**: 
```
Expected `thinking` or `redacted_thinking`, but found `tool_use`. When `thinking` is enabled, a final `assistant` message must start with a thinking block
```

**Context**: 
When using Anthropic's thinking mode (enabled by default in Pattern), message compression can create invalid message sequences. If the final assistant message after compression contains tool use blocks, it must start with a thinking block.

**Current Status**: Fix implemented as of 2025-08-04

**Fix Applied**:
- Modified message truncation strategy in MessageCompressor
- Assistant messages with thinking blocks are preserved during compression, especially those near the end
- Instead of modifying messages, the fix attempts to keep important messages intact
- This preserves the exact thinking content which may be contextually important

**Remaining Work**:
- Monitor for edge cases where thinking mode messages still cause issues
- Make thinking mode configurable per agent rather than global

## Gemini API Issues

### 1. Missing Response Path Error

**Error**:
```
JsonValueExt(PropertyNotFound("/candidates/0/content/parts"))
```

**Context**:
Gemini API response structure may vary, and the genai crate expects a specific path that doesn't always exist. This happens during heartbeat continuations when the response structure differs from expected.

**Current Status**: Active issue as of 2025-08-03 (in genai crate dependency)

**Root Cause**:
- The error occurs in the genai crate when it tries to extract content from Gemini responses
- Gemini sometimes returns responses without the expected `/candidates/0/content/parts` structure
- This is more likely during error conditions or rate limiting

**Workaround**:
- Retry the request after a short delay
- Consider switching to a different model temporarily
- Monitor Gemini API status for service issues

**Fix Plan**:
- This needs to be fixed in the genai crate itself
- Could fork genai and add more robust response parsing
- Alternative: wrap model calls with retry logic that catches this specific error

## General Database Issues

### 1. Transaction Conflicts

**Error**:
```
Failed to commit transaction due to a read or write conflict. This transaction can be retried
```

**Context**:
SurrealDB transaction conflicts can occur during concurrent message persistence.

**Current Status**: Retry logic implemented but may need tuning

**Workaround**:
- System already retries with backoff
- May need to adjust retry parameters