#!/usr/bin/env bash
# Script to manually register Pattern MCP server with Letta

echo "Manual MCP Server Registration for Pattern"
echo "=========================================="
echo ""

# Default values
LETTA_URL="${LETTA_URL:-http://localhost:8283}"
MCP_PORT="${MCP_PORT:-8080}"

echo "Using Letta URL: $LETTA_URL"
echo "Using MCP Port: $MCP_PORT"
echo ""

# Check if Pattern MCP server is running
echo "Checking if Pattern MCP server is running..."
if curl -s -f http://localhost:$MCP_PORT/mcp >/dev/null 2>&1; then
    echo "✓ Pattern MCP server is running on port $MCP_PORT"
else
    echo "✗ Pattern MCP server is not responding on port $MCP_PORT"
    echo "  Make sure Pattern is running with MCP_TRANSPORT=http"
    exit 1
fi

# Check if Letta is running
echo ""
echo "Checking if Letta is running..."
if curl -s -f $LETTA_URL/v1/health >/dev/null 2>&1; then
    echo "✓ Letta is running at $LETTA_URL"
else
    echo "✗ Letta is not responding at $LETTA_URL"
    echo "  Make sure Letta server is running"
    exit 1
fi

# Check if already registered
echo ""
echo "Checking if MCP server is already registered..."
if curl -s $LETTA_URL/v1/tools/mcp/servers/pattern-discord/tools 2>&1 | grep -q "name"; then
    echo "✓ MCP server 'pattern-discord' is already registered"
    echo "  Listing available tools:"
    curl -s $LETTA_URL/v1/tools/mcp/servers/pattern-discord/tools | jq -r '.[] | "  - \(.name)"' 2>/dev/null || echo "  (Could not parse tools list)"
    exit 0
fi

# Register the MCP server
echo ""
echo "Registering Pattern MCP server with Letta..."
echo "This may take a while if Letta is slow or has a backlog..."

RESPONSE=$(curl -s -X PUT $LETTA_URL/v1/tools/mcp/servers \
  -H "Content-Type: application/json" \
  -d '{
    "server_name": "pattern-discord",
    "server_type": "streamable_http",
    "server_url": "http://localhost:'$MCP_PORT'/mcp"
  }' 2>&1)

if echo "$RESPONSE" | grep -q "already exists"; then
    echo "✓ MCP server already exists (this is fine)"
elif echo "$RESPONSE" | grep -q "error"; then
    echo "✗ Error registering MCP server:"
    echo "$RESPONSE" | jq . 2>/dev/null || echo "$RESPONSE"
    exit 1
else
    echo "✓ Successfully registered MCP server"
fi

# Try to list tools
echo ""
echo "Attempting to list available tools..."
echo "Note: This may fail with 'Internal Server Error' if Letta is still processing"
echo "      You can try running this script again in a few minutes"
echo ""

TOOLS=$(curl -s $LETTA_URL/v1/tools/mcp/servers/pattern-discord/tools 2>&1)
if echo "$TOOLS" | grep -q "Internal Server Error"; then
    echo "⚠ Letta returned Internal Server Error"
    echo "  This usually means Letta is still initializing the connection"
    echo "  The MCP server is registered but may need time to become available"
    echo "  Try again in a few minutes"
else
    echo "Available tools:"
    echo "$TOOLS" | jq -r '.[] | "  - \(.name): \(.description)"' 2>/dev/null || echo "  (Could not parse tools list)"
fi

echo ""
echo "Done. You can now use Pattern's MCP tools in Letta agents!"