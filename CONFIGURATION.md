# Pattern Configuration Guide

## Quick Start

### Option 1: Configuration File
```bash
cp pattern.toml.example pattern.toml
# Edit pattern.toml and add your Discord token
cargo run --features full
```

### Option 2: .env File
```bash
cp .env.example .env
# Edit .env and add your Discord token
cargo run --features full
```

## Configuration Hierarchy

Pattern looks for configuration in this order (highest priority first):
1. Environment variables
2. pattern.toml file (or file specified by PATTERN_CONFIG)
3. Built-in defaults

## Common Configurations

### Local Development (Default)
- Uses local Letta server at http://localhost:8000
- SQLite database at pattern.db
- MCP server disabled
- Built-in agent personalities

### Letta Cloud
Set `LETTA_API_KEY` environment variable or add to pattern.toml:
```toml
[letta]
api_key = "your_api_key"
```

### Custom Agent Personalities (Optional)
The default agent personalities are compiled into the binary, so no configuration is needed.

To customize agents:
1. Create your own agents.toml file (use src/default_agents.toml as a template)
2. Set `AGENT_CONFIG_PATH=/path/to/agents.toml` environment variable, or
3. Add `agent_config_path = "/path/to/agents.toml"` to pattern.toml

### Enable MCP Server
```toml
[mcp]
enabled = true
transport = "sse"  # Recommended for Letta
port = 3000
```

## All Configuration Options

See pattern.toml.example and .env.example for all available options.

Most users only need to set their Discord token!
