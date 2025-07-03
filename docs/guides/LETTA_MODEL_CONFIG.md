# Letta Model Configuration

## The letta/letta-free Timeout Issue

The default `letta/letta-free` model is known to have severe timeout issues:
- First requests can take 5-10 minutes or timeout completely
- Even subsequent requests can be very slow
- This causes Discord interactions to fail

## Solution: Use a Different Model

Pattern now supports configuring the model via environment variables:

```bash
# Use OpenAI (requires API key in Letta)
export LETTA_MODEL="openai/gpt-4o-mini"
export LETTA_EMBEDDING_MODEL="openai/text-embedding-3-small"

# Use local models (if configured in Letta)
export LETTA_MODEL="local/llama3"
export LETTA_EMBEDDING_MODEL="local/embeddings"

# Use Groq (fast and free with API key)
export LETTA_MODEL="groq/llama-3.1-70b-versatile"
export LETTA_EMBEDDING_MODEL="letta/letta-free"  # Can keep embeddings as letta-free

# Then run Pattern
cargo run --features full
```

## Setting up Alternative Models in Letta

### Option 1: OpenAI (Recommended for reliability)
1. Get an OpenAI API key from https://platform.openai.com
2. Configure in Letta:
   ```bash
   letta configure
   # Select OpenAI as provider
   # Enter your API key
   ```

### Option 2: Groq (Free and fast)
1. Get a free API key from https://console.groq.com
2. Add to Letta's models via the UI or API
3. Use model names like `groq/llama-3.1-70b-versatile`

### Option 3: Local Models
1. Set up Ollama or similar local inference
2. Configure endpoint in Letta
3. Use model names like `local/llama3`

## Quick Test Script

```bash
# Test with a faster model
LETTA_MODEL="groq/llama-3.1-70b-versatile" \
LETTA_EMBEDDING_MODEL="letta/letta-free" \
cargo run --features full
```

## Making it Permanent

Add to your `.env` file or shell profile:
```bash
export LETTA_MODEL="groq/llama-3.1-70b-versatile"
export LETTA_EMBEDDING_MODEL="letta/letta-free"
```

Or add to the Pattern config (future feature):
```toml
[letta]
base_url = "http://localhost:8283"
model = "groq/llama-3.1-70b-versatile"
embedding_model = "letta/letta-free"
```

## Debugging Model Issues

1. Check available models:
   ```bash
   curl http://localhost:8283/v1/models | jq
   ```

2. Test a model directly:
   ```bash
   curl -X POST http://localhost:8283/v1/agents \
     -H "Content-Type: application/json" \
     -d '{
       "name": "test_model_agent",
       "model": "groq/llama-3.1-70b-versatile",
       "agent_type": "memgpt_agent"
     }'
   ```

3. Monitor Letta logs:
   ```bash
   # Check Letta logs for model loading issues
   tail -f ~/.letta/logs/letta.log
   ```

## Known Working Models

- ✅ `openai/gpt-4o-mini` - Fast and reliable (requires API key)
- ✅ `openai/gpt-3.5-turbo` - Cheaper alternative
- ✅ `groq/llama-3.1-70b-versatile` - Free and fast (requires API key)
- ✅ `groq/mixtral-8x7b-32768` - Good alternative
- ❌ `letta/letta-free` - Often times out
- ❓ Local models - Depends on your setup

## Emergency Fallback

If all else fails, you can manually create agents in the Letta UI with a working model, then use Pattern to interact with them.