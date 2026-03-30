// Test program to verify Windows LOCAL_MACHINE persistence
use copilot_adapter::storage::{keyring::KeyringStorage, TokenStorage};

fn main() {
    println!("Creating credential with LOCAL_MACHINE persistence...");

    let storage = KeyringStorage::new().expect("Failed to create keyring storage");

    // Store a token
    storage
        .store_github_token("test_verification_token_123")
        .expect("Failed to store token");

    println!("Token stored successfully!");
    println!("Now checking persistence with PowerShell...");
    println!();
    println!("Run this PowerShell command to check:");
    println!();
    println!("powershell -ExecutionPolicy Bypass -Command \"Add-Type -TypeDefinition @'");
    println!("using System;");
    println!("using System.Runtime.InteropServices;");
    println!("");
    println!("public class CredentialManager {{");
    println!("    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]");
    println!("    public struct CREDENTIAL {{");
    println!("        public int Flags;");
    println!("        public int Type;");
    println!("        public string TargetName;");
    println!("        public string Comment;");
    println!("        public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;");
    println!("        public int CredentialBlobSize;");
    println!("        public IntPtr CredentialBlob;");
    println!("        public int Persist;");
    println!("        public int AttributeCount;");
    println!("        public IntPtr Attributes;");
    println!("        public string TargetAlias;");
    println!("        public string UserName;");
    println!("    }}");
    println!("");
    println!("    [DllImport(\\\"advapi32.dll\\\", CharSet = CharSet.Unicode, SetLastError = true)]");
    println!("    public static extern bool CredRead(string target, int type, int flags, out IntPtr credential);");
    println!("");
    println!("    [DllImport(\\\"advapi32.dll\\\")]");
    println!("    public static extern void CredFree(IntPtr buffer);");
    println!("}}");
    println!("'@; \\$cred = [IntPtr]::Zero; \\$result = [CredentialManager]::CredRead('github_token.copilot-adapter', 1, 0, [ref]\\$cred); if (\\$result) {{ \\$credStruct = [System.Runtime.InteropServices.Marshal]::PtrToStructure(\\$cred, [type][CredentialManager+CREDENTIAL]); Write-Host 'Persistence:' \\$credStruct.Persist '(2=LOCAL_MACHINE)'; [CredentialManager]::CredFree(\\$cred) }} else {{ Write-Host 'Not found' }}\"");
    println!();
    println!("Press Ctrl+C when done, or the credential will be deleted...");

    std::thread::sleep(std::time::Duration::from_secs(30));

    // Clean up
    storage
        .delete_github_token()
        .expect("Failed to delete token");

    println!("Credential deleted.");
}
