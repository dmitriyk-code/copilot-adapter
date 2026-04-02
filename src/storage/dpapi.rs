/*!
Windows DPAPI encryption/decryption module.

Provides [`encrypt`] and [`decrypt`] functions that wrap the Windows Data
Protection API (DPAPI). The encryption key is derived from the current
user's Windows credentials — only the same user on the same machine can
decrypt the data.

DPAPI has been available since Windows 2000, so no runtime availability
check is needed.
*/

#[cfg(target_os = "windows")]
use anyhow::{anyhow, Result};
#[cfg(target_os = "windows")]
use windows_sys::Win32::Foundation::LocalFree;
#[cfg(target_os = "windows")]
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
};

/// Encrypt data using Windows DPAPI.
///
/// The encryption key is derived from the current user's Windows credentials.
/// Only the same user on the same machine can decrypt.
#[cfg(target_os = "windows")]
pub fn encrypt(plaintext: &[u8]) -> Result<Vec<u8>> {
    unsafe {
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: plaintext.len() as u32,
            pbData: plaintext.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };

        let result = CryptProtectData(
            &mut input,
            std::ptr::null(),     // no description
            std::ptr::null_mut(), // no entropy
            std::ptr::null_mut(), // reserved
            std::ptr::null_mut(), // no prompt
            0,                    // current-user scope
            &mut output,
        );

        if result == 0 {
            return Err(anyhow!(
                "CryptProtectData failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        let encrypted =
            std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as *mut _);
        Ok(encrypted)
    }
}

/// Decrypt data using Windows DPAPI.
///
/// Returns an error if the data was not encrypted by the current user on
/// this machine, or if the ciphertext is malformed.
#[cfg(target_os = "windows")]
pub fn decrypt(encrypted: &[u8]) -> Result<Vec<u8>> {
    unsafe {
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: encrypted.len() as u32,
            pbData: encrypted.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };

        let result = CryptUnprotectData(
            &mut input,
            std::ptr::null_mut(), // description (out)
            std::ptr::null_mut(), // entropy (optional)
            std::ptr::null_mut(), // reserved
            std::ptr::null_mut(), // prompt struct
            0,                    // flags
            &mut output,
        );

        if result == 0 {
            return Err(anyhow!(
                "CryptUnprotectData failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        let decrypted =
            std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as *mut _);
        Ok(decrypted)
    }
}

#[cfg(test)]
#[cfg(target_os = "windows")]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_round_trip() {
        let plaintext = b"test-token-ghp_123456789";
        let encrypted = encrypt(plaintext).expect("encrypt failed");
        assert_ne!(encrypted, plaintext, "encrypted should differ from plaintext");

        let decrypted = decrypt(&encrypted).expect("decrypt failed");
        assert_eq!(decrypted, plaintext, "decrypted should match original");
    }

    #[test]
    fn test_empty_string() {
        let plaintext = b"";
        let encrypted = encrypt(plaintext).expect("encrypt failed");
        let decrypted = decrypt(&encrypted).expect("decrypt failed");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_large_payload() {
        let plaintext: Vec<u8> = (0..10_000).map(|i| (i % 256) as u8).collect();
        let encrypted = encrypt(&plaintext).expect("encrypt failed");
        let decrypted = decrypt(&encrypted).expect("decrypt failed");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_decrypt_invalid_data_fails() {
        let garbage = b"not-encrypted-data";
        let result = decrypt(garbage);
        assert!(result.is_err(), "decrypting garbage should fail");
    }

    #[test]
    fn test_unicode_content() {
        let plaintext = "ghp_tëst_tökéñ_🔑".as_bytes();
        let encrypted = encrypt(plaintext).expect("encrypt failed");
        let decrypted = decrypt(&encrypted).expect("decrypt failed");
        assert_eq!(decrypted, plaintext);
    }
}
