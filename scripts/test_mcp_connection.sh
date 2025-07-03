#!/bin/bash
# Test script to verify MCP HTTP server is responding

echo "Testing MCP server connection..."
echo "================================"

# Wait a moment for server to start
sleep 3

# Test basic HTTP connection
echo "Testing HTTP connection to MCP server..."
curl -v http://localhost:8080/mcp 2>&1 | grep -E "Connected|HTTP" || echo "Connection failed"

echo ""
echo "Testing MCP protocol..."
# Send a basic JSON-RPC request to get server info
curl -X POST http://localhost:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc": "2.0", "method": "initialize", "params": {"protocolVersion": "0.1.0", "capabilities": {}}, "id": 1}' \
  -v 2>&1

echo ""
echo "Done."