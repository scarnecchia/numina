#!/usr/bin/env bash
# Cleanup script to remove MCP server from Letta before restarting Pattern

echo "Cleaning up MCP server registration in Letta..."

# Check if Letta is running
if ! curl -s http://localhost:8283/v1/health >/dev/null 2>&1; then
    echo "Letta server is not running at localhost:8283"
    echo "Please start Letta first."
    exit 1
fi

# Try to delete the MCP server
echo "Attempting to remove 'pattern_mcp' MCP server..."
curl -X DELETE http://localhost:8283/v1/tools/mcp/servers/pattern_mcp 2>&1 | grep -v "^%" || true

# Also try old name in case it exists
echo "Attempting to remove 'pattern-discord' MCP server (old name)..."
curl -X DELETE http://localhost:8283/v1/tools/mcp/servers/pattern-discord 2>&1 | grep -v "^%" || true

echo "Done. You can now start Pattern with MCP transport."
