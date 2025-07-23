#!/bin/bash
# Test script for archival memory search

set -e

echo "=== Archival Memory Search Test ==="
echo

# First, create an agent if it doesn't exist
echo "1. Creating test agent..."
./target/debug/pattern-cli agent create TestAgent || echo "Agent might already exist"
echo

# List all agents to confirm
echo "2. Listing agents..."
./target/debug/pattern-cli agent list
echo

# Show agent status
echo "3. Agent status..."
./target/debug/pattern-cli agent status TestAgent
echo

# List archival memories for the agent
echo "4. Listing archival memories..."
./target/debug/pattern-cli debug list-archival --agent TestAgent
echo

# Try searching for archival memories
echo "5. Searching archival memories..."
echo "   Search query: 'color'"
./target/debug/pattern-cli debug search-archival --agent TestAgent "color"
echo

echo "6. Direct database query to check memory blocks..."
./target/debug/pattern-cli db query "SELECT id, owner_id, label, memory_type, value FROM mem WHERE memory_type = 'archival' LIMIT 5"
echo

echo "7. Check full-text search index..."
./target/debug/pattern-cli db query "INFO FOR TABLE mem"
echo

echo "=== Test complete ==="