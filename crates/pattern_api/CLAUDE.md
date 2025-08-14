# CLAUDE.md - Pattern API

Shared API types and contracts for Pattern's client-server communication.

## Purpose

This crate defines the API contract between Pattern's backend server and frontend clients, ensuring type safety and consistency across the system.

## Current Status

### ✅ Implemented Types

#### Request/Response Structures
- Authentication requests (password, API key)
- Token refresh requests
- Health check endpoint
- Error types with proper HTTP status codes

#### Core Models
- User responses
- Agent responses  
- Message responses
- Group responses
- Memory block responses

#### API Traits
- `ApiEndpoint` trait for path definitions
- Consistent JSON serialization
- Type-safe error handling

## Architecture

### Endpoint Definition Pattern
```rust
pub trait ApiEndpoint {
    const PATH: &'static str;
    const METHOD: Method;
}

impl ApiEndpoint for HealthCheckRequest {
    const PATH: &'static str = "/api/health";
    const METHOD: Method = Method::GET;
}
```

### Error Handling
```rust
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "error", content = "details")]
pub enum ApiError {
    Validation { field: String, message: String },
    Unauthorized { message: Option<String> },
    NotFound { resource: String, id: String },
    RateLimited { retry_after_seconds: u64 },
    // ...
}
```

### Request/Response Pairs
Each operation has matched request and response types:
- `AuthRequest` → `AuthResponse`
- `CreateAgentRequest` → `AgentResponse`
- `SendMessageRequest` → `MessageResponse`

## Usage Guidelines

### Adding New Endpoints
1. Define request struct with validation
2. Define response struct
3. Implement `ApiEndpoint` trait
4. Add error variants if needed
5. Update server handlers
6. Update client code

### Versioning Strategy
- Breaking changes bump minor version
- New endpoints are additive
- Deprecated fields marked but not removed
- Version in URL path when v2 needed

## Integration Points

### Server Side (pattern_server)
```rust
use pattern_api::{requests::*, responses::*, ApiError};

async fn handle_request(
    Json(req): Json<CreateAgentRequest>
) -> Result<Json<AgentResponse>, ApiError> {
    // Implementation
}
```

### Client Side
```rust
use pattern_api::{requests::*, responses::*};

let response: AgentResponse = client
    .post(CreateAgentRequest::PATH)
    .json(&request)
    .send()
    .await?
    .json()
    .await?;
```

## Type Safety Benefits

- Compile-time validation of API contracts
- Automatic serialization/deserialization
- Consistent error handling
- Self-documenting code
- Easy refactoring

## Future Additions

### Planned Endpoints
- WebSocket events for real-time updates
- Bulk operations for efficiency
- Pagination for large result sets
- Filtering and sorting parameters
- File upload/download support

### Schema Evolution
- Optional fields for backward compatibility
- Explicit versioning when needed
- Migration guides for breaking changes
- OpenAPI spec generation