@echo off
REM Helper script to debug Copilot responses with detailed logging (Windows)

set TIMESTAMP=%date:~10,4%%date:~4,2%%date:~7,2%_%time:~0,2%%time:~3,2%%time:~6,2%
set TIMESTAMP=%TIMESTAMP: =0%
set LOG_FILE=debug_responses_%TIMESTAMP%.log

echo Starting copilot-adapter with trace-level logging...
echo Log file: %LOG_FILE%
echo.
echo This will capture:
echo   - Model names being requested
echo   - Tools being injected
echo   - Raw response content from Copilot
echo   - Full JSON responses (trace level)
echo   - Tool call parsing results
echo.
echo Press Ctrl+C to stop
echo.

REM Start the adapter with trace logging
cargo run --release -- start --experimental-tools --log-level trace --log-file "%LOG_FILE%"
