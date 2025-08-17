# CLAUDE.md - Pattern CLI

⚠️ **CRITICAL WARNING**: DO NOT run ANY CLI commands during development!
Production agents are running. Any CLI invocation will disrupt active agents.
Testing must be done offline after stopping production agents.

Command-line interface for testing and managing the Pattern ADHD support system.

## Purpose

This crate provides the primary development and testing interface for Pattern, allowing direct interaction with agents, groups, data sources, and all system components.

## Current Status

### ✅ Implemented Commands

#### Agent Operations
- `agent create` - Create new agents with custom prompts
- `agent list` - List all agents in the system
- `agent show <name>` - Display agent details
- `agent delete <name>` - Remove an agent

#### Group Operations  
- `group create` - Create agent groups with coordination patterns
- `group add-member` - Add agents to groups
- `group list` - List all groups
- `group status <name>` - Show group details and members

#### Chat Modes
- `chat` - Interactive chat with single agent
- `chat --group <name>` - Chat with agent group
- `chat --discord` - Test Discord integration locally
- `chat --export` - Export conversation history

#### Data Operations
- `dq` - Query database directly (DuckQuery)
- `export` - Export agent data to CAR format
- `import` - Import agent data from CAR files

#### ATProto/Bluesky
- `atproto fetch-thread` - Fetch Bluesky thread context
- `atproto list-identities` - List stored ATProto identities
- `firehose` - Connect to Bluesky firehose

#### Debug Commands
- `debug memory` - View agent memory blocks
- `debug context` - Show context state
- `debug messages` - List conversation messages

## Architecture Patterns

### Command Structure
```rust
#[derive(Subcommand)]
enum Commands {
    Agent(AgentCommand),
    Group(GroupCommand),
    Chat(ChatArgs),
    Export(ExportArgs),
    // ...
}
```

### Chat Mode Features
- Interactive terminal UI with message display
- Typing indicators for agent processing
- Memory block visibility
- Tool call display
- Export functionality

### Output System
- Colored terminal output with status indicators
- Progress bars for long operations
- Structured message formatting
- Error display with context

## Testing Guidelines

### Quick Test Commands
```bash
# Test agent creation and chat
pattern-cli agent create test-agent
pattern-cli chat --agent test-agent

# Test group coordination
pattern-cli group create test-group --pattern round-robin
pattern-cli group add-member test-group Pattern
pattern-cli chat --group test-group

# Test Discord locally
pattern-cli chat --discord

# Query database
pattern-cli dq "SELECT * FROM agent"
```

### Integration Testing
- Use CLI for end-to-end testing of features
- Verify agent responses and tool usage
- Test coordination patterns with groups
- Validate data source processing

## Common Workflows

### Agent Development
1. Create agent with custom prompt
2. Test in chat mode
3. Adjust memory blocks as needed
4. Export for backup

### Group Testing
1. Create group with desired pattern
2. Add member agents
3. Test coordination in chat
4. Monitor pattern behavior

### Data Source Testing
1. Configure data source
2. Run with `--watch` or `--notify`
3. Verify agent notifications
4. Check processing results

## Implementation Notes

- Uses `clap` for command parsing
- `tokio` runtime for async operations
- `ratatui` for terminal UI in chat mode
- Direct database access via pattern_core
- Shares endpoints with other crates

## Known Issues

- CAR export may not capture full message history
- Some terminal emulators have issues with colors
- Large exports can be memory intensive