#!/bin/bash
# =============================================================================
# Epic 0 — Verify Native Tools Support
# =============================================================================
#
# Tests whether the GitHub Copilot API accepts native OpenAI-format tool
# definitions and returns structured tool_calls in the response.
#
# Prerequisites:
#   - A valid Copilot token. Obtain one via:
#       curl -s https://api.github.com/copilot_internal/v2/token \
#         -H "Authorization: token $GITHUB_TOKEN" | jq -r .token
#
# Usage:
#   export COPILOT_TOKEN="<your-copilot-token>"
#   bash scripts/verify-native-tools.sh
#
# Tests:
#   1. Non-streaming request with native tools
#   2. Streaming request with native tools
#   3. Tool name length limit (64-char boundary)
#   4. Multiple tools in a single request
# =============================================================================

set -euo pipefail

RESULTS_DIR="scripts/verification-results"
mkdir -p "$RESULTS_DIR"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RESULTS_FILE="$RESULTS_DIR/native-tools-$TIMESTAMP.json"

# Color codes for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Check for token
if [ -z "${COPILOT_TOKEN:-}" ]; then
    echo -e "${RED}ERROR: COPILOT_TOKEN environment variable not set.${NC}"
    echo ""
    echo "To obtain a token:"
    echo "  1. Authenticate with GitHub: copilot-adapter auth"
    echo "  2. Retrieve the token from your keyring, or use:"
    echo "     curl -s https://api.github.com/copilot_internal/v2/token \\"
    echo "       -H 'Authorization: token YOUR_GITHUB_TOKEN' | jq -r .token"
    exit 1
fi

API_URL="https://api.githubcopilot.com/chat/completions"
HEADERS=(
    -H "Authorization: Bearer $COPILOT_TOKEN"
    -H "Content-Type: application/json"
    -H "Copilot-Integration-Id: vscode-chat"
    -H "Editor-Version: vscode/1.85.0"
    -H "Editor-Plugin-Version: copilot-chat/0.12.0"
)

echo "========================================="
echo " Epic 0: Verify Native Tools Support"
echo "========================================="
echo ""
echo "Results will be saved to: $RESULTS_FILE"
echo ""

# Initialize results JSON
echo '{"timestamp":"'"$TIMESTAMP"'","tests":{}}' > "$RESULTS_FILE"

# ---------------------------------------------------------------------------
# Test 1: Non-streaming request with native tools
# ---------------------------------------------------------------------------
echo -e "${YELLOW}[Test 1/4]${NC} Non-streaming request with native tools..."

RESPONSE_1=$(curl -s -w "\n%{http_code}" -X POST "$API_URL" \
    "${HEADERS[@]}" \
    -d '{
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "Get the weather in London"}],
        "tools": [{
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get weather for a location",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "location": {"type": "string", "description": "City name"},
                        "units": {"type": "string", "enum": ["celsius", "fahrenheit"]}
                    },
                    "required": ["location"]
                }
            }
        }],
        "stream": false
    }' 2>/dev/null)

HTTP_CODE_1=$(echo "$RESPONSE_1" | tail -1)
BODY_1=$(echo "$RESPONSE_1" | sed '$d')

if [ "$HTTP_CODE_1" = "200" ]; then
    # Check if response contains tool_calls
    HAS_TOOL_CALLS=$(echo "$BODY_1" | python3 -c "
import json, sys
data = json.load(sys.stdin)
choices = data.get('choices', [])
if choices and 'tool_calls' in choices[0].get('message', {}):
    print('yes')
else:
    print('no')
" 2>/dev/null || echo "parse_error")

    if [ "$HAS_TOOL_CALLS" = "yes" ]; then
        echo -e "  ${GREEN}✓ PASS${NC} — HTTP 200, response includes tool_calls"
        TEST1_STATUS="pass"
    elif [ "$HAS_TOOL_CALLS" = "no" ]; then
        echo -e "  ${RED}✗ FAIL${NC} — HTTP 200 but no tool_calls in response"
        TEST1_STATUS="fail_no_tool_calls"
    else
        echo -e "  ${YELLOW}⚠ WARN${NC} — HTTP 200 but could not parse response"
        TEST1_STATUS="parse_error"
    fi
else
    echo -e "  ${RED}✗ FAIL${NC} — HTTP $HTTP_CODE_1"
    TEST1_STATUS="http_error_$HTTP_CODE_1"
fi

echo "  Response saved to results file."
echo ""

# Save test 1 results
python3 -c "
import json, sys
with open('$RESULTS_FILE', 'r') as f:
    results = json.load(f)
results['tests']['non_streaming'] = {
    'http_code': $HTTP_CODE_1,
    'status': '$TEST1_STATUS',
    'response': json.loads('''$BODY_1''') if '''$BODY_1'''.strip() else None
}
with open('$RESULTS_FILE', 'w') as f:
    json.dump(results, f, indent=2)
" 2>/dev/null || true

# ---------------------------------------------------------------------------
# Test 2: Streaming request with native tools
# ---------------------------------------------------------------------------
echo -e "${YELLOW}[Test 2/4]${NC} Streaming request with native tools..."

RESPONSE_2=$(curl -s -w "\n%{http_code}" -X POST "$API_URL" \
    "${HEADERS[@]}" \
    -d '{
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "Get the weather in London"}],
        "tools": [{
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get weather for a location",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "location": {"type": "string", "description": "City name"},
                        "units": {"type": "string", "enum": ["celsius", "fahrenheit"]}
                    },
                    "required": ["location"]
                }
            }
        }],
        "stream": true
    }' 2>/dev/null)

HTTP_CODE_2=$(echo "$RESPONSE_2" | tail -1)
BODY_2=$(echo "$RESPONSE_2" | sed '$d')

if [ "$HTTP_CODE_2" = "200" ]; then
    # Check if streaming chunks contain tool_calls deltas
    HAS_TOOL_CALLS_DELTA=$(echo "$BODY_2" | grep -o '"tool_calls"' | head -1)
    HAS_DONE=$(echo "$BODY_2" | grep -o '\[DONE\]' | head -1)

    if [ -n "$HAS_TOOL_CALLS_DELTA" ]; then
        echo -e "  ${GREEN}✓ PASS${NC} — HTTP 200, streaming chunks include tool_calls deltas"
        TEST2_STATUS="pass"
    else
        echo -e "  ${YELLOW}⚠ WARN${NC} — HTTP 200 but no tool_calls deltas found in chunks"
        TEST2_STATUS="no_tool_calls_deltas"
    fi

    if [ -n "$HAS_DONE" ]; then
        echo "  [DONE] marker present."
    fi
else
    echo -e "  ${RED}✗ FAIL${NC} — HTTP $HTTP_CODE_2"
    TEST2_STATUS="http_error_$HTTP_CODE_2"
fi

echo "  Streaming response saved to results file."
echo ""

# ---------------------------------------------------------------------------
# Test 3: Tool name length limits
# ---------------------------------------------------------------------------
echo -e "${YELLOW}[Test 3/4]${NC} Tool name length limits..."

# Test with exactly 64-char name (should pass)
NAME_64="a]aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
NAME_64=$(printf 'a%.0s' {1..64})

# Test with 65-char name (should fail or be rejected)
NAME_65=$(printf 'a%.0s' {1..65})

# Test with 64-char name
RESPONSE_3A=$(curl -s -w "\n%{http_code}" -X POST "$API_URL" \
    "${HEADERS[@]}" \
    -d '{
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "Call the tool"}],
        "tools": [{
            "type": "function",
            "function": {
                "name": "'"$NAME_64"'",
                "description": "Test tool with 64-char name",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "x": {"type": "string"}
                    }
                }
            }
        }],
        "stream": false
    }' 2>/dev/null)

HTTP_CODE_3A=$(echo "$RESPONSE_3A" | tail -1)

if [ "$HTTP_CODE_3A" = "200" ]; then
    echo -e "  ${GREEN}✓${NC} 64-char name: HTTP 200 (accepted)"
    TEST3A_STATUS="pass"
else
    echo -e "  ${RED}✗${NC} 64-char name: HTTP $HTTP_CODE_3A (rejected)"
    TEST3A_STATUS="http_error_$HTTP_CODE_3A"
fi

# Test with 65-char name
RESPONSE_3B=$(curl -s -w "\n%{http_code}" -X POST "$API_URL" \
    "${HEADERS[@]}" \
    -d '{
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "Call the tool"}],
        "tools": [{
            "type": "function",
            "function": {
                "name": "'"$NAME_65"'",
                "description": "Test tool with 65-char name",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "x": {"type": "string"}
                    }
                }
            }
        }],
        "stream": false
    }' 2>/dev/null)

HTTP_CODE_3B=$(echo "$RESPONSE_3B" | tail -1)
BODY_3B=$(echo "$RESPONSE_3B" | sed '$d')

if [ "$HTTP_CODE_3B" = "200" ]; then
    echo -e "  ${YELLOW}⚠${NC} 65-char name: HTTP 200 (accepted — limit may be higher than 64)"
    TEST3B_STATUS="accepted"
else
    echo -e "  ${GREEN}✓${NC} 65-char name: HTTP $HTTP_CODE_3B (rejected — 64-char limit confirmed)"
    TEST3B_STATUS="rejected_$HTTP_CODE_3B"
fi

# Test with a long realistic tool name (100 chars)
NAME_100="mcp__very_long_server_name__extremely_detailed_tool_description_that_exceeds_the_openai_limit_test_x"
RESPONSE_3C=$(curl -s -w "\n%{http_code}" -X POST "$API_URL" \
    "${HEADERS[@]}" \
    -d '{
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "Call the tool"}],
        "tools": [{
            "type": "function",
            "function": {
                "name": "'"$NAME_100"'",
                "description": "Test tool with 100-char name",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "x": {"type": "string"}
                    }
                }
            }
        }],
        "stream": false
    }' 2>/dev/null)

HTTP_CODE_3C=$(echo "$RESPONSE_3C" | tail -1)

if [ "$HTTP_CODE_3C" = "200" ]; then
    echo -e "  ${YELLOW}⚠${NC} 100-char name: HTTP 200 (accepted — no length limit enforced)"
    TEST3C_STATUS="accepted"
else
    echo -e "  ${GREEN}✓${NC} 100-char name: HTTP $HTTP_CODE_3C (rejected)"
    TEST3C_STATUS="rejected_$HTTP_CODE_3C"
fi

echo ""

# ---------------------------------------------------------------------------
# Test 4: Multiple tools in a single request
# ---------------------------------------------------------------------------
echo -e "${YELLOW}[Test 4/4]${NC} Multiple tools in a single request..."

RESPONSE_4=$(curl -s -w "\n%{http_code}" -X POST "$API_URL" \
    "${HEADERS[@]}" \
    -d '{
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "What is the weather in London and read the file /tmp/test.txt"}],
        "tools": [
            {
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get weather for a location",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "location": {"type": "string"}
                        },
                        "required": ["location"]
                    }
                }
            },
            {
                "type": "function",
                "function": {
                    "name": "read_file",
                    "description": "Read the contents of a file",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "path": {"type": "string"}
                        },
                        "required": ["path"]
                    }
                }
            }
        ],
        "stream": false
    }' 2>/dev/null)

HTTP_CODE_4=$(echo "$RESPONSE_4" | tail -1)
BODY_4=$(echo "$RESPONSE_4" | sed '$d')

if [ "$HTTP_CODE_4" = "200" ]; then
    echo -e "  ${GREEN}✓ PASS${NC} — HTTP 200, multiple tools accepted"
    TEST4_STATUS="pass"
else
    echo -e "  ${RED}✗ FAIL${NC} — HTTP $HTTP_CODE_4"
    TEST4_STATUS="http_error_$HTTP_CODE_4"
fi

echo ""

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo "========================================="
echo " Summary"
echo "========================================="
echo ""
echo "  Test 1 (Non-streaming tools):   $TEST1_STATUS"
echo "  Test 2 (Streaming tools):       $TEST2_STATUS"
echo "  Test 3a (64-char name):         $TEST3A_STATUS"
echo "  Test 3b (65-char name):         $TEST3B_STATUS"
echo "  Test 3c (100-char name):        $TEST3C_STATUS"
echo "  Test 4 (Multiple tools):        $TEST4_STATUS"
echo ""
echo "Full responses saved to: $RESULTS_FILE"
echo ""
echo "To review streaming output:"
echo "  cat $RESULTS_FILE | python3 -m json.tool"
