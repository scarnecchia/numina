# CLAUDE.md - Pattern Main

This crate is the main orchestrator that ties together all Pattern components into a unified application.

## Core Principles

- **Integration Over Implementation**: This crate coordinates, it doesn't implement core logic
- **Configuration-Driven**: Behavior should be controllable via config files
- **Graceful Startup/Shutdown**: Handle component initialization order and cleanup
- **Feature Flag Coordination**: Manage which components are active
- **Observability**: Comprehensive logging and metrics

## Architecture Overview

### Key Components

1. **Application Core** (`app.rs`)
   - Component initialization and lifecycle
   - Configuration loading and validation
   - Graceful shutdown handling
   - Health check coordination

2. **Service Integration** (`services/`)
   - Discord bot initialization
   - MCP server startup
   - Background task scheduling
   - Database connection management

3. **Configuration** (`config.rs`)
   - TOML-based configuration
   - Environment variable overrides
   - Hot-reload support for some settings
   - Model mapping configuration

4. **Background Tasks** (`background/`)
   - Sleeptime monitoring
   - Database maintenance
   - Cache cleanup
   - Metric collection

## Startup Sequence

```rust
// 1. Load configuration
let config = Config::load_from_file_or_default()?;

// 2. Initialize database
let db = SurrealDB::connect(&config.database.url).await?;
run_migrations(&db).await?;

// 3. Create shared services
let tool_registry = create_tool_registry(&config);
let model_registry = create_model_registry(&config);

// 4. Initialize agents
let agent_manager = AgentManager::new(db.clone(), config.agents);
agent_manager.initialize_all().await?;

// 5. Start background tasks
let background_handle = spawn_background_tasks(&config, db.clone());

// 6. Start public services (Discord, MCP)
let discord = if config.discord.enabled {
    Some(DiscordBot::start(config.discord, agent_manager.clone()).await?)
} else {
    None
};

let mcp = if config.mcp.enabled {
    Some(McpServer::start(config.mcp, tool_registry).await?)
} else {
    None
};

// 7. Wait for shutdown signal
shutdown_signal().await;

// 8. Graceful cleanup (reverse order)
```

## Configuration Structure

```toml
# pattern.toml
[database]
url = "surreal://localhost:8000"
namespace = "pattern"
database = "main"

[models]
default = "anthropic/claude-3-sonnet"
summarization = "openai/gpt-3.5-turbo"

[models.capabilities]
routine = ["openai/gpt-3.5-turbo", "anthropic/claude-3-haiku"]
interactive = ["anthropic/claude-3-sonnet", "openai/gpt-4"]
investigative = ["anthropic/claude-3-opus", "openai/gpt-4-turbo"]
critical = ["anthropic/claude-3-opus"]

[agents]
config_path = "./config/agents.toml"
default_prompt_size = 2000

[discord]
enabled = true
token = "${DISCORD_TOKEN}"
command_prefix = "!"
slash_commands = true

[mcp]
enabled = true
transport = "sse"
port = 3000
allowed_origins = ["http://localhost:*"]

[background]
sleeptime_check_interval = "20m"
cache_cleanup_interval = "1h"
metrics_collection_interval = "5m"

[logging]
level = "info"
format = "pretty" # or "json"
targets = ["stdout", "file"]
```

## Feature Flags

```toml
[features]
default = ["discord", "mcp", "background"]
discord = ["pattern-discord", "serenity"]
mcp = ["pattern-mcp", "mcp-server"]
background = ["tokio-cron"]
full = ["discord", "mcp", "background", "metrics"]
```

## Environment Variables

Priority: Environment > Config File > Defaults

```bash
# Required
PATTERN_DATABASE_URL=surreal://localhost:8000
DISCORD_TOKEN=your_token

# Optional overrides
PATTERN_LOG_LEVEL=debug
PATTERN_MCP_PORT=3001
PATTERN_CONFIG_PATH=/custom/pattern.toml
```

## Health Checks

```rust
// Exposed at /health (HTTP) or via status command (Discord)
pub struct HealthStatus {
    pub database: ComponentHealth,
    pub agents: HashMap<AgentId, ComponentHealth>,
    pub discord: Option<ComponentHealth>,
    pub mcp: Option<ComponentHealth>,
    pub background_tasks: Vec<TaskHealth>,
}

pub enum ComponentHealth {
    Healthy,
    Degraded { reason: String },
    Unhealthy { error: String },
}
```

## Error Handling Philosophy

1. **Startup Failures**: Fail fast with clear error messages
2. **Runtime Failures**: Isolate and recover where possible
3. **Component Failures**: Degrade gracefully, alert operators
4. **User-Facing Errors**: Always provide helpful context

## Monitoring & Metrics

- Request/response times per component
- Agent invocation counts and latency
- Memory usage by component
- Database query performance
- Discord message processing rate
- MCP tool execution metrics

## Common Operational Tasks

### Adding a New Agent
1. Update agent configuration file
2. Send SIGHUP or use reload command
3. Verify in logs/status endpoint

### Rotating Discord Token
1. Update environment variable
2. Restart Discord component only
3. Other services remain running

### Database Maintenance
1. Background task handles routine cleanup
2. Manual maintenance via CLI commands
3. Backup before major operations

## Performance Tuning

- Connection pooling for database
- Message queuing for high-volume channels
- Caching for frequently accessed data
- Lazy loading of agent configurations
- Resource limits per component

## Security Considerations

- Validate all configuration on load
- Sanitize environment variables in logs
- Rate limiting on public endpoints
- Audit logging for sensitive operations
- Principle of least privilege for components

## Debugging Tips

1. Enable debug logging: `PATTERN_LOG_LEVEL=debug`
2. Component-specific logs: `PATTERN_LOG_FILTER=pattern_discord=trace`
3. Health endpoint shows component states
4. Metrics endpoint for performance data
5. Debug commands in Discord for live inspection