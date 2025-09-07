# CLAUDE.md - Pattern MCP

Model Context Protocol implementation for Pattern - client fully functional, server stub only.

## Current Status

### ✅ MCP Client - IMPLEMENTED
- All three transports working (stdio, HTTP, SSE)
- Tool discovery via rmcp SDK
- Dynamic tool wrapper system
- Integration with Pattern's tool registry
- Mock tools for testing when no server available
- Basic auth support (Bearer tokens, custom headers)
- Needs testing with real MCP servers

### 🚧 MCP Server - STUB ONLY
- Basic crate structure exists
- Smoke tests in place
- Core server functionality not implemented
- Lower priority

## MCP Client Architecture

### Transport Support
- **stdio**: Child process communication via rmcp's TokioChildProcess
- **HTTP**: Streamable HTTP via rmcp's StreamableHttpClientTransport  
- **SSE**: Server-Sent Events via rmcp's SseClientTransport
- **Auth**: Bearer tokens and Authorization headers supported

### Tool Integration Flow
1. Connect to MCP server via configured transport
2. Discover available tools using rmcp peer
3. Wrap each tool in McpToolWrapper (implements DynamicTool)
4. Register wrapped tools with Pattern's tool registry
5. Handle tool invocations via channel-based request/response

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