@echo off
REM Helper script to debug Copilot responses with detailed logging (Windows)

set TIMESTAMP=%date:~10,4%%date:~4,2%%date:~7,2%_%time:~0,2%%time:~3,2%%time:~6,2%
set TIMESTAMP=%TIMESTAMP: =0%
set LOG_FILE=debug_responses_%TIMESTAMP%.log

echo =========================================
echo Copilot Adapter Debug Helper
echo =========================================
echo.
echo This will:
echo   1. Build the adapter in release mode
echo   2. Force re-authentication
echo   3. Start with trace-level logging
echo.
echo Logged to: %LOG_FILE%
echo.

REM Step 1: Build
echo [1/3] Building copilot-adapter (release mode)...
cargo build --release
if %ERRORLEVEL% neq 0 (
    echo ERROR: Build failed!
    exit /b 1
)
echo √ Build successful
echo.

REM Step 2: Force re-authentication
echo [2/3] Forcing re-authentication...
target\release\copilot-adapter.exe auth --force
if %ERRORLEVEL% neq 0 (
    echo ERROR: Authentication failed!
    exit /b 1
)
echo √ Authentication successful
echo.

REM Step 3: Start with logging
echo [3/3] Starting adapter with trace-level logging...
echo.
echo This will capture:
echo   - Model names being requested
echo   - Tools being injected
echo   - Raw response content from Copilot (streaming)
echo   - Full JSON responses (trace level)
echo   - Tool call parsing results
echo.
echo Press Ctrl+C to stop
echo.

REM Start the adapter with trace logging
target\release\copilot-adapter.exe start --experimental-tools --log-level trace --log-file "%LOG_FILE%"
