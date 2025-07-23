# CLAUDE.md - Pattern MCP

This crate provides the Model Context Protocol (MCP) server implementation for Pattern, enabling external tools and integrations.

## Core Principles

- **MCP Specification Compliance**: Follow the official MCP spec strictly
- **Schema Simplicity**: No $ref, no nested enums, no unsigned integers in tool schemas
- **Transport Flexibility**: Support stdio, HTTP, and SSE transports
- **Tool Modularity**: Each tool should be self-contained and testable

## Architecture Overview

### Key Components

1. **Server Implementation** (`lib.rs`)
   - Uses the official `mcp` crate (formerly `rmcp`)
   - Handles multiple transport types
   - Tool registration and routing

2. **Tool Implementations** (`tools/`)
   - Each tool in its own module
   - Consistent parameter/result patterns
   - Clear error messages for users

3. **Transport Layer** (`transport.rs`)
   - stdio for CLI integration
   - HTTP for web services
   - SSE for streaming updates

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
    pub count: i32,
    
    // Flatten enums to strings
    #[serde(default = "default_mode")]
    pub mode: String, // Validate in execute()
}
```

## Tool Development Guidelines

### Tool Structure
```rust
pub struct MyTool {
    // Shared state (e.g., database connection)
}

impl MyTool {
    pub async fn execute(&self, params: Value) -> Result<Value> {
        // 1. Deserialize and validate
        let params: ToolParams = serde_json::from_value(params)?;
        
        // 2. Validate string enums
        match params.mode.as_str() {
            "fast" | "slow" => {},
            _ => return Err(anyhow!("Invalid mode")),
        }
        
        // 3. Execute logic
        
        // 4. Return simple JSON response
    }
}
```

### Error Handling
- Use descriptive error messages
- Include suggestions for fixes
- Return structured errors when possible
- Log errors for debugging but don't expose internals

## Transport Configuration

### stdio (Default)
```toml
[mcp]
transport = "stdio"
```

### HTTP
```toml
[mcp]
transport = "http"
port = 3000
```

### SSE
```toml
[mcp]
transport = "sse"
port = 3001
```

## Testing Tools

```bash
# Test individual tool
cargo test -p pattern-mcp tool_name

# Test with MCP client
mcp-client stdio -- cargo run -p pattern-mcp

# Test HTTP endpoint
curl -X POST http://localhost:3000/tool/execute \
  -H "Content-Type: application/json" \
  -d '{"tool": "tool_name", "params": {}}'
```

## Common Issues

1. **Schema Validation Failures**
   - Check for nested enums
   - Ensure no unsigned integers
   - Verify no $ref generation

2. **Transport Errors**
   - stdio: Check for debug prints polluting output
   - HTTP: Verify CORS headers if needed
   - SSE: Ensure proper event formatting

3. **Tool Registration**
   - Tools must be registered in server initialization
   - Check tool names don't conflict

## Performance Considerations

- Keep tool execution under 30s (MCP timeout)
- Use streaming for long operations
- Cache expensive computations
- Minimize JSON payload sizes

## Security

- Validate all inputs
- Sanitize file paths
- Rate limit if exposed to internet
- Don't expose internal errors to clients