use copilot_adapter::daemon::status::write_status_to;
use copilot_adapter::profile::{validate_profile_name, ProfileManager};
use std::fs;

/// Helper: create a unique temp directory for each test to avoid interference.
fn test_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "copilot-adapter-profile-test-{}-{}",
        name,
        std::process::id()
    ));
    // Clean up any leftover from a prior run
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::create_dir_all(&dir);
    dir
}

// ============================================================
// Name validation tests
// ============================================================

#[test]
fn validate_name_accepts_default() {
    assert!(validate_profile_name("default").is_ok());
}

#[test]
fn validate_name_accepts_simple_word() {
    assert!(validate_profile_name("work").is_ok());
}

#[test]
fn validate_name_accepts_dashes() {
    assert!(validate_profile_name("my-profile").is_ok());
}

#[test]
fn validate_name_accepts_underscores() {
    assert!(validate_profile_name("profile_2").is_ok());
}

#[test]
fn validate_name_accepts_single_char() {
    assert!(validate_profile_name("a").is_ok());
}

#[test]
fn validate_name_accepts_64_char_name() {
    let name = "a".repeat(64);
    assert!(validate_profile_name(&name).is_ok());
}

#[test]
fn validate_name_rejects_empty() {
    assert!(validate_profile_name("").is_err());
}

#[test]
fn validate_name_rejects_65_chars() {
    let name = "a".repeat(65);
    assert!(validate_profile_name(&name).is_err());
}

#[test]
fn validate_name_rejects_spaces() {
    assert!(validate_profile_name("my profile").is_err());
}

#[test]
fn validate_name_rejects_dots() {
    assert!(validate_profile_name("my.profile").is_err());
}

#[test]
fn validate_name_rejects_slashes() {
    assert!(validate_profile_name("my/profile").is_err());
}

#[test]
fn validate_name_rejects_unicode_letters() {
    // Unicode alphanumeric chars should be rejected (ASCII-only)
    assert!(validate_profile_name("café").is_err());
    assert!(validate_profile_name("用户").is_err());
    assert!(validate_profile_name("naïve").is_err());
}

// ============================================================
// ProfileManager CRUD tests
// ============================================================

#[test]
fn create_profile_creates_directory_and_returns_profile() {
    let base = test_dir("create");
    let mgr = ProfileManager::with_base_dir(base.clone());

    let profile = mgr.create("foo").unwrap();
    assert_eq!(profile.name, "foo");
    assert!(profile.dir.exists());
    assert!(profile.dir.is_dir());
    assert_eq!(
        profile.status_path(),
        base.join("profiles").join("foo").join("status.json")
    );
    assert_eq!(
        profile.credentials_path(),
        base.join("profiles").join("foo").join("credentials.json")
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn get_profile_after_create_returns_same_profile() {
    let base = test_dir("get-after-create");
    let mgr = ProfileManager::with_base_dir(base.clone());

    mgr.create("foo").unwrap();
    let profile = mgr.get("foo").unwrap();
    assert_eq!(profile.name, "foo");
    assert_eq!(profile.dir, base.join("profiles").join("foo"));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn list_profiles_returns_all_created() {
    let base = test_dir("list");
    let mgr = ProfileManager::with_base_dir(base.clone());

    mgr.create("alpha").unwrap();
    mgr.create("beta").unwrap();

    let mut profiles: Vec<String> = mgr.list().into_iter().map(|p| p.name).collect();
    profiles.sort();
    assert_eq!(profiles, vec!["alpha", "beta"]);

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn delete_removes_directory() {
    let base = test_dir("delete");
    let mgr = ProfileManager::with_base_dir(base.clone());

    let profile = mgr.create("foo").unwrap();
    assert!(profile.dir.exists());

    mgr.delete("foo").unwrap();
    assert!(!profile.dir.exists());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn delete_default_returns_error() {
    let base = test_dir("delete-default");
    let mgr = ProfileManager::with_base_dir(base.clone());

    // Ensure default exists
    mgr.get("default").unwrap();

    let result = mgr.delete("default");
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Cannot delete the default profile"));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn create_duplicate_returns_error() {
    let base = test_dir("create-dup");
    let mgr = ProfileManager::with_base_dir(base.clone());

    mgr.create("foo").unwrap();
    let result = mgr.create("foo");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("already exists"));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn get_nonexistent_profile_returns_error() {
    let base = test_dir("get-nonexistent");
    let mgr = ProfileManager::with_base_dir(base.clone());

    let result = mgr.get("nonexistent");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("does not exist"));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn delete_nonexistent_profile_returns_error() {
    let base = test_dir("delete-nonexistent");
    let mgr = ProfileManager::with_base_dir(base.clone());

    let result = mgr.delete("nonexistent");
    assert!(result.is_err());

    let _ = fs::remove_dir_all(&base);
}

// ============================================================
// Port conflict detection tests
// ============================================================

#[test]
fn check_port_conflict_ok_when_no_status_files() {
    let base = test_dir("port-no-status");
    let mgr = ProfileManager::with_base_dir(base.clone());

    // Create default profile but no status.json
    mgr.get("default").unwrap();

    assert!(mgr.check_port_conflict(6767, "default").is_ok());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn check_port_conflict_err_when_another_profile_uses_port() {
    let base = test_dir("port-conflict");
    let mgr = ProfileManager::with_base_dir(base.clone());

    // Create "default" with a status.json on port 6767
    let default_profile = mgr.get("default").unwrap();
    write_status_to(&default_profile.status_path(), 6767).unwrap();

    // Create "work" profile
    mgr.create("work").unwrap();

    // Checking port 6767 for "work" should fail because "default" has it
    let result = mgr.check_port_conflict(6767, "work");
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("6767"));
    assert!(err_msg.contains("default"));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn check_port_conflict_ok_for_same_profile() {
    let base = test_dir("port-same");
    let mgr = ProfileManager::with_base_dir(base.clone());

    // Create "default" with a status.json on port 6767
    let default_profile = mgr.get("default").unwrap();
    write_status_to(&default_profile.status_path(), 6767).unwrap();

    // Same profile using same port is fine
    assert!(mgr.check_port_conflict(6767, "default").is_ok());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn check_port_conflict_skips_stale_status_file() {
    use copilot_adapter::daemon::status::StatusFile;

    let base = test_dir("port-stale");
    let mgr = ProfileManager::with_base_dir(base.clone());

    // Create "default" with a status.json claiming port 6767 but with a dead PID
    let default_profile = mgr.get("default").unwrap();
    let stale_status = StatusFile {
        pid: 99999999, // bogus PID that doesn't exist
        port: 6767,
        started_at: None,
        version: None,
    };
    fs::write(
        &default_profile.status_path(),
        serde_json::to_string_pretty(&stale_status).unwrap(),
    )
    .unwrap();

    // Create "work" profile
    mgr.create("work").unwrap();

    // Should succeed because the process behind "default" is dead (stale)
    assert!(mgr.check_port_conflict(6767, "work").is_ok());

    // The stale status file should have been cleaned up
    assert!(!default_profile.status_path().exists());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn find_by_port_returns_matching_profile() {
    let base = test_dir("find-port");
    let mgr = ProfileManager::with_base_dir(base.clone());

    let profile = mgr.create("web").unwrap();
    write_status_to(&profile.status_path(), 8080).unwrap();

    let found = mgr.find_by_port(8080);
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "web");

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn find_by_port_returns_none_when_no_match() {
    let base = test_dir("find-port-none");
    let mgr = ProfileManager::with_base_dir(base.clone());

    mgr.create("web").unwrap();
    // No status.json written

    let found = mgr.find_by_port(8080);
    assert!(found.is_none());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn find_by_port_skips_stale_status_and_cleans_up() {
    use copilot_adapter::daemon::status::StatusFile;

    let base = test_dir("find-port-stale");
    let mgr = ProfileManager::with_base_dir(base.clone());

    let profile = mgr.create("stale-web").unwrap();
    // Write a status file with a dead PID
    let stale_status = StatusFile {
        pid: 99999999,
        port: 8080,
        started_at: None,
        version: None,
    };
    fs::write(
        &profile.status_path(),
        serde_json::to_string_pretty(&stale_status).unwrap(),
    )
    .unwrap();

    // Should return None because the process is dead
    let found = mgr.find_by_port(8080);
    assert!(found.is_none());

    // The stale status file should have been cleaned up
    assert!(!profile.status_path().exists());

    let _ = fs::remove_dir_all(&base);
}

// ============================================================
// Default profile behavior tests
// ============================================================

#[test]
fn get_default_creates_directory_if_missing() {
    let base = test_dir("default-autocreate");
    let mgr = ProfileManager::with_base_dir(base.clone());

    // profiles/default/ should not exist yet
    let expected_dir = base.join("profiles").join("default");
    assert!(!expected_dir.exists());

    let profile = mgr.get("default").unwrap();
    assert_eq!(profile.name, "default");
    assert!(expected_dir.exists());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn profile_status_path_is_correct() {
    let base = test_dir("status-path");
    let mgr = ProfileManager::with_base_dir(base.clone());

    let profile = mgr.get("default").unwrap();
    assert!(profile.status_path().ends_with("status.json"));
    assert_eq!(
        profile.status_path(),
        base.join("profiles").join("default").join("status.json")
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn profile_credentials_path_is_correct() {
    let base = test_dir("creds-path");
    let mgr = ProfileManager::with_base_dir(base.clone());

    let profile = mgr.get("default").unwrap();
    assert!(profile.credentials_path().ends_with("credentials.json"));
    assert_eq!(
        profile.credentials_path(),
        base.join("profiles")
            .join("default")
            .join("credentials.json")
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn list_returns_empty_when_no_profiles_dir() {
    let base = test_dir("list-empty");
    let mgr = ProfileManager::with_base_dir(base.clone());

    let profiles = mgr.list();
    assert!(profiles.is_empty());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn create_default_returns_error() {
    let base = test_dir("create-default");
    let mgr = ProfileManager::with_base_dir(base.clone());

    let result = mgr.create("default");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("default"));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn list_skips_directories_with_invalid_names() {
    let base = test_dir("list-invalid");
    let mgr = ProfileManager::with_base_dir(base.clone());

    // Create a valid profile
    mgr.create("valid").unwrap();

    // Manually create directories with invalid names
    let profiles_dir = base.join("profiles");
    fs::create_dir_all(profiles_dir.join("has.dot")).unwrap();
    fs::create_dir_all(profiles_dir.join("has space")).unwrap();

    let profiles: Vec<String> = mgr.list().into_iter().map(|p| p.name).collect();
    assert_eq!(profiles, vec!["valid"]);

    let _ = fs::remove_dir_all(&base);
}
