# Testing Agent-to-Agent Messaging

This guide helps you test the newly implemented agent-to-agent messaging system.

## Prerequisites

1. Build the CLI:
```bash
cargo build --bin pattern-cli
```

2. Ensure the database is running (happens automatically with embedded SurrealDB)

## Testing Steps

### 1. Create Two Agents

In terminal 1:
```bash
pattern-cli agent create Alice
```

In terminal 2:
```bash
pattern-cli agent create Bob
```

### 2. Start Chat Sessions

In terminal 1:
```bash
pattern-cli chat --agent Alice
```

In terminal 2:
```bash
pattern-cli chat --agent Bob
```

### 3. Test Slash Commands

In either terminal, try these commands:

- `/help` - Shows available commands
- `/list` - Lists all agents
- `/status` - Shows current agent status

### 4. Send Messages Between Agents

In Alice's chat (terminal 1):
```
/send Bob Hello Bob, this is Alice!
```

Watch Bob's terminal - you should see:
- A heartbeat notification
- Bob processing the incoming message
- Bob's response to Alice's message

### 5. Test Bidirectional Communication

In Bob's chat (terminal 2):
```
/send Alice Hi Alice, I got your message!
```

### 6. Monitor Live Queries

The system uses SurrealDB live queries to monitor incoming messages. Each agent:
- Subscribes to their message queue on startup
- Processes messages immediately when they arrive
- Marks messages as read to prevent reprocessing

## Troubleshooting

### Messages Not Appearing
1. Check if tools are enabled: `pattern-cli chat --agent Alice` (tools enabled by default)
2. Look for "📬 Agent ... message monitoring started" in the logs
3. Verify both agents are running

### Debug Mode
Run with debug logging to see more details:
```bash
pattern-cli --debug chat --agent Alice
```

## Architecture Notes

- Messages are stored in the `queue_msg` table
- Each agent has a live query watching for `to_agent = self AND read = false`
- The `AgentMessageRouter` handles routing to the appropriate endpoint
- Currently only `CliEndpoint` is implemented (prints to stdout)

## Next Steps

Once basic messaging works:
1. Test with groups: `pattern-cli chat --group MyGroup`
2. Test heartbeat functionality (agents continuing conversations)
3. Test message loops prevention (agent A → B → A → ...)