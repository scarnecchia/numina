#!/usr/bin/env bash
# Cleanup script to reset Pattern state

echo "Pattern Cleanup Script"
echo "====================="

# 1. Remove database files
echo "Removing database files..."
rm -f pattern.db test_pattern.db
echo "✓ Database files removed"

# 2. Clean up any log files
echo "Cleaning up logs..."
rm -rf logs/*.log*
echo "✓ Log files removed"

# 3. Remove any cache directories
echo "Removing cache directories..."
rm -rf .pattern_cache cache
echo "✓ Cache directories removed"

# 4. Instructions for Letta cleanup
echo ""
echo "Letta Cleanup:"
echo "=============="
echo "To clean up Letta agents, run:"
echo "  ./scripts/cleanup_agents.sh --all"
echo ""
echo "To clean up MCP servers:"
echo "  ./scripts/cleanup_mcp.sh"
echo ""
echo "For manual cleanup:"
echo "  1. Open Letta UI at http://localhost:8283"
echo "  2. Go to Agents tab and delete all Pattern agents"
echo "  3. Go to Tools > MCP Servers and delete 'pattern_mcp' server"
echo ""

# 5. Create fresh config if needed
if [ ! -f "pattern.toml" ] && [ -f "pattern.toml.example" ]; then
    echo "Creating fresh pattern.toml from example..."
    cp pattern.toml.example pattern.toml
    echo "✓ Config file created"
fi

echo ""
echo "Cleanup complete! You can now start fresh."
echo ""
echo "Next steps:"
echo "1. Make sure Letta server is running: letta server"
echo "2. Start Pattern: cargo run --features full"
