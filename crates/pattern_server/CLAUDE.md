# CLAUDE.md - Pattern Server

Backend API server for Pattern, providing HTTP/WebSocket APIs for multi-user hosting.

## Current Status

### 🚧 MOSTLY STUB - IN DEVELOPMENT

#### ✅ Implemented
- Basic Axum server setup with state management
- JWT-based authentication (access + refresh tokens)
- Password hashing with bcrypt
- Health check endpoint
- Auth middleware for protected routes
- CORS configuration
- Database connection pool

#### 🔴 Not Implemented (Stubs/TODOs)
- Most API endpoints beyond auth
- WebSocket support for real-time updates
- Agent management endpoints
- Message handling endpoints
- Group coordination endpoints
- MCP integration
- Rate limiting
- Metrics and monitoring

## Architecture

### Server Structure
```rust
pub struct AppState {
    pub db: SurrealDB,
    pub config: ServerConfig,
    pub jwt_encoding_key: EncodingKey,
    pub jwt_decoding_key: DecodingKey,
    pub agent_registry: Arc<DashMap<String, Arc<dyn Agent>>>,
}
```

### Authentication Flow
1. User logs in with username/password
2. Server validates credentials against database
3. Returns JWT access token (15min) + refresh token (7 days)
4. Client includes access token in Authorization header
5. Refresh token used to get new access token when expired

### Middleware Stack
- CORS handling
- Request ID generation
- Authentication verification
- Rate limiting (planned)
- Error handling

## Implementation Roadmap

### Phase 1: Core API (Current)
- [x] Authentication endpoints
- [x] Health check
- [ ] User management
- [ ] Agent CRUD operations
- [ ] Basic message handling

### Phase 2: Real-time Features
- [ ] WebSocket support
- [ ] Live message streaming
- [ ] Agent status updates
- [ ] Typing indicators
- [ ] Presence system

### Phase 3: Advanced Features
- [ ] Group management
- [ ] Data source configuration
- [ ] MCP tool management
- [ ] Export/import functionality
- [ ] Admin dashboard API

### Phase 4: Production Ready
- [ ] Rate limiting
- [ ] Metrics (Prometheus)
- [ ] Audit logging
- [ ] Backup/restore
- [ ] Multi-tenancy

## Development Guidelines

### Adding New Endpoints
1. Define request/response types in pattern_api
2. Create handler function in appropriate module
3. Add route to router in handlers/mod.rs
4. Implement business logic
5. Add tests

### Handler Pattern
```rust
pub async fn create_agent(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Json(request): Json<CreateAgentRequest>,
) -> Result<Json<AgentResponse>, ApiError> {
    // Validate request
    // Perform database operations
    // Return response
}
```

### Error Handling
- Use pattern_api::ApiError for all errors
- Include helpful error messages
- Log errors with appropriate level
- Return proper HTTP status codes

## Testing

### Unit Tests
```bash
cargo test --package pattern-server --lib
```

### Integration Tests
```bash
# Start test database
surreal start --log debug memory

# Run integration tests
cargo test --package pattern-server --test '*'
```

### Manual Testing
```bash
# Start server
cargo run --bin pattern-server

# Test health endpoint
curl http://localhost:3000/api/health

# Test auth
curl -X POST http://localhost:3000/api/auth \
  -H "Content-Type: application/json" \
  -d '{"username":"test","password":"test123"}'
```

## Configuration

### Environment Variables
```bash
# Required
DATABASE_URL=surreal://localhost:8000
JWT_SECRET=your-secret-key

# Optional
PORT=3000
ACCESS_TOKEN_TTL=900
REFRESH_TOKEN_TTL=604800
BCRYPT_COST=12
```

### Config File
```toml
[server]
port = 3000
host = "0.0.0.0"

[database]
url = "surreal://localhost:8000"
namespace = "pattern"
database = "main"

[auth]
access_token_ttl = 900
refresh_token_ttl = 604800
bcrypt_cost = 12
```

## Security Considerations

- JWT secrets must be strong and rotated
- Passwords hashed with bcrypt (cost 12)
- CORS configured for specific origins
- Rate limiting on auth endpoints (TODO)
- SQL injection prevented via parameterized queries
- XSS prevention via proper escaping

## Known Issues

- Token refresh doesn't check if family is revoked
- No rate limiting implemented yet
- WebSocket support not implemented
- Some error messages may leak information