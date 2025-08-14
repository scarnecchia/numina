# CLAUDE.md - Pattern MCP

Model Context Protocol implementation for Pattern - currently a skeleton awaiting development.

## Current Status

**STATUS: Stub implementation only**
- Basic crate structure exists
- Smoke tests in place
- Core functionality not implemented
- Lower priority than MCP client integration

## Planned Architecture

### MCP Server
- Expose Pattern tools to external MCP clients
- Support stdio, HTTP, and SSE transports
- Tool registration and discovery

### MCP Client (Higher Priority)
- Consume external MCP tools
- Integrate with agent tool registry
- Dynamic tool discovery

## MCP Schema Requirements

### CRITICAL: Schema Restrictions
- **NO `$ref` references** - All types must be inlined
- **NO nested enums** - Flatten to string with validation
- **NO unsigned integers** - Use signed integers or numbers
- **NO complex generics** - Keep schemas simple

### Example Schema Pattern
```rust
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolParams {
    #[serde(default)]
    #[schemars(description = "Optional parameter")]
    pub optional_field: Option<String>,
    
    #[schemars(description = "Required parameter")]
    pub required_field: String,
    
    // Use i32/i64 instead of u32/u64
    #[schemars(description = "Count parameter")]
    pub count: i32,
}
```

## Implementation Guidelines

When this crate is developed:

1. **Tool Modularity**
   - Each tool in its own module
   - Self-contained with tests
   - Clear parameter/result types

2. **Transport Support**
   - stdio for CLI integration
   - HTTP for web services
   - SSE for streaming updates

3. **Error Handling**
   - User-friendly error messages
   - Proper error propagation
   - Graceful degradation

## Future Tools

Planned Pattern-specific tools:
- Memory management (create, update, search blocks)
- Agent communication (send messages, query status)
- Task management (create, breakdown, track)
- Discord context (channel info, user roles)
- ADHD support (energy tracking, focus detection)

## References

- [MCP Specification](https://modelcontextprotocol.io/specification)
- [MCP Rust SDK](https://github.com/modelcontextprotocol/rust-sdk)