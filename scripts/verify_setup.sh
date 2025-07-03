#!/usr/bin/env bash
# Verify Pattern is set up correctly

echo "Pattern Setup Verification"
echo "=========================="

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m' # No Color

# Check if Letta is running
echo -n "1. Checking Letta server... "
if curl -s -f "http://localhost:8283/health" > /dev/null 2>&1; then
    echo -e "${GREEN}✓ Running${NC}"
else
    echo -e "${RED}✗ Not running${NC}"
    echo "   Start with: letta server"
    exit 1
fi

# Check if MCP server is registered
echo -n "2. Checking MCP server registration... "
MCP_SERVERS=$(curl -s "http://localhost:8283/v1/tools/mcp/servers" 2>/dev/null)
if echo "$MCP_SERVERS" | grep -q "pattern_mcp"; then
    echo -e "${GREEN}✓ Registered${NC}"
else
    echo -e "${RED}✗ Not found${NC}"
    echo "   Pattern needs to register the MCP server on startup"
fi

# Check if tools are available
echo -n "3. Checking MCP tools... "
TOOLS=$(curl -s "http://localhost:8283/v1/tools/mcp/servers/pattern_mcp/tools" 2>/dev/null)
TOOL_COUNT=$(echo "$TOOLS" | jq 'length' 2>/dev/null || echo "0")
if [ "$TOOL_COUNT" -gt 0 ]; then
    echo -e "${GREEN}✓ $TOOL_COUNT tools available${NC}"
    echo "   Tools:"
    echo "$TOOLS" | jq -r '.[] | "   - \(.name)"' 2>/dev/null | head -5
    if [ "$TOOL_COUNT" -gt 5 ]; then
        echo "   ... and $((TOOL_COUNT - 5)) more"
    fi
else
    echo -e "${RED}✗ No tools found${NC}"
fi

# Check if any agents exist
echo -n "4. Checking agents... "
AGENTS=$(curl -s "http://localhost:8283/v1/agents" 2>/dev/null)
AGENT_COUNT=$(echo "$AGENTS" | jq 'length' 2>/dev/null || echo "0")
if [ "$AGENT_COUNT" -gt 0 ]; then
    echo -e "${GREEN}✓ $AGENT_COUNT agents exist${NC}"
    echo "   Agents:"
    echo "$AGENTS" | jq -r '.[] | "   - \(.name) (ID: \(.id))"' 2>/dev/null | head -3
else
    echo -e "${RED}✗ No agents found${NC}"
    echo "   Agents will be created when users interact"
fi

# Check Pattern process
echo -n "5. Checking Pattern process... "
if pgrep -f "pattern" > /dev/null; then
    echo -e "${GREEN}✓ Running${NC}"
else
    echo -e "${RED}✗ Not running${NC}"
    echo "   Start with: cargo run --features full"
fi

echo ""
echo "Setup verification complete!"