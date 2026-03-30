#!/bin/bash
# Test script to verify LOCAL_MACHINE persistence

echo "Building and running test..."
cargo run --example test_persistence &
PID=$!

# Wait a bit for credential to be created
sleep 2

# Check the credential persistence using PowerShell
echo ""
echo "Checking credential persistence..."
powershell -ExecutionPolicy Bypass -Command '
$code = @"
using System;
using System.Runtime.InteropServices;

public class CredManager {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct CREDENTIAL {
        public int Flags;
        public int Type;
        public IntPtr TargetName;
        public IntPtr Comment;
        public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
        public int CredentialBlobSize;
        public IntPtr CredentialBlob;
        public int Persist;
        public int AttributeCount;
        public IntPtr Attributes;
        public IntPtr TargetAlias;
        public IntPtr UserName;
    }

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern bool CredRead(string target, int type, int flags, out IntPtr credential);

    [DllImport("advapi32.dll")]
    public static extern void CredFree(IntPtr buffer);
}
"@

Add-Type -TypeDefinition $code

$target = "github_token.copilot-adapter"
$credPtr = [IntPtr]::Zero
$result = [CredManager]::CredRead($target, 1, 0, [ref]$credPtr)

if ($result) {
    $cred = [System.Runtime.InteropServices.Marshal]::PtrToStructure($credPtr, [type][CredManager+CREDENTIAL])
    $persist = $cred.Persist
    Write-Host "✓ Credential found!"
    Write-Host "  Persistence value: $persist"
    if ($persist -eq 2) {
        Write-Host "  ✓ SUCCESS: Using LOCAL_MACHINE persistence (2)"
    } elseif ($persist -eq 3) {
        Write-Host "  ✗ FAIL: Using ENTERPRISE persistence (3)"
    } elseif ($persist -eq 1) {
        Write-Host "  ✗ FAIL: Using SESSION persistence (1)"
    } else {
        Write-Host "  ? Unknown persistence value: $persist"
    }
    [CredManager]::CredFree($credPtr)
} else {
    Write-Host "✗ Credential not found: $target"
}
'

# Kill the test program
kill $PID 2>/dev/null
wait $PID 2>/dev/null

echo ""
echo "Test complete."
