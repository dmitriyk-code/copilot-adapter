/*!
Custom Windows Credential implementation with LOCAL_MACHINE persistence.

This module provides a Windows-specific credential storage implementation
that uses CRED_PERSIST_LOCAL_MACHINE for machine-wide credential persistence,
instead of the default CRED_PERSIST_ENTERPRISE used by the keyring crate.
*/

use anyhow::{anyhow, Result};
use keyring::credential::CredentialApi;
use std::iter::once;
use std::mem::MaybeUninit;
use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND, FILETIME};
use windows_sys::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW,
    CRED_FLAGS, CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_MAX_GENERIC_TARGET_NAME_LENGTH,
    CRED_MAX_USERNAME_LENGTH, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
};

/// Windows Generic credential with LOCAL_MACHINE persistence scope.
#[derive(Debug, Clone)]
pub struct LocalMachineCredential {
    target_name: String,
    username: String,
}

impl LocalMachineCredential {
    /// Create a new credential with the given service and username.
    ///
    /// The target name is computed as "username.service" to match the
    /// keyring crate's convention.
    pub fn new(service: &str, username: &str) -> Result<Self> {
        let target_name = format!("{}.{}", username, service);

        // Validate lengths
        if username.len() > CRED_MAX_USERNAME_LENGTH as usize {
            return Err(anyhow!(
                "Username too long: {} > {}",
                username.len(),
                CRED_MAX_USERNAME_LENGTH
            ));
        }
        if target_name.len() > CRED_MAX_GENERIC_TARGET_NAME_LENGTH as usize {
            return Err(anyhow!(
                "Target name too long: {} > {}",
                target_name.len(),
                CRED_MAX_GENERIC_TARGET_NAME_LENGTH
            ));
        }

        Ok(Self {
            target_name,
            username: username.to_string(),
        })
    }
}

impl CredentialApi for LocalMachineCredential {
    fn set_password(&self, password: &str) -> keyring::Result<()> {
        // Convert password to UTF-16 bytes for storage
        let password_utf16: Vec<u16> = password.encode_utf16().collect();
        let mut blob: Vec<u8> = Vec::with_capacity(password_utf16.len() * 2);
        for &val in &password_utf16 {
            blob.push((val & 0xFF) as u8);
            blob.push((val >> 8) as u8);
        }
        self.set_secret(&blob)
    }

    fn get_password(&self) -> keyring::Result<String> {
        let blob = self.get_secret()?;

        // Convert blob to UTF-16 slice
        if blob.len() % 2 != 0 {
            return Err(keyring::Error::BadEncoding(blob));
        }

        let mut blob_u16 = vec![0u16; blob.len() / 2];
        for i in 0..blob_u16.len() {
            blob_u16[i] = u16::from_le_bytes([blob[i * 2], blob[i * 2 + 1]]);
        }

        String::from_utf16(&blob_u16).map_err(|_| keyring::Error::BadEncoding(blob))
    }

    fn set_secret(&self, secret: &[u8]) -> keyring::Result<()> {
        // Check length constraint
        if secret.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize {
            return Err(keyring::Error::TooLong(
                "secret".to_string(),
                CRED_MAX_CREDENTIAL_BLOB_SIZE,
            ));
        }

        // Convert strings to wide strings for Windows API
        let target_name_wide: Vec<u16> = self.target_name.encode_utf16().chain(once(0)).collect();
        let username_wide: Vec<u16> = self.username.encode_utf16().chain(once(0)).collect();

        // Prepare credential structure
        let mut credential = CREDENTIALW {
            Flags: CRED_FLAGS::default(),
            Type: CRED_TYPE_GENERIC,
            TargetName: target_name_wide.as_ptr() as *mut u16,
            Comment: std::ptr::null_mut(),
            LastWritten: FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            },
            CredentialBlobSize: secret.len() as u32,
            CredentialBlob: secret.as_ptr() as *mut u8,
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            AttributeCount: 0,
            Attributes: std::ptr::null_mut(),
            TargetAlias: std::ptr::null_mut(),
            UserName: username_wide.as_ptr() as *mut u16,
        };

        // Call Windows API to write credential
        let result = unsafe { CredWriteW(&mut credential as *mut CREDENTIALW, 0) };

        if result == 0 {
            let error = unsafe { GetLastError() };
            return Err(keyring::Error::PlatformFailure(Box::new(
                std::io::Error::from_raw_os_error(error as i32),
            )));
        }

        Ok(())
    }

    fn get_secret(&self) -> keyring::Result<Vec<u8>> {
        let target_name_wide: Vec<u16> = self.target_name.encode_utf16().chain(once(0)).collect();
        let mut p_credential = MaybeUninit::uninit();

        // Read credential from Windows Credential Manager
        let result = unsafe {
            CredReadW(
                target_name_wide.as_ptr(),
                CRED_TYPE_GENERIC,
                0,
                p_credential.as_mut_ptr(),
            )
        };

        if result == 0 {
            let error = unsafe { GetLastError() };
            if error == ERROR_NOT_FOUND {
                return Err(keyring::Error::NoEntry);
            }
            return Err(keyring::Error::PlatformFailure(Box::new(
                std::io::Error::from_raw_os_error(error as i32),
            )));
        }

        let p_credential = unsafe { p_credential.assume_init() };

        // Extract secret from credential blob
        let secret = unsafe {
            let blob_size = (*p_credential).CredentialBlobSize as usize;
            let blob_ptr = (*p_credential).CredentialBlob;

            let secret = std::slice::from_raw_parts(blob_ptr, blob_size).to_vec();

            CredFree(p_credential as *mut _);
            secret
        };

        Ok(secret)
    }

    fn delete_credential(&self) -> keyring::Result<()> {
        let target_name_wide: Vec<u16> = self.target_name.encode_utf16().chain(once(0)).collect();

        let result = unsafe { CredDeleteW(target_name_wide.as_ptr(), CRED_TYPE_GENERIC, 0) };

        if result == 0 {
            let error = unsafe { GetLastError() };
            if error == ERROR_NOT_FOUND {
                // Credential doesn't exist, which is fine for deletion
                return Ok(());
            }
            return Err(keyring::Error::PlatformFailure(Box::new(
                std::io::Error::from_raw_os_error(error as i32),
            )));
        }

        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
