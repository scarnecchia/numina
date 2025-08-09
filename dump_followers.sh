#!/usr/bin/env bash
# Simple script to dump Pattern's followers' DIDs

set -e

# Check if we have jq installed
if ! command -v jq &> /dev/null; then
    echo "Error: jq is not installed. Please install it first."
    exit 1
fi

# Get Pattern's handle from the configuration or environment
PATTERN_HANDLE="${PATTERN_HANDLE:-pattern.atproto.systems}"
OUTPUT_FILE="${1:-pattern_followers.txt}"

echo "Fetching followers for: $PATTERN_HANDLE"
echo "Output file: $OUTPUT_FILE"

# Use the public API to get followers
# We'll paginate through all followers
CURSOR=""
> "$OUTPUT_FILE"  # Clear/create the file

while true; do
    echo -n "Fetching page... "

    # Build the URL with optional cursor
    URL="https://public.api.bsky.app/xrpc/app.bsky.graph.getFollowers?actor=$PATTERN_HANDLE&limit=100"
    if [ -n "$CURSOR" ]; then
        URL="$URL&cursor=$CURSOR"
    fi

    # Fetch the data
    RESPONSE=$(curl -s "$URL")

    # Check for errors
    if echo "$RESPONSE" | jq -e '.error' > /dev/null 2>&1; then
        echo "Error: $(echo "$RESPONSE" | jq -r '.message // .error')"
        exit 1
    fi

    # Extract DIDs from followers
    FOLLOWER_COUNT=$(echo "$RESPONSE" | jq -r '.followers | length')
    echo "Got $FOLLOWER_COUNT followers"

    # Append DIDs to file
    echo "$RESPONSE" | jq -r '.followers[].did' >> "$OUTPUT_FILE"

    # Check if there's a next page
    CURSOR=$(echo "$RESPONSE" | jq -r '.cursor // empty')
    if [ -z "$CURSOR" ]; then
        break
    fi

    # Be nice to the API


    sleep 0.5
done

# Count total followers
TOTAL=$(wc -l < "$OUTPUT_FILE")
echo "Total followers: $TOTAL"
echo "DIDs saved to: $OUTPUT_FILE"

# Create a JSON file as well for easier parsing
JSON_FILE="${OUTPUT_FILE%.txt}.json"
jq -Rs 'split("\n") | map(select(length > 0))' "$OUTPUT_FILE" > "$JSON_FILE"
echo "JSON array saved to: $JSON_FILE"
