# =============================================================================
# Epic 0 — Verify Native Tools Support (Windows PowerShell)
# =============================================================================
#
# Tests whether the GitHub Copilot API accepts native OpenAI-format tool
# definitions and returns structured tool_calls in the response.
#
# Prerequisites:
#   - A valid Copilot token set in $env:COPILOT_TOKEN
#
# Usage:
#   $env:COPILOT_TOKEN = "<your-copilot-token>"
#   .\scripts\verify-native-tools.ps1
# =============================================================================

$ErrorActionPreference = "Stop"

if (-not $env:COPILOT_TOKEN) {
    Write-Host "ERROR: COPILOT_TOKEN environment variable not set." -ForegroundColor Red
    Write-Host ""
    Write-Host "To obtain a token, authenticate with copilot-adapter and retrieve"
    Write-Host "the Copilot token from the API."
    exit 1
}

$ApiUrl = "https://api.githubcopilot.com/chat/completions"
$Headers = @{
    "Authorization"          = "Bearer $env:COPILOT_TOKEN"
    "Content-Type"           = "application/json"
    "Copilot-Integration-Id" = "vscode-chat"
    "Editor-Version"         = "vscode/1.85.0"
    "Editor-Plugin-Version"  = "copilot-chat/0.12.0"
}

Write-Host "========================================="
Write-Host " Epic 0: Verify Native Tools Support"
Write-Host "========================================="
Write-Host ""

# ---------------------------------------------------------------------------
# Test 1: Non-streaming request with native tools
# ---------------------------------------------------------------------------
Write-Host "[Test 1/4] Non-streaming request with native tools..." -ForegroundColor Yellow

$Body1 = @{
    model    = "gpt-4o"
    messages = @(@{ role = "user"; content = "Get the weather in London" })
    tools    = @(@{
        type     = "function"
        function = @{
            name        = "get_weather"
            description = "Get weather for a location"
            parameters  = @{
                type       = "object"
                properties = @{
                    location = @{ type = "string"; description = "City name" }
                    units    = @{ type = "string"; enum = @("celsius", "fahrenheit") }
                }
                required   = @("location")
            }
        }
    })
    stream   = $false
} | ConvertTo-Json -Depth 10

try {
    $Response1 = Invoke-RestMethod -Uri $ApiUrl -Method Post -Headers $Headers -Body $Body1 -ContentType "application/json"
    $HasToolCalls = $null -ne $Response1.choices[0].message.tool_calls
    if ($HasToolCalls) {
        Write-Host "  PASS - HTTP 200, response includes tool_calls" -ForegroundColor Green
        $Response1.choices[0].message.tool_calls | ConvertTo-Json -Depth 5 | Write-Host
    } else {
        Write-Host "  WARN - HTTP 200 but no tool_calls in response" -ForegroundColor Yellow
        $Response1 | ConvertTo-Json -Depth 5 | Write-Host
    }
} catch {
    Write-Host "  FAIL - $($_.Exception.Message)" -ForegroundColor Red
}

Write-Host ""

# ---------------------------------------------------------------------------
# Test 2: Streaming request with native tools
# ---------------------------------------------------------------------------
Write-Host "[Test 2/4] Streaming request with native tools..." -ForegroundColor Yellow

$Body2 = @{
    model    = "gpt-4o"
    messages = @(@{ role = "user"; content = "Get the weather in London" })
    tools    = @(@{
        type     = "function"
        function = @{
            name        = "get_weather"
            description = "Get weather for a location"
            parameters  = @{
                type       = "object"
                properties = @{
                    location = @{ type = "string"; description = "City name" }
                }
                required   = @("location")
            }
        }
    })
    stream   = $true
} | ConvertTo-Json -Depth 10

try {
    $Response2 = Invoke-WebRequest -Uri $ApiUrl -Method Post -Headers $Headers -Body $Body2 -ContentType "application/json"
    $Content = $Response2.Content
    if ($Content -match '"tool_calls"') {
        Write-Host "  PASS - Streaming chunks include tool_calls deltas" -ForegroundColor Green
    } else {
        Write-Host "  WARN - No tool_calls deltas in streaming chunks" -ForegroundColor Yellow
    }
    Write-Host "  First 500 chars of stream:"
    Write-Host ($Content.Substring(0, [Math]::Min(500, $Content.Length)))
} catch {
    Write-Host "  FAIL - $($_.Exception.Message)" -ForegroundColor Red
}

Write-Host ""

# ---------------------------------------------------------------------------
# Test 3: Tool name length limits
# ---------------------------------------------------------------------------
Write-Host "[Test 3/4] Tool name length limits..." -ForegroundColor Yellow

foreach ($Length in @(64, 65, 100)) {
    $Name = "a" * $Length
    $Body3 = @{
        model    = "gpt-4o"
        messages = @(@{ role = "user"; content = "Call the tool" })
        tools    = @(@{
            type     = "function"
            function = @{
                name        = $Name
                description = "Test tool with $Length-char name"
                parameters  = @{
                    type       = "object"
                    properties = @{ x = @{ type = "string" } }
                }
            }
        })
        stream   = $false
    } | ConvertTo-Json -Depth 10

    try {
        $Response3 = Invoke-RestMethod -Uri $ApiUrl -Method Post -Headers $Headers -Body $Body3 -ContentType "application/json"
        Write-Host "  $Length-char name: HTTP 200 (accepted)" -ForegroundColor Green
    } catch {
        $StatusCode = $_.Exception.Response.StatusCode.value__
        Write-Host "  $Length-char name: HTTP $StatusCode (rejected)" -ForegroundColor Red
    }
}

Write-Host ""

# ---------------------------------------------------------------------------
# Test 4: Multiple tools
# ---------------------------------------------------------------------------
Write-Host "[Test 4/4] Multiple tools in a single request..." -ForegroundColor Yellow

$Body4 = @{
    model    = "gpt-4o"
    messages = @(@{ role = "user"; content = "What is the weather in London and read /tmp/test.txt" })
    tools    = @(
        @{
            type     = "function"
            function = @{
                name       = "get_weather"
                description = "Get weather"
                parameters = @{ type = "object"; properties = @{ location = @{ type = "string" } }; required = @("location") }
            }
        },
        @{
            type     = "function"
            function = @{
                name       = "read_file"
                description = "Read a file"
                parameters = @{ type = "object"; properties = @{ path = @{ type = "string" } }; required = @("path") }
            }
        }
    )
    stream   = $false
} | ConvertTo-Json -Depth 10

try {
    $Response4 = Invoke-RestMethod -Uri $ApiUrl -Method Post -Headers $Headers -Body $Body4 -ContentType "application/json"
    Write-Host "  PASS - HTTP 200, multiple tools accepted" -ForegroundColor Green
    $Response4 | ConvertTo-Json -Depth 5 | Write-Host
} catch {
    Write-Host "  FAIL - $($_.Exception.Message)" -ForegroundColor Red
}

Write-Host ""
Write-Host "========================================="
Write-Host " Done"
Write-Host "========================================="
