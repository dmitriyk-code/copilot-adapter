#!/bin/bash
# Helper script to debug Copilot responses with detailed logging

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
LOG_FILE="debug_responses_${TIMESTAMP}.log"

echo "Starting copilot-adapter with trace-level logging..."
echo "Log file: $LOG_FILE"
echo ""
echo "This will capture:"
echo "  - Model names being requested"
echo "  - Tools being injected"
echo "  - Raw response content from Copilot"
echo "  - Full JSON responses (trace level)"
echo "  - Tool call parsing results"
echo ""
echo "Press Ctrl+C to stop"
echo ""

# Start the adapter with trace logging
cargo run --release -- start \
    --experimental-tools \
    --log-level trace \
    --log-file "$LOG_FILE"
