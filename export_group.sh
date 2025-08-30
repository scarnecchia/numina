#!/bin/bash

# export_group.sh - Export Pattern group via cron
# Usage: ./export_group.sh <config_path> <group_string> <output_dir>

set -e

# Check if all required arguments are provided
if [ $# -ne 3 ]; then
    echo "Usage: $0 <config_path> <group_string> <output_dir>" >&2
    echo "Example: $0 /path/to/config.toml 'Cluster A' /path/to/backups" >&2
    exit 1
fi

CONFIG_PATH="$1"
GROUP_STRING="$2"
OUTPUT_DIR="$3"

# Validate config file exists
if [ ! -f "$CONFIG_PATH" ]; then
    echo "Error: Config file '$CONFIG_PATH' not found" >&2
    exit 1
fi

# Create output directory if it doesn't exist
mkdir -p "$OUTPUT_DIR"

# Generate original filename (group string with spaces -> hyphens, case preserved)
ORIGINAL_FILENAME=$(echo "$GROUP_STRING" | tr ' ' '-').car

# Generate timestamped filename
TIMESTAMP=$(date +%s)
TIMESTAMPED_FILENAME=$(echo "$GROUP_STRING" | tr ' ' '-')_${TIMESTAMP}.car
FINAL_OUTPUT_PATH="$OUTPUT_DIR/$TIMESTAMPED_FILENAME"

# Run the cargo command (it will create the original filename)
cargo run --bin pattern-cli -- -c "$CONFIG_PATH" export group "$GROUP_STRING"

# Move the original file to the timestamped name in the output directory
if [ -f "$ORIGINAL_FILENAME" ]; then
    mv "$ORIGINAL_FILENAME" "$FINAL_OUTPUT_PATH"
else
    echo "Error: Expected output file '$ORIGINAL_FILENAME' not found" >&2
    exit 1
fi

# Delete backups older than 7 days
find "$OUTPUT_DIR" -name "*_*.car" -type f -mtime +7 -delete

echo "Export completed: $FINAL_OUTPUT_PATH"
echo "Cleaned up backups older than 7 days from: $OUTPUT_DIR"