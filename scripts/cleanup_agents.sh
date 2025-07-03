#!/usr/bin/env bash
# cleanup_agents.sh - Clean up Pattern agents from Letta (local or cloud)

set -e

echo "🧹 Pattern Agent Cleanup Script"
echo "==============================="
echo

# Determine if using Letta cloud or local
if [ -n "$LETTA_API_KEY" ]; then
    echo "🌐 Using Letta Cloud"
    BASE_URL="https://api.letta.com"
    AUTH_HEADER="Authorization: Bearer $LETTA_API_KEY"
else
    echo "💻 Using Local Letta"
    BASE_URL="http://localhost:8000"
    AUTH_HEADER=""
    
    # Check if local Letta is running
    if ! curl -s "$BASE_URL/v1/health" > /dev/null 2>&1; then
        echo "❌ Letta doesn't appear to be running at $BASE_URL"
        echo "   Please start Letta first with: letta server --port 8000"
        echo "   Or set LETTA_API_KEY to use Letta Cloud"
        exit 1
    fi
fi

# Function to delete agents matching pattern
delete_pattern_agents() {
    echo "🔍 Searching for Pattern agents..."
    
    # Get all agents
    if [ -n "$AUTH_HEADER" ]; then
        AGENTS=$(curl -s -H "$AUTH_HEADER" "$BASE_URL/v1/agents" | jq -r '.[] | select(.name | startswith("pattern_") or startswith("entropy_") or startswith("flux_") or startswith("archive_") or startswith("momentum_") or startswith("anchor_")) | .id + ":" + .name')
    else
        AGENTS=$(curl -s "$BASE_URL/v1/agents" | jq -r '.[] | select(.name | startswith("pattern_") or startswith("entropy_") or startswith("flux_") or startswith("archive_") or startswith("momentum_") or startswith("anchor_")) | .id + ":" + .name')
    fi
    
    if [ -z "$AGENTS" ]; then
        echo "✅ No Pattern agents found to clean up"
        return
    fi
    
    echo "Found the following agents to delete:"
    echo "$AGENTS" | while IFS=: read -r id name; do
        echo "  - $name (ID: $id)"
    done
    echo
    
    read -p "Delete all these agents? (y/N) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        echo "$AGENTS" | while IFS=: read -r id name; do
            echo -n "🗑️  Deleting $name... "
            if [ -n "$AUTH_HEADER" ]; then
                RESULT=$(curl -s -X DELETE -H "$AUTH_HEADER" "$BASE_URL/v1/agents/$id")
            else
                RESULT=$(curl -s -X DELETE "$BASE_URL/v1/agents/$id")
            fi
            if [ $? -eq 0 ]; then
                echo "✅"
            else
                echo "❌ Failed"
            fi
        done
    else
        echo "Cancelled"
    fi
}

# Function to delete specific user's agents
delete_user_agents() {
    local user_id=$1
    echo "🔍 Searching for agents for user $user_id..."
    
    # Pattern agents use a hash of the user ID in the name
    USER_HASH=$(printf "%x" $((user_id % 1000000)))
    
    if [ -n "$AUTH_HEADER" ]; then
        AGENTS=$(curl -s -H "$AUTH_HEADER" "$BASE_URL/v1/agents" | jq -r --arg hash "$USER_HASH" '.[] | select(.name | endswith("_" + $hash)) | .id + ":" + .name')
    else
        AGENTS=$(curl -s "$BASE_URL/v1/agents" | jq -r --arg hash "$USER_HASH" '.[] | select(.name | endswith("_" + $hash)) | .id + ":" + .name')
    fi
    
    if [ -z "$AGENTS" ]; then
        echo "✅ No agents found for user $user_id"
        return
    fi
    
    echo "Found the following agents for user $user_id:"
    echo "$AGENTS" | while IFS=: read -r id name; do
        echo "  - $name (ID: $id)"
    done
    echo
    
    read -p "Delete these agents? (y/N) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        echo "$AGENTS" | while IFS=: read -r id name; do
            echo -n "🗑️  Deleting $name... "
            if [ -n "$AUTH_HEADER" ]; then
                RESULT=$(curl -s -X DELETE -H "$AUTH_HEADER" "$BASE_URL/v1/agents/$id")
            else
                RESULT=$(curl -s -X DELETE "$BASE_URL/v1/agents/$id")
            fi
            if [ $? -eq 0 ]; then
                echo "✅"
            else
                echo "❌ Failed"
            fi
        done
    else
        echo "Cancelled"
    fi
}

# Function to list all agents
list_agents() {
    echo "📋 All agents in Letta:"
    if [ -n "$AUTH_HEADER" ]; then
        curl -s -H "$AUTH_HEADER" "$BASE_URL/v1/agents" | jq -r '.[] | "  - \(.name) (ID: \(.id))"'
    else
        curl -s "$BASE_URL/v1/agents" | jq -r '.[] | "  - \(.name) (ID: \(.id))"'
    fi
}

# Main menu
if [ "$1" == "--user" ] && [ -n "$2" ]; then
    delete_user_agents "$2"
elif [ "$1" == "--list" ]; then
    list_agents
elif [ "$1" == "--all" ]; then
    delete_pattern_agents
else
    echo "Usage:"
    echo "  $0 --all              # Delete all Pattern agents"
    echo "  $0 --user <user_id>   # Delete agents for specific user"
    echo "  $0 --list             # List all agents"
    echo
    echo "Examples:"
    echo "  $0 --all"
    echo "  $0 --user 549170854458687509"
    echo "  $0 --list"
fi