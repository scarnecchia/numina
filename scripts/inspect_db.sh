#!/usr/bin/env bash
# Inspect Pattern database

DB_FILE="${1:-test_pattern.db}"

echo "Inspecting database: $DB_FILE"
echo "==============================="

# Check if database exists
if [ ! -f "$DB_FILE" ]; then
    echo "Database file not found: $DB_FILE"
    exit 1
fi

# Use Python to inspect SQLite database
python3 << EOF
import sqlite3
import json

conn = sqlite3.connect('$DB_FILE')
cursor = conn.cursor()

print("\n1. Users in database:")
print("-" * 50)
cursor.execute("SELECT id, name, discord_id, created_at FROM users")
for row in cursor.fetchall():
    print(f"ID: {row[0]}, Name: {row[1]}, Discord: {row[2]}, Created: {row[3]}")

print("\n2. Agents in database:")
print("-" * 50)
cursor.execute("SELECT id, user_id, letta_agent_id, name, created_at FROM agents")
for row in cursor.fetchall():
    print(f"ID: {row[0]}, User: {row[1]}, Agent ID: {row[2]}, Name: {row[3]}, Created: {row[4]}")

print("\n3. Shared memory blocks:")
print("-" * 50)
cursor.execute("SELECT user_id, block_name, block_value FROM shared_memory LIMIT 10")
for row in cursor.fetchall():
    print(f"User: {row[0]}, Block: {row[1]}, Value: {row[2][:50]}...")

conn.close()
EOF

echo ""
echo "Now checking Letta agents..."
echo "-" 
curl -s "http://localhost:8283/v1/agents" | python3 -c "
import sys, json
agents = json.load(sys.stdin)
print(f'\\nLetta has {len(agents)} agents:')
for agent in agents[:10]:  # Show first 10
    print(f\"  - {agent.get('name', 'Unknown')} (ID: {agent.get('id', 'Unknown')})\")"