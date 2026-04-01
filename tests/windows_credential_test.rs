#[cfg(test)]
#[cfg(target_os = "windows")]
mod windows_credential_tests {
    use copilot_adapter::storage::{keyring::KeyringStorage, TokenStorage};

    #[test]
    fn test_store_and_retrieve_token() {
        let storage = KeyringStorage::new().expect("Failed to create keyring storage");

        // Verify keyring is available
        assert!(
            storage
                .verify_available()
                .expect("Failed to verify keyring"),
            "Keyring should be available on Windows"
        );

        // Store a test token
        let test_token = "ghp_test_token_1234567890abcdef";
        storage
            .store_github_token(test_token)
            .expect("Failed to store token");

        // Retrieve the token
        let retrieved_token = storage
            .get_github_token()
            .expect("Failed to retrieve token");

        assert_eq!(
            retrieved_token, test_token,
            "Retrieved token should match stored token"
        );

        // Clean up
        storage
            .delete_github_token()
            .expect("Failed to delete token");

        // Verify deletion
        assert!(
            storage.get_github_token().is_err(),
            "Token should not exist after deletion"
        );
    }

    #[test]
    fn test_local_machine_persistence() {
        use copilot_adapter::storage::windows_credential::LocalMachineCredential;

        // Create a credential
        let credential = LocalMachineCredential::new("test-service", "test-user")
            .expect("Failed to create credential");

        let entry = keyring::Entry::new_with_credential(Box::new(credential.clone()));

        // Store a test password
        let test_password = "test_password_123";
        entry
            .set_password(test_password)
            .expect("Failed to store password");

        // Retrieve the password
        let retrieved_password = entry.get_password().expect("Failed to retrieve password");

        assert_eq!(
            retrieved_password, test_password,
            "Retrieved password should match stored password"
        );

        // Clean up
        entry
            .delete_credential()
            .expect("Failed to delete credential");
    }
}
