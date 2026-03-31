use clap::Parser;
use copilot_adapter::cli::{Cli, Command};

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
    assert!(matches!(cli.command, Command::Stop));
}

#[test]
fn parse_status() {
    let cli = Cli::parse_from(["copilot-adapter", "status"]);
    assert!(matches!(cli.command, Command::Status));
}

#[test]
fn parse_auth() {
    let cli = Cli::parse_from(["copilot-adapter", "auth"]);
    match cli.command {
        Command::Auth { force } => assert!(!force),
        _ => panic!("Expected Auth command"),
    }
}

#[test]
fn parse_auth_force() {
    let cli = Cli::parse_from(["copilot-adapter", "auth", "--force"]);
    match cli.command {
        Command::Auth { force } => assert!(force),
        _ => panic!("Expected Auth command"),
    }
}

#[test]
fn parse_logout() {
    let cli = Cli::parse_from(["copilot-adapter", "logout"]);
    assert!(matches!(cli.command, Command::Logout));
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
        Command::Start {
            static_models, ..
        } => assert!(static_models),
        _ => panic!("Expected Start command"),
    }
}

#[test]
fn parse_start_static_models_default_false() {
    let cli = Cli::parse_from(["copilot-adapter", "start"]);
    match cli.command {
        Command::Start {
            static_models, ..
        } => assert!(!static_models),
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
        Command::Start {
            daemon, quiet, ..
        } => {
            assert!(daemon);
            assert!(quiet);
        }
        _ => panic!("Expected Start command"),
    }
}
