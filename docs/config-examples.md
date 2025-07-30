# Pattern Configuration Examples

This directory contains example configuration files for Pattern:

## Main Configuration

- `pattern.example.toml` - Complete example showing all configuration options including:
  - User settings
  - Single agent configuration with memory blocks
  - Model provider settings
  - Database configuration (embedded and remote)
  - Agent groups with various member configuration methods
  - Bluesky integration settings

## Agent Configurations

- `agents/task_manager.toml.example` - Example external agent configuration file showing:
  - Agent persona and instructions
  - Multiple memory blocks (inline and from files)
  - Different memory types (Core, Working, Archival)

## Getting Started

1. Copy `pattern.example.toml` to `pattern.toml` (or `~/.config/pattern/config.toml`)
2. Edit the configuration to match your setup
3. Set required environment variables for your chosen model provider:
   - `OPENAI_API_KEY` for OpenAI
   - `ANTHROPIC_API_KEY` for Anthropic
   - `GEMINI_API_KEY` for Google Gemini
   - `GROQ_API_KEY` for Groq
   - etc.

## Group Member Configuration

Groups support three ways to configure members:

1. **Reference existing agent**: Use `agent_id` to reference an agent already in the database
2. **External config file**: Use `config_path` to load agent configuration from a separate file
3. **Inline configuration**: Define the agent configuration directly in the group member section

See `pattern.example.toml` for examples of all three methods.