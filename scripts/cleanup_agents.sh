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
    BASE_URL="http://localhost:8283"
    AUTH_HEADER=""

    # Check if local Letta is running (handle redirects with -L)
    if ! curl -sL "$BASE_URL/v1/health/" > /dev/null 2>&1; then
        echo "❌ Letta doesn't appear to be running at $BASE_URL"
        echo "   Please start Letta first with: letta server --port 8283"
        echo "   Or set LETTA_API_KEY to use Letta Cloud"
        exit 1
    fi
fi

# Function to delete groups containing an agent
delete_groups_for_agent() {
    local agent_id=$1
    local agent_name=$2

    # Get groups this agent is in
    if [ -n "$AUTH_HEADER" ]; then
        GROUPS_JSON=$(curl -sL -H "$AUTH_HEADER" "$BASE_URL/v1/agents/$agent_id/groups" 2>/dev/null || echo "[]")
    else
        GROUPS_JSON=$(curl -sL "$BASE_URL/v1/agents/$agent_id/groups" 2>/dev/null || echo "[]")
    fi

    # Check if we got valid JSON and it's not empty
    if echo "$GROUPS_JSON" | jq -e '. | length > 0' >/dev/null 2>&1; then
        echo "  Agent $agent_name is in groups:"

        # Process each group
        echo "$GROUPS_JSON" | jq -r '.[] | @base64' | while read -r group_data; do
            # Decode the group data
            GROUP=$(echo "$group_data" | base64 -d)
            GROUP_ID=$(echo "$GROUP" | jq -r '.id')
            GROUP_DESC=$(echo "$GROUP" | jq -r '.description')

            echo "    - $GROUP_DESC (ID: $GROUP_ID)"
            echo -n "    🗑️  Deleting group $GROUP_DESC... "

            if [ -n "$AUTH_HEADER" ]; then
                HTTP_STATUS=$(curl -sL -w "%{http_code}" -X DELETE -H "$AUTH_HEADER" "$BASE_URL/v1/groups/$GROUP_ID" -o /dev/null)
            else
                HTTP_STATUS=$(curl -sL -w "%{http_code}" -X DELETE "$BASE_URL/v1/groups/$GROUP_ID" -o /dev/null)
            fi

            if [ "$HTTP_STATUS" -eq 200 ] || [ "$HTTP_STATUS" -eq 204 ]; then
                echo "✅"
            else
                echo "❌ Failed (HTTP $HTTP_STATUS)"
            fi
        done
    fi
}

# Function to delete agents matching pattern
delete_pattern_agents() {
    echo "🔍 Searching for Pattern agents..."

    # Get all agents
    if [ -n "$AUTH_HEADER" ]; then
        AGENTS_JSON=$(curl -sL -H "$AUTH_HEADER" "$BASE_URL/v1/agents")
    else
        AGENTS_JSON=$(curl -sL "$BASE_URL/v1/agents")
    fi

    # Filter for pattern agents
    PATTERN_AGENTS=$(echo "$AGENTS_JSON" | jq -r '.[] | select(.name | startswith("pattern_") or startswith("entropy_") or startswith("flux_") or startswith("archive_") or startswith("momentum_") or startswith("anchor_")) | @base64')

    if [ -z "$PATTERN_AGENTS" ]; then
        echo "✅ No Pattern agents found to clean up"
        return
    fi

    echo "Found the following agents to delete:"

    # First pass: display agents and their groups
    echo "$PATTERN_AGENTS" | while read -r agent_data; do
        # Decode the agent data
        AGENT=$(echo "$agent_data" | base64 -d)
        ID=$(echo "$AGENT" | jq -r '.id')
        NAME=$(echo "$AGENT" | jq -r '.name')

        echo "  - $NAME (ID: $ID)"

        # Get groups for this agent
        if [ -n "$AUTH_HEADER" ]; then
            GROUPS_JSON=$(curl -sL -H "$AUTH_HEADER" "$BASE_URL/v1/agents/$ID/groups" 2>/dev/null || echo "[]")
        else
            GROUPS_JSON=$(curl -sL "$BASE_URL/v1/agents/$ID/groups" 2>/dev/null || echo "[]")
        fi

        # Check if we got valid JSON
        if echo "$GROUPS_JSON" | jq -e '. | length > 0' >/dev/null 2>&1; then
            echo "$GROUPS_JSON" | jq -r '.[] | "    In group: " + .description + " (ID: " + .id + ")"'
        fi
    done
    echo

    read -p "Delete all these agents and their groups? (y/N) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        # Second pass: delete groups
        echo "🗑️  Deleting groups first..."
        echo "$PATTERN_AGENTS" | while read -r agent_data; do
            # Decode the agent data
            AGENT=$(echo "$agent_data" | base64 -d)
            ID=$(echo "$AGENT" | jq -r '.id')
            NAME=$(echo "$AGENT" | jq -r '.name')

            delete_groups_for_agent "$ID" "$NAME"
        done

        # Third pass: delete agents
        echo
        echo "🗑️  Now deleting agents..."
        echo "$PATTERN_AGENTS" | while read -r agent_data; do
            # Decode the agent data
            AGENT=$(echo "$agent_data" | base64 -d)
            ID=$(echo "$AGENT" | jq -r '.id')
            NAME=$(echo "$AGENT" | jq -r '.name')

            echo -n "🗑️  Deleting $NAME... "
            if [ -n "$AUTH_HEADER" ]; then
                HTTP_STATUS=$(curl -sL -w "%{http_code}" -X DELETE -H "$AUTH_HEADER" "$BASE_URL/v1/agents/$ID" -o /dev/null)
            else
                HTTP_STATUS=$(curl -sL -w "%{http_code}" -X DELETE "$BASE_URL/v1/agents/$ID" -o /dev/null)
            fi

            if [ "$HTTP_STATUS" -eq 200 ] || [ "$HTTP_STATUS" -eq 204 ]; then
                echo "✅"
            else
                echo "❌ Failed (HTTP $HTTP_STATUS)"
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
        AGENTS=$(curl -sL -H "$AUTH_HEADER" "$BASE_URL/v1/agents" | jq -r --arg hash "$USER_HASH" '.[] | select(.name | endswith("_" + $hash)) | .id + ":" + .name')
    else
        AGENTS=$(curl -sL "$BASE_URL/v1/agents" | jq -r --arg hash "$USER_HASH" '.[] | select(.name | endswith("_" + $hash)) | .id + ":" + .name')
    fi

    if [ -z "$AGENTS" ]; then
        echo "✅ No agents found for user $user_id"
        return
    fi

    echo "Found the following agents for user $user_id:"
    echo "$AGENTS" | while IFS=: read -r id name; do
        echo "  - $name (ID: $id)"

        # Show groups this agent is in
        if [ -n "$AUTH_HEADER" ]; then
            GROUPS=$(curl -sL -H "$AUTH_HEADER" "$BASE_URL/v1/agents/$id/groups" 2>/dev/null | jq -r '.[] | "    In group: " + .name + " (ID: " + .id + ")"' 2>/dev/null)
        else
            GROUPS=$(curl -sL "$BASE_URL/v1/agents/$id/groups" 2>/dev/null | jq -r '.[] | "    In group: " + .name + " (ID: " + .id + ")"' 2>/dev/null)
        fi

        if [ -n "$GROUPS" ]; then
            echo "$GROUPS"
        fi
    done
    echo

    read -p "Delete these agents and their groups? (y/N) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        # First delete groups
        echo "🗑️  Deleting groups first..."
        echo "$AGENTS" | while IFS=: read -r id name; do
            delete_groups_for_agent "$id" "$name"
        done

        # Then delete agents
        echo
        echo "🗑️  Now deleting agents..."
        echo "$AGENTS" | while IFS=: read -r id name; do
            echo -n "🗑️  Deleting $name... "
            if [ -n "$AUTH_HEADER" ]; then
                RESULT=$(curl -sL -X DELETE -H "$AUTH_HEADER" "$BASE_URL/v1/agents/$id")
            else
                RESULT=$(curl -sL -X DELETE "$BASE_URL/v1/agents/$id")
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

    # Get all agents first
    if [ -n "$AUTH_HEADER" ]; then
        AGENTS_JSON=$(curl -sL -H "$AUTH_HEADER" "$BASE_URL/v1/agents")
    else
        AGENTS_JSON=$(curl -sL "$BASE_URL/v1/agents")
    fi

    # Process each agent
    echo "$AGENTS_JSON" | jq -r '.[] | @base64' | while read -r agent_data; do
        # Decode the agent data
        AGENT=$(echo "$agent_data" | base64 -d)
        ID=$(echo "$AGENT" | jq -r '.id')
        NAME=$(echo "$AGENT" | jq -r '.name')

        echo "  - $NAME (ID: $ID)"

        # Get groups for this agent
        if [ -n "$AUTH_HEADER" ]; then
            GROUPS_JSON=$(curl -sL -H "$AUTH_HEADER" "$BASE_URL/v1/agents/$ID/groups" 2>/dev/null || echo "[]")
        else
            GROUPS_JSON=$(curl -sL "$BASE_URL/v1/agents/$ID/groups" 2>/dev/null || echo "[]")
        fi

        # Check if we got valid JSON
        if echo "$GROUPS_JSON" | jq -e . >/dev/null 2>&1; then
            # Process groups if any exist
            GROUP_COUNT=$(echo "$GROUPS_JSON" | jq '. | length')
            if [ "$GROUP_COUNT" -gt 0 ]; then
                echo "$GROUPS_JSON" | jq -r '.[] | "    In group: " + .description + " (ID: " + .id + ")"'
            fi
        fi
    done
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
