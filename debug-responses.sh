#!/bin/bash
# Helper script to debug Copilot responses with detailed logging

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
LOG_FILE="debug_responses_${TIMESTAMP}.log"

echo "========================================="
echo "Copilot Adapter Debug Helper"
echo "========================================="
echo ""
echo "This will:"
echo "  1. Build the adapter in release mode"
echo "  2. Force re-authentication"
echo "  3. Start with trace-level logging"
echo ""
echo "Logged to: $LOG_FILE"
echo ""

# Step 1: Build
echo "[1/3] Building copilot-adapter (release mode)..."
cargo build --release
if [ $? -ne 0 ]; then
    echo "ERROR: Build failed!"
    exit 1
fi
echo "✓ Build successful"
echo ""

# Step 2: Force re-authentication
echo "[2/3] Forcing re-authentication..."
./target/release/copilot-adapter auth --force
if [ $? -ne 0 ]; then
    echo "ERROR: Authentication failed!"
    exit 1
fi
echo "✓ Authentication successful"
echo ""

# Step 3: Start with logging
echo "[3/3] Starting adapter with trace-level logging..."
echo ""
echo "This will capture:"
echo "  - Model names being requested"
echo "  - Tools being injected"
echo "  - Raw response content from Copilot (streaming)"
echo "  - Full JSON responses (trace level)"
echo "  - Tool call parsing results"
echo ""
echo "Press Ctrl+C to stop"
echo ""

# Start the adapter with trace logging
./target/release/copilot-adapter start \
    --experimental-tools \
    --log-level trace \
    --log-file "$LOG_FILE"
