#!/usr/bin/env bash
# Quick test script for Pattern Discord bot

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${GREEN}Pattern Test Script${NC}"
echo "==================="

# Check if .env exists
if [ ! -f .env ]; then
    echo -e "${RED}Error: .env file not found!${NC}"
    echo "Please create a .env file with:"
    echo "  DISCORD_TOKEN=your_token_here"
    echo "  LETTA_BASE_URL=http://localhost:8283"
    exit 1
fi

# Set up environment for HTTP transport
export DATABASE_PATH=test_pattern.db

# Load .env
export $(cat .env | grep -v '^#' | xargs)

# Check if DISCORD_TOKEN is set
if [ -z "$DISCORD_TOKEN" ]; then
    echo -e "${RED}Error: DISCORD_TOKEN not set in .env!${NC}"
    exit 1
fi

# Check if Letta is running
echo -e "${YELLOW}Checking if Letta is running...${NC}"
if ! curl -s http://localhost:8283/v1/health > /dev/null 2>&1; then
    echo -e "${YELLOW}Letta doesn't seem to be running. Starting Docker container...${NC}"

    # Start Letta Docker container
    if ! docker compose -f docker-compose.dev.yml up -d; then
        echo -e "${RED}Failed to start Letta Docker container${NC}"
        echo "Make sure Docker is running"
        exit 1
    fi

    # Wait for Letta to start
    echo "Waiting for Letta to start..."
    for i in {1..30}; do
        if curl -s http://localhost:8283/v1/health > /dev/null 2>&1; then
            echo -e "${GREEN}Letta started successfully${NC}"
            LETTA_STARTED=true
            break
        fi
        echo -n "."
        sleep 1
    done
    echo ""

    if [ -z "$LETTA_STARTED" ]; then
        echo -e "${RED}Letta failed to start after 30 seconds${NC}"
        echo "Check Docker logs with: docker compose -f docker-compose.dev.yml logs"
        exit 1
    fi

    # No need to change directories anymore
else
    echo -e "${GREEN}Letta is already running${NC}"
fi

# Run Pattern
echo -e "${YELLOW}Starting Pattern Discord bot...${NC}"
echo "Press Ctrl+C to stop"
echo ""

# Set logging and run
RUST_LOG=pattern=info cargo run --bin pattern --features full

# Note about cleanup
if [ ! -z "$LETTA_STARTED" ]; then
    echo -e "${YELLOW}Note: Letta Docker container is still running${NC}"
    echo "To stop it: docker compose -f docker-compose.dev.yml down"
fi

echo -e "${GREEN}Done!${NC}"
