#!/bin/bash
# Test script for group functionality

echo "=== Testing Group Functionality ==="
echo

# First, let's create some agents
echo "1. Creating test agents..."
cargo run --bin pattern-cli -- agent create TestAgent1 --agent-type generic
cargo run --bin pattern-cli -- agent create TestAgent2 --agent-type generic
cargo run --bin pattern-cli -- agent create TestAgent3 --agent-type generic

echo
echo "2. Creating a test group..."
cargo run --bin pattern-cli -- group create TestGroup -d "A test group for development" -p round_robin

echo
echo "3. Listing groups..."
cargo run --bin pattern-cli -- group list

echo
echo "4. Adding agents to the group..."
cargo run --bin pattern-cli -- group add-member TestGroup TestAgent1 --role regular
cargo run --bin pattern-cli -- group add-member TestGroup TestAgent2 --role regular
cargo run --bin pattern-cli -- group add-member TestGroup TestAgent3 --role supervisor

echo
echo "5. Checking group status..."
cargo run --bin pattern-cli -- group status TestGroup

echo
echo "6. Testing group chat..."
echo "quit" | cargo run --bin pattern-cli -- chat --group TestGroup

echo
echo "=== Test Complete ==="