use clap::Parser;
use copilot_adapter::cli::{Cli, Command, ProfilesAction};

#[test]
fn parse_start_defaults() {
    let cli = Cli::parse_from(["copilot-adapter", "start"]);
    match cli.command {
        Command::Start {
            daemon,
            port,
            host,
            log_level,
            log_file,
            models_cache_ttl,
            static_models,
            conversation_log,
            conversation_log_max_size,
            debug_tools,
            skip_auth,
            quiet,
            disable_native_tools,
            profile,
        } => {
            assert!(!daemon);
            assert_eq!(port, 6767);
            assert_eq!(host, "127.0.0.1");
            assert_eq!(log_level, "info");
            assert!(log_file.is_none());
            assert_eq!(models_cache_ttl, 300);
            assert!(!static_models);
            assert!(conversation_log.is_none());
            assert_eq!(conversation_log_max_size, 10_485_760);
            assert!(!debug_tools);
            assert!(!skip_auth);
            assert!(!quiet);
            assert!(!disable_native_tools);
            assert_eq!(profile, "default");
        }
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_custom_port() {
    let cli = Cli::parse_from(["copilot-adapter", "start", "--port", "9090"]);
    match cli.command {
        Command::Start { port, .. } => assert_eq!(port, 9090),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_short_port() {
    let cli = Cli::parse_from(["copilot-adapter", "start", "-p", "3000"]);
    match cli.command {
        Command::Start { port, .. } => assert_eq!(port, 3000),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_custom_host() {
    let cli = Cli::parse_from(["copilot-adapter", "start", "--host", "0.0.0.0"]);
    match cli.command {
        Command::Start { host, .. } => assert_eq!(host, "0.0.0.0"),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_daemon_flag() {
    let cli = Cli::parse_from(["copilot-adapter", "start", "--daemon"]);
    match cli.command {
        Command::Start { daemon, .. } => assert!(daemon),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_daemon_short_flag() {
    let cli = Cli::parse_from(["copilot-adapter", "start", "-d"]);
    match cli.command {
        Command::Start { daemon, .. } => assert!(daemon),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_log_level() {
    let cli = Cli::parse_from(["copilot-adapter", "start", "--log-level", "debug"]);
    match cli.command {
        Command::Start { log_level, .. } => assert_eq!(log_level, "debug"),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_log_file() {
    let cli = Cli::parse_from(["copilot-adapter", "start", "--log-file", "/tmp/adapter.log"]);
    match cli.command {
        Command::Start { log_file, .. } => {
            assert_eq!(log_file, Some("/tmp/adapter.log".to_string()))
        }
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_all_flags() {
    let cli = Cli::parse_from([
        "copilot-adapter",
        "start",
        "-d",
        "-p",
        "9000",
        "--host",
        "0.0.0.0",
        "--log-level",
        "trace",
        "--log-file",
        "/var/log/adapter.log",
    ]);
    match cli.command {
        Command::Start {
            daemon,
            port,
            host,
            log_level,
            log_file,
            models_cache_ttl,
            static_models,
            ..
        } => {
            assert!(daemon);
            assert_eq!(port, 9000);
            assert_eq!(host, "0.0.0.0");
            assert_eq!(log_level, "trace");
            assert_eq!(log_file, Some("/var/log/adapter.log".to_string()));
            assert_eq!(models_cache_ttl, 300);
            assert!(!static_models);
        }
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_stop() {
    let cli = Cli::parse_from(["copilot-adapter", "stop"]);
    match cli.command {
        Command::Stop { profile, all } => {
            assert_eq!(profile, "default");
            assert!(!all);
        }
        _ => panic!("Expected Stop command"),
    }
}

#[test]
fn parse_status() {
    let cli = Cli::parse_from(["copilot-adapter", "status"]);
    match cli.command {
        Command::Status { profile, all } => {
            assert_eq!(profile, "default");
            assert!(!all);
        }
        _ => panic!("Expected Status command"),
    }
}

#[test]
fn parse_auth() {
    let cli = Cli::parse_from(["copilot-adapter", "auth"]);
    match cli.command {
        Command::Auth { force, profile } => {
            assert!(!force);
            assert_eq!(profile, "default");
        }
        _ => panic!("Expected Auth command"),
    }
}

#[test]
fn parse_auth_force() {
    let cli = Cli::parse_from(["copilot-adapter", "auth", "--force"]);
    match cli.command {
        Command::Auth { force, profile } => {
            assert!(force);
            assert_eq!(profile, "default");
        }
        _ => panic!("Expected Auth command"),
    }
}

#[test]
fn parse_logout() {
    let cli = Cli::parse_from(["copilot-adapter", "logout"]);
    match cli.command {
        Command::Logout { profile } => {
            assert_eq!(profile, "default");
        }
        _ => panic!("Expected Logout command"),
    }
}

#[test]
fn parse_no_command_fails() {
    let result = Cli::try_parse_from(["copilot-adapter"]);
    assert!(result.is_err());
}

#[test]
fn parse_unknown_command_fails() {
    let result = Cli::try_parse_from(["copilot-adapter", "unknown"]);
    assert!(result.is_err());
}

#[test]
fn parse_start_models_cache_ttl_default() {
    let cli = Cli::parse_from(["copilot-adapter", "start"]);
    match cli.command {
        Command::Start {
            models_cache_ttl, ..
        } => assert_eq!(models_cache_ttl, 300),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_models_cache_ttl_custom() {
    let cli = Cli::parse_from(["copilot-adapter", "start", "--models-cache-ttl", "60"]);
    match cli.command {
        Command::Start {
            models_cache_ttl, ..
        } => assert_eq!(models_cache_ttl, 60),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_models_cache_ttl_zero() {
    let cli = Cli::parse_from(["copilot-adapter", "start", "--models-cache-ttl", "0"]);
    match cli.command {
        Command::Start {
            models_cache_ttl, ..
        } => assert_eq!(models_cache_ttl, 0),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_static_models_flag() {
    let cli = Cli::parse_from(["copilot-adapter", "start", "--static-models"]);
    match cli.command {
        Command::Start { static_models, .. } => assert!(static_models),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_static_models_default_false() {
    let cli = Cli::parse_from(["copilot-adapter", "start"]);
    match cli.command {
        Command::Start { static_models, .. } => assert!(!static_models),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_all_model_flags() {
    let cli = Cli::parse_from([
        "copilot-adapter",
        "start",
        "--models-cache-ttl",
        "120",
        "--static-models",
    ]);
    match cli.command {
        Command::Start {
            models_cache_ttl,
            static_models,
            ..
        } => {
            assert_eq!(models_cache_ttl, 120);
            assert!(static_models);
        }
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_conversation_log() {
    let cli = Cli::parse_from([
        "copilot-adapter",
        "start",
        "--conversation-log",
        "/tmp/conv.log",
    ]);
    match cli.command {
        Command::Start {
            conversation_log, ..
        } => assert_eq!(conversation_log, Some("/tmp/conv.log".to_string())),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_conversation_log_max_size() {
    let cli = Cli::parse_from([
        "copilot-adapter",
        "start",
        "--conversation-log-max-size",
        "5000000",
    ]);
    match cli.command {
        Command::Start {
            conversation_log_max_size,
            ..
        } => assert_eq!(conversation_log_max_size, 5_000_000),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_debug_tools_flag() {
    let cli = Cli::parse_from(["copilot-adapter", "start", "--debug-tools"]);
    match cli.command {
        Command::Start { debug_tools, .. } => assert!(debug_tools),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_debug_tools_default_false() {
    let cli = Cli::parse_from(["copilot-adapter", "start"]);
    match cli.command {
        Command::Start { debug_tools, .. } => assert!(!debug_tools),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_skip_auth_flag() {
    let cli = Cli::parse_from(["copilot-adapter", "start", "--skip-auth"]);
    match cli.command {
        Command::Start { skip_auth, .. } => assert!(skip_auth),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_skip_auth_default_false() {
    let cli = Cli::parse_from(["copilot-adapter", "start"]);
    match cli.command {
        Command::Start { skip_auth, .. } => assert!(!skip_auth),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_skip_auth_with_daemon() {
    let cli = Cli::parse_from(["copilot-adapter", "start", "--daemon", "--skip-auth"]);
    match cli.command {
        Command::Start {
            daemon, skip_auth, ..
        } => {
            assert!(daemon);
            assert!(skip_auth);
        }
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_quiet_flag() {
    let cli = Cli::parse_from(["copilot-adapter", "start", "--quiet"]);
    match cli.command {
        Command::Start { quiet, .. } => assert!(quiet),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_quiet_short_flag() {
    let cli = Cli::parse_from(["copilot-adapter", "start", "-q"]);
    match cli.command {
        Command::Start { quiet, .. } => assert!(quiet),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_quiet_default_false() {
    let cli = Cli::parse_from(["copilot-adapter", "start"]);
    match cli.command {
        Command::Start { quiet, .. } => assert!(!quiet),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_quiet_with_daemon() {
    let cli = Cli::parse_from(["copilot-adapter", "start", "--daemon", "--quiet"]);
    match cli.command {
        Command::Start { daemon, quiet, .. } => {
            assert!(daemon);
            assert!(quiet);
        }
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_native_tools_default_true() {
    let cli = Cli::parse_from(["copilot-adapter", "start"]);
    match cli.command {
        Command::Start { disable_native_tools, .. } => assert!(!disable_native_tools),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_disable_native_tools_flag() {
    let cli = Cli::parse_from(["copilot-adapter", "start", "--disable-native-tools"]);
    match cli.command {
        Command::Start {
            disable_native_tools,
            ..
        } => {
            assert!(disable_native_tools);
        }
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_disable_native_tools_default_false() {
    let cli = Cli::parse_from(["copilot-adapter", "start"]);
    match cli.command {
        Command::Start { disable_native_tools, .. } => assert!(!disable_native_tools),
        _ => panic!("Expected Start command"),
    }
}

// --- Epic 4: use_keyring flag removed ---
// The --use-keyring flag has been removed. Verify it is no longer accepted.

#[test]
fn parse_start_use_keyring_flag_rejected() {
    let result = Cli::try_parse_from(["copilot-adapter", "start", "--use-keyring"]);
    assert!(result.is_err(), "--use-keyring should no longer be accepted for start");
}

#[test]
fn parse_auth_use_keyring_flag_rejected() {
    let result = Cli::try_parse_from(["copilot-adapter", "auth", "--use-keyring"]);
    assert!(result.is_err(), "--use-keyring should no longer be accepted for auth");
}

// --- Epic 6: --profile, --all, and profiles subcommand tests ---

#[test]
fn parse_start_profile_default() {
    let cli = Cli::parse_from(["copilot-adapter", "start"]);
    match cli.command {
        Command::Start { profile, .. } => assert_eq!(profile, "default"),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_profile_custom() {
    let cli = Cli::parse_from(["copilot-adapter", "start", "--profile", "work"]);
    match cli.command {
        Command::Start { profile, .. } => assert_eq!(profile, "work"),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_profile_short_flag() {
    let cli = Cli::parse_from(["copilot-adapter", "start", "-P", "staging"]);
    match cli.command {
        Command::Start { profile, .. } => assert_eq!(profile, "staging"),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_profile_with_port() {
    let cli = Cli::parse_from([
        "copilot-adapter",
        "start",
        "-P",
        "work",
        "-p",
        "9090",
    ]);
    match cli.command {
        Command::Start { profile, port, .. } => {
            assert_eq!(profile, "work");
            assert_eq!(port, 9090);
        }
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_stop_profile_default() {
    let cli = Cli::parse_from(["copilot-adapter", "stop"]);
    match cli.command {
        Command::Stop { profile, all } => {
            assert_eq!(profile, "default");
            assert!(!all);
        }
        _ => panic!("Expected Stop command"),
    }
}

#[test]
fn parse_stop_profile_custom() {
    let cli = Cli::parse_from(["copilot-adapter", "stop", "--profile", "work"]);
    match cli.command {
        Command::Stop { profile, all } => {
            assert_eq!(profile, "work");
            assert!(!all);
        }
        _ => panic!("Expected Stop command"),
    }
}

#[test]
fn parse_stop_all() {
    let cli = Cli::parse_from(["copilot-adapter", "stop", "--all"]);
    match cli.command {
        Command::Stop { all, .. } => assert!(all),
        _ => panic!("Expected Stop command"),
    }
}

#[test]
fn parse_stop_profile_short_flag() {
    let cli = Cli::parse_from(["copilot-adapter", "stop", "-P", "dev"]);
    match cli.command {
        Command::Stop { profile, .. } => assert_eq!(profile, "dev"),
        _ => panic!("Expected Stop command"),
    }
}

#[test]
fn parse_status_profile_default() {
    let cli = Cli::parse_from(["copilot-adapter", "status"]);
    match cli.command {
        Command::Status { profile, all } => {
            assert_eq!(profile, "default");
            assert!(!all);
        }
        _ => panic!("Expected Status command"),
    }
}

#[test]
fn parse_status_profile_custom() {
    let cli = Cli::parse_from(["copilot-adapter", "status", "--profile", "work"]);
    match cli.command {
        Command::Status { profile, all } => {
            assert_eq!(profile, "work");
            assert!(!all);
        }
        _ => panic!("Expected Status command"),
    }
}

#[test]
fn parse_status_all() {
    let cli = Cli::parse_from(["copilot-adapter", "status", "--all"]);
    match cli.command {
        Command::Status { all, .. } => assert!(all),
        _ => panic!("Expected Status command"),
    }
}

#[test]
fn parse_status_profile_short_flag() {
    let cli = Cli::parse_from(["copilot-adapter", "status", "-P", "staging"]);
    match cli.command {
        Command::Status { profile, .. } => assert_eq!(profile, "staging"),
        _ => panic!("Expected Status command"),
    }
}

#[test]
fn parse_auth_profile_default() {
    let cli = Cli::parse_from(["copilot-adapter", "auth"]);
    match cli.command {
        Command::Auth { profile, .. } => assert_eq!(profile, "default"),
        _ => panic!("Expected Auth command"),
    }
}

#[test]
fn parse_auth_profile_custom() {
    let cli = Cli::parse_from(["copilot-adapter", "auth", "--profile", "work"]);
    match cli.command {
        Command::Auth { profile, .. } => assert_eq!(profile, "work"),
        _ => panic!("Expected Auth command"),
    }
}

#[test]
fn parse_auth_profile_short_flag() {
    let cli = Cli::parse_from(["copilot-adapter", "auth", "-P", "dev"]);
    match cli.command {
        Command::Auth { profile, .. } => assert_eq!(profile, "dev"),
        _ => panic!("Expected Auth command"),
    }
}

#[test]
fn parse_logout_profile_default() {
    let cli = Cli::parse_from(["copilot-adapter", "logout"]);
    match cli.command {
        Command::Logout { profile } => assert_eq!(profile, "default"),
        _ => panic!("Expected Logout command"),
    }
}

#[test]
fn parse_logout_profile_custom() {
    let cli = Cli::parse_from(["copilot-adapter", "logout", "--profile", "work"]);
    match cli.command {
        Command::Logout { profile } => assert_eq!(profile, "work"),
        _ => panic!("Expected Logout command"),
    }
}

#[test]
fn parse_logout_profile_short_flag() {
    let cli = Cli::parse_from(["copilot-adapter", "logout", "-P", "staging"]);
    match cli.command {
        Command::Logout { profile } => assert_eq!(profile, "staging"),
        _ => panic!("Expected Logout command"),
    }
}

#[test]
fn parse_profiles_list() {
    let cli = Cli::parse_from(["copilot-adapter", "profiles", "list"]);
    match cli.command {
        Command::Profiles { action: ProfilesAction::List } => {}
        _ => panic!("Expected Profiles List"),
    }
}

#[test]
fn parse_profiles_create() {
    let cli = Cli::parse_from(["copilot-adapter", "profiles", "create", "staging"]);
    match cli.command {
        Command::Profiles { action: ProfilesAction::Create { name } } => {
            assert_eq!(name, "staging");
        }
        _ => panic!("Expected Profiles Create"),
    }
}

#[test]
fn parse_profiles_delete() {
    let cli = Cli::parse_from(["copilot-adapter", "profiles", "delete", "old-profile"]);
    match cli.command {
        Command::Profiles { action: ProfilesAction::Delete { name } } => {
            assert_eq!(name, "old-profile");
        }
        _ => panic!("Expected Profiles Delete"),
    }
}

#[test]
fn parse_profiles_no_subcommand_fails() {
    let result = Cli::try_parse_from(["copilot-adapter", "profiles"]);
    assert!(result.is_err());
}

#[test]
fn parse_profiles_create_no_name_fails() {
    let result = Cli::try_parse_from(["copilot-adapter", "profiles", "create"]);
    assert!(result.is_err());
}

#[test]
fn parse_profiles_delete_no_name_fails() {
    let result = Cli::try_parse_from(["copilot-adapter", "profiles", "delete"]);
    assert!(result.is_err());
}
