//! CLI login commands for xAI (Grok / SuperGrok).
//!
//! Supports browser OAuth (`codex login`) and API key login via stdin
//! (`printenv XAI_API_KEY | codex login --with-api-key`).

use codex_config::types::AuthCredentialsStoreMode;
use codex_core::config::Config;
use codex_login::AuthKeyringBackendKind;
use codex_login::AuthRouteConfig;
use codex_login::CodexAuth;
use codex_login::XAI_API_KEY_ENV_VAR;
use codex_login::XaiLoginServerOptions;
use codex_login::login_with_api_key;
use codex_login::logout_with_revoke;
use codex_login::run_xai_login_server;
use codex_protocol::auth::AuthMode;
use codex_utils_cli::CliConfigOverrides;
use std::fs::OpenOptions;
use std::io::IsTerminal;
use std::io::Read;
use std::path::Path;
use tracing_appender::non_blocking;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

const LOGIN_SUCCESS_MESSAGE: &str = "Successfully logged in";

fn init_login_file_logging(config: &Config) -> Option<WorkerGuard> {
    let log_dir = match codex_core::config::log_dir(config) {
        Ok(log_dir) => log_dir,
        Err(err) => {
            eprintln!("Warning: failed to resolve login log directory: {err}");
            return None;
        }
    };

    if let Err(err) = std::fs::create_dir_all(&log_dir) {
        eprintln!(
            "Warning: failed to create login log directory {}: {err}",
            log_dir.display()
        );
        return None;
    }

    let mut log_file_opts = OpenOptions::new();
    log_file_opts.create(true).append(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        log_file_opts.mode(0o600);
    }

    let log_path = log_dir.join("codex-login.log");
    let log_file = match log_file_opts.open(&log_path) {
        Ok(log_file) => log_file,
        Err(err) => {
            eprintln!(
                "Warning: failed to open login log file {}: {err}",
                log_path.display()
            );
            return None;
        }
    };

    let (non_blocking, guard) = non_blocking(log_file);
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("codex_cli=info,codex_core=info,codex_login=info"));
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_target(true)
        .with_ansi(false)
        .with_filter(env_filter);

    if let Err(err) = tracing_subscriber::registry().with(file_layer).try_init() {
        eprintln!(
            "Warning: failed to initialize login log file {}: {err}",
            log_path.display()
        );
        return None;
    }

    Some(guard)
}

async fn clear_existing_auth_before_login(
    codex_home: &Path,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
    auth_keyring_backend_kind: AuthKeyringBackendKind,
    auth_route_config: Option<&AuthRouteConfig>,
) {
    if let Err(err) = logout_with_revoke(
        codex_home,
        auth_credentials_store_mode,
        auth_keyring_backend_kind,
        auth_route_config,
    )
    .await
    {
        tracing::warn!("failed to clear existing auth before login: {err}");
    }
}

/// Log in with xAI (Grok / SuperGrok) OAuth via browser loopback flow.
pub async fn run_login_with_xai(cli_config_overrides: CliConfigOverrides) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;
    let _login_log_guard = init_login_file_logging(&config);
    tracing::info!("starting xAI (Grok / SuperGrok) login flow");

    clear_existing_auth_before_login(
        &config.codex_home,
        config.cli_auth_credentials_store_mode,
        config.auth_keyring_backend_kind(),
        config.auth_route_config().as_ref(),
    )
    .await;

    let opts = XaiLoginServerOptions::new(
        config.codex_home.to_path_buf(),
        config.cli_auth_credentials_store_mode,
        config.auth_keyring_backend_kind(),
        config.auth_route_config(),
    );
    match run_xai_login_server(opts) {
        Ok(server) => {
            eprintln!(
                "Starting xAI login server on http://localhost:{}.\nIf your browser did not open, navigate to this URL to authenticate:\n\n{}\n",
                server.actual_port, server.auth_url
            );
            match server.block_until_done().await {
                Ok(()) => {
                    eprintln!("{LOGIN_SUCCESS_MESSAGE}");
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Error logging in to xAI: {e}");
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("Error starting xAI login server: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn run_login_with_api_key(
    cli_config_overrides: CliConfigOverrides,
    api_key: String,
) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;
    let _login_log_guard = init_login_file_logging(&config);
    tracing::info!("starting xAI api key login flow");

    match login_with_api_key(
        &config.codex_home,
        &api_key,
        config.cli_auth_credentials_store_mode,
        config.auth_keyring_backend_kind(),
    ) {
        Ok(()) => {
            eprintln!("{LOGIN_SUCCESS_MESSAGE}");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Error logging in: {e}");
            std::process::exit(1);
        }
    }
}

pub fn read_api_key_from_stdin() -> String {
    read_stdin_secret(
        &format!(
            "--with-api-key expects the API key on stdin. Try piping it, e.g. `printenv {XAI_API_KEY_ENV_VAR} | codex login --with-api-key`."
        ),
        "Reading API key from stdin...",
        "No API key provided via stdin.",
    )
}

fn read_stdin_secret(terminal_message: &str, reading_message: &str, empty_message: &str) -> String {
    let mut stdin = std::io::stdin();

    if stdin.is_terminal() {
        eprintln!("{terminal_message}");
        std::process::exit(1);
    }

    eprintln!("{reading_message}");

    let mut buffer = String::new();
    if let Err(err) = stdin.read_to_string(&mut buffer) {
        eprintln!("Failed to read stdin: {err}");
        std::process::exit(1);
    }

    let secret = buffer.trim().to_string();
    if secret.is_empty() {
        eprintln!("{empty_message}");
        std::process::exit(1);
    }

    secret
}

pub async fn run_login_status(cli_config_overrides: CliConfigOverrides) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;
    let auth_route_config = config.auth_route_config();

    match CodexAuth::from_auth_storage(
        &config.codex_home,
        config.cli_auth_credentials_store_mode,
        Some(&config.chatgpt_base_url),
        config.auth_keyring_backend_kind(),
        auth_route_config.as_ref(),
    )
    .await
    {
        Ok(Some(auth)) => match auth.auth_mode() {
            AuthMode::ApiKey => match auth.get_token() {
                Ok(api_key) => {
                    eprintln!(
                        "Logged in using an xAI API key - {}",
                        safe_format_key(&api_key)
                    );
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Unexpected error retrieving API key: {e}");
                    std::process::exit(1);
                }
            },
            AuthMode::XaiOAuth => {
                eprintln!("Logged in using xAI (Grok / SuperGrok)");
                std::process::exit(0);
            }
            AuthMode::Chatgpt | AuthMode::ChatgptAuthTokens => {
                eprintln!(
                    "Logged in using legacy ChatGPT credentials (run `codex login` to switch to xAI)"
                );
                std::process::exit(0);
            }
            AuthMode::AgentIdentity => {
                eprintln!("Logged in using access token");
                std::process::exit(0);
            }
            AuthMode::PersonalAccessToken => {
                eprintln!("Logged in using personal access token");
                std::process::exit(0);
            }
            AuthMode::BedrockApiKey => {
                eprintln!("Logged in using Amazon Bedrock API key");
                std::process::exit(0);
            }
        },
        Ok(None) => {
            eprintln!(
                "Not logged in. Run `codex login` for xAI OAuth or `printenv {XAI_API_KEY_ENV_VAR} | codex login --with-api-key`."
            );
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Error checking login status: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn run_logout(cli_config_overrides: CliConfigOverrides) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;
    let auth_route_config = config.auth_route_config();

    match logout_with_revoke(
        &config.codex_home,
        config.cli_auth_credentials_store_mode,
        config.auth_keyring_backend_kind(),
        auth_route_config.as_ref(),
    )
    .await
    {
        Ok(true) => {
            eprintln!("Successfully logged out");
            std::process::exit(0);
        }
        Ok(false) => {
            eprintln!("Not logged in");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Error logging out: {e}");
            std::process::exit(1);
        }
    }
}

async fn load_config_or_exit(cli_config_overrides: CliConfigOverrides) -> Config {
    let cli_overrides = match cli_config_overrides.parse_overrides() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error parsing -c overrides: {e}");
            std::process::exit(1);
        }
    };

    match Config::load_with_cli_overrides(cli_overrides).await {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Error loading configuration: {e}");
            std::process::exit(1);
        }
    }
}

fn safe_format_key(key: &str) -> String {
    if key.len() <= 13 {
        return "***".to_string();
    }
    let prefix = &key[..8];
    let suffix = &key[key.len() - 5..];
    format!("{prefix}***{suffix}")
}

#[cfg(test)]
mod tests {
    use codex_config::types::AuthCredentialsStoreMode;
    use codex_login::AuthKeyringBackendKind;
    use codex_login::load_auth_dot_json;
    use codex_login::login_with_api_key;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use super::clear_existing_auth_before_login;
    use super::safe_format_key;

    #[tokio::test]
    async fn clears_existing_auth_before_login() {
        let codex_home = tempdir().expect("create temporary Codex home");
        login_with_api_key(
            codex_home.path(),
            "xai-test-key",
            AuthCredentialsStoreMode::File,
            AuthKeyringBackendKind::default(),
        )
        .expect("save existing auth");

        clear_existing_auth_before_login(
            codex_home.path(),
            AuthCredentialsStoreMode::File,
            AuthKeyringBackendKind::default(),
            /*auth_route_config*/ None,
        )
        .await;

        let auth = load_auth_dot_json(
            codex_home.path(),
            AuthCredentialsStoreMode::File,
            AuthKeyringBackendKind::default(),
        )
        .expect("load auth after cleanup");
        assert_eq!(auth, None);
    }

    #[test]
    fn formats_long_key() {
        let key = "xai-1234567890ABCDE";
        assert_eq!(safe_format_key(key), "xai-1234***ABCDE");
    }

    #[test]
    fn short_key_returns_stars() {
        let key = "xai-12345";
        assert_eq!(safe_format_key(key), "***");
    }
}
