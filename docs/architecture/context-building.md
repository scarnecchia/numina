# Context Building System

## Overview

The context building system in `pattern-core` provides a complete framework for creating stateful agents on top of stateless LLM APIs. It follows the MemGPT/Letta pattern of agentic context engineering, where agents manage their own memory and context window.

## Architecture

### Core Components

1. **AgentContext** - The complete context ready to send to an LLM
2. **ContextBuilder** - Fluent builder for constructing contexts
3. **AgentState** - Persistent state management between conversations
4. **MessageCompressor** - Strategies for handling context overflow
5. **Memory System** - MemGPT-style memory blocks with character limits

### Key Features

- **Stateful agents on stateless protocols** - Maintains conversation history and memory
- **Automatic context compression** - Multiple strategies for handling overflow
- **Type-safe tool integration** - Tools are properly typed and validated
- **Model-agnostic** - Works with any provider through the genai crate
- **Memory persistence** - Core memory blocks persist between conversations

## Usage Examples

### Basic Context Building

```rust
use pattern_core::prelude::*;
use pattern_core::context::ContextBuilder;

// Create a context for an agent
let context = ContextBuilder::new("agent_123", ContextConfig::default())
    .add_memory_block(MemoryBlock {
        label: "persona".to_string(),
        value: "I am a helpful AI assistant.".to_string(),
        description: Some("Agent personality".to_string()),
        last_modified: Some(Utc::now()),
    })
    .add_memory_block(MemoryBlock {
        label: "human".to_string(),
        value: "The user's name is Alice.".to_string(),
        description: Some("Information about the user".to_string()),
        last_modified: Some(Utc::now()),
    })
    .with_tools_from_registry(&tool_registry)
    .with_messages(conversation_history)
    .build().await?;

// The context is now ready to use with genai
let request = genai::ChatRequest::new(context.system_prompt, context.messages)
    .with_tools(context.tools);
```

### Stateful Agent Management

```rust
use pattern_core::context::{AgentContextBuilder, CompressionStrategy};

// Create a stateful agent context
let agent_context = AgentContextBuilder::new(agent_id, AgentType::Generic)
    .with_memory_block(
        "persona",
        "I am Pattern, an AI assistant specializing in ADHD support.",
        Some("Agent personality and capabilities")
    )
    .with_memory_block(
        "human",
        "User has ADHD and prefers structured communication.",
        Some("User preferences and needs")
    )
    .with_context_config(ContextConfig {
        base_instructions: "You are an ADHD support agent...".to_string(),
        memory_char_limit: 5000,
        max_context_messages: 50,
        enable_thinking: true,
        ..Default::default()
    })
    .with_compression_strategy(CompressionStrategy::RecursiveSummarization {
        chunk_size: 10,
        summarization_model: "gpt-3.5-turbo".to_string(),
    })
    .build().await?;

// Process a user message
agent_context.add_message(Message::user("I need help organizing my tasks")).await;

// Build context for the LLM
let context = agent_context.build_context().await?;

// After getting response from LLM
let responses = agent_context.process_response(chat_response).await?;

// Update memory based on conversation
agent_context.update_memory_block("human", "User struggles with task organization").await?;
```

## Memory Management

### Memory Blocks

Memory blocks follow the MemGPT pattern with permissions and memory types:

```rust
pub struct MemoryBlock {
    /// Unique identifier
    pub id: MemoryId,
    
    /// The user who owns this memory
    pub owner_id: UserId,
    
    /// Label like "persona", "human", "context"
    pub label: CompactString,

    /// The actual memory content
    pub value: String,

    /// Description of what this block contains
    pub description: Option<String>,
    
    /// Type: Core (always in context), Working (swappable), Archival (searchable)
    pub memory_type: MemoryType,
    
    /// Permission level (ReadOnly, Partner, Human, Append, ReadWrite, Admin)
    pub permission: MemoryPermission,
    
    /// Whether pinned (can't be swapped out)
    pub pinned: bool,
    
    /// Embedding for semantic search (if any)
    pub embedding: Option<Vec<f32>>,
}
```

### Core Memory Operations

```rust
// Append to a memory block
agent.append_to_memory_block("human", "Prefers morning meetings")?;

// Replace content in a memory block
agent.replace_in_memory_block(
    "context",
    "working on project X",
    "completed project X, now working on project Y"
)?;

// Update entire block
agent.update_memory_block("persona", "Updated personality description")?;
```

## Message Compression

When conversation history exceeds the context window, various compression strategies are available:

### Truncation (Default)
```rust
CompressionStrategy::Truncate { keep_recent: 50 }
```

### Recursive Summarization (MemGPT Paper)
```rust
CompressionStrategy::RecursiveSummarization {
    chunk_size: 10,
    summarization_model: "gpt-3.5-turbo".to_string(),
}
```

### Importance-Based Selection
```rust
CompressionStrategy::ImportanceBased {
    keep_recent: 20,
    keep_important: 10,
}
```

### Time-Decay Compression
```rust
CompressionStrategy::TimeDecay {
    compress_after_hours: 24.0,
    min_keep_recent: 10,
}
```

## Context Configuration

The `ContextConfig` struct provides fine-grained control:

```rust
pub struct ContextConfig {
    /// Base system instructions
    pub base_instructions: String,

    /// Max characters per memory block
    pub memory_char_limit: usize,

    /// Max messages before compression
    pub max_context_messages: usize,

    /// Enable thinking/reasoning
    pub enable_thinking: bool,

    /// Tool usage rules
    pub tool_rules: Vec<ToolRule>,

    /// Model-specific adjustments
    pub model_adjustments: ModelAdjustments,
}
```

## Working with genai

The context builder produces output compatible with the genai crate:

```rust
// Build context
let context = agent.build_context().await?;

// Create genai request
let request = genai::ChatRequest::new(
    genai::chat::ChatRole::System,
    context.system_prompt
);

// Add messages
for message in context.messages {
    request = request.with_message(message);
}

// Add tools
for tool in context.tools {
    request = request.with_tool(tool);
}

// Execute with your chosen model
let response = client.chat(model, request).await?;
```

## Advanced Features

### Message Search

Search through all messages including archived ones:

```rust
let results = agent.search_messages("task management", page: 0, page_size: 10);
for message in results {
    println!("{}: {}", message.role, message.text_content().unwrap_or_default());
}
```

### State Checkpoints

Create and restore agent state checkpoints:

```rust
// Create checkpoint
let checkpoint = agent.checkpoint();

// ... do some operations ...

// Restore if needed
agent.restore_from_checkpoint(checkpoint)?;
```

### Agent Statistics

Get insights into agent usage:

```rust
let stats = agent.get_stats();
println!("Total messages: {}", stats.total_messages);
println!("Tool calls: {}", stats.total_tool_calls);
println!("Compressions: {}", stats.compression_events);
```

## Best Practices

1. **Set appropriate memory limits** - Keep character limits reasonable for your model's context window
2. **Choose compression strategy wisely** - Recursive summarization preserves more information but costs more tokens
3. **Update memory regularly** - Keep memory blocks current with learned information
4. **Monitor token usage** - Use `context.metadata.total_tokens_estimate` to track usage
5. **Handle compression events** - Important information might be compressed, ensure critical data is in memory blocks

## Integration with Tools

The context builder seamlessly integrates with the type-safe tool system:

```rust
// Register tools (DashMap allows concurrent access)
let registry = ToolRegistry::new();
registry.register(SearchTool);
registry.register(MemoryTool);

// Add to context
let context = ContextBuilder::new(agent_id, config)
    .with_tools_from_registry(&registry)
    .build()?;

// Tools are automatically formatted for the LLM
// with proper schemas and usage rules
```

## Error Handling

The system uses rich error types with miette for detailed diagnostics:

```rust
match agent.update_memory_block("nonexistent", "value") {
    Err(CoreError::MemoryNotFound { block_name, available_blocks, .. }) => {
        println!("Block '{}' not found. Available: {:?}", block_name, available_blocks);
    }
    Err(e) => return Err(e),
    Ok(_) => {}
}
```
