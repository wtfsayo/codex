//! xAI (Grok / SuperGrok) OAuth loopback login flow.
//!
//! This module implements the PKCE loopback OAuth flow against `auth.x.ai`,
//! mirroring the ChatGPT flow in `server.rs` but targeting xAI's issuer. The
//! resulting tokens are persisted in `auth.json` under `AuthMode::XaiOAuth`
//! and refreshed against `https://auth.x.ai/oauth/token`.

use base64::Engine;
use rand::RngCore;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use tiny_http::Header;
use tiny_http::Request;
use tiny_http::Response;
use tiny_http::Server;
use tiny_http::StatusCode;
use tracing::info;
use tracing::warn;

use crate::auth::AuthDotJson;
use crate::auth::AuthKeyringBackendKind;
use crate::auth::save_auth;
use crate::default_client::build_raw_auth_reqwest_client;
use crate::outbound_proxy::AuthRouteConfig;
use crate::pkce::PkceCodes;
use crate::pkce::generate_pkce;
use crate::token_data::TokenData;
use crate::token_data::parse_chatgpt_jwt_claims;
use chrono::Utc;
use codex_config::types::AuthCredentialsStoreMode;
use codex_protocol::auth::AuthMode;

/// xAI OAuth issuer.
pub const XAI_OAUTH_ISSUER: &str = "https://auth.x.ai";

/// xAI OAuth discovery document URL.
pub const XAI_OAUTH_DISCOVERY_URL: &str = "https://auth.x.ai/.well-known/openid-configuration";

/// xAI OAuth client id for Codex CLI. This is the public SuperGrok CLI client.
pub const XAI_OAUTH_CLIENT_ID: &str = "org_4t2qZ4p1vM5K9nL7x";

/// OAuth scopes requested from xAI.
pub const XAI_OAUTH_SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";

/// Loopback callback host.
pub const XAI_OAUTH_REDIRECT_HOST: &str = "127.0.0.1";

/// Loopback callback port.
pub const XAI_OAUTH_REDIRECT_PORT: u16 = 56121;

/// Loopback callback path.
pub const XAI_OAUTH_REDIRECT_PATH: &str = "/callback";

/// xAI inference base URL.
pub const XAI_INFERENCE_BASE_URL: &str = "https://api.x.ai/v1";

/// Options for launching the xAI local login callback server.
#[derive(Debug, Clone)]
pub struct XaiLoginServerOptions {
    pub codex_home: PathBuf,
    pub auth_credentials_store_mode: AuthCredentialsStoreMode,
    pub auth_keyring_backend_kind: AuthKeyringBackendKind,
    pub auth_route_config: Option<AuthRouteConfig>,
    pub open_browser: bool,
}

impl XaiLoginServerOptions {
    pub fn new(
        codex_home: PathBuf,
        auth_credentials_store_mode: AuthCredentialsStoreMode,
        auth_keyring_backend_kind: AuthKeyringBackendKind,
        auth_route_config: Option<AuthRouteConfig>,
    ) -> Self {
        Self {
            codex_home,
            auth_credentials_store_mode,
            auth_keyring_backend_kind,
            auth_route_config,
            open_browser: true,
        }
    }
}

/// Handle for a running xAI login callback server.
pub struct XaiLoginServer {
    pub auth_url: String,
    pub actual_port: u16,
    server_handle: tokio::task::JoinHandle<io::Result<()>>,
    shutdown_notify: Arc<tokio::sync::Notify>,
}

impl XaiLoginServer {
    /// Waits for the login callback loop to finish.
    pub async fn block_until_done(self) -> io::Result<()> {
        self.server_handle
            .await
            .map_err(|err| io::Error::other(format!("xai login server thread panicked: {err:?}")))?
    }

    /// Requests shutdown of the callback server.
    pub fn cancel(&self) {
        self.shutdown_notify.notify_one();
    }
}

#[derive(Clone, Debug)]
pub struct XaiShutdownHandle {
    shutdown_notify: Arc<tokio::sync::Notify>,
}

impl XaiShutdownHandle {
    pub fn shutdown(&self) {
        self.shutdown_notify.notify_one();
    }
}

#[allow(dead_code)]
enum HandledRequest {
    Response(Response<std::io::Cursor<Vec<u8>>>),
    ResponseAndExit {
        headers: Vec<Header>,
        body: Vec<u8>,
        result: io::Result<()>,
    },
    RedirectWithHeader(Header),
}

/// Starts a local callback server for xAI OAuth and returns the browser auth URL.
pub fn run_xai_login_server(opts: XaiLoginServerOptions) -> io::Result<XaiLoginServer> {
    let pkce = generate_pkce();
    let state = generate_state();

    let bind_address = format!("{XAI_OAUTH_REDIRECT_HOST}:{XAI_OAUTH_REDIRECT_PORT}");
    let server = Server::http(&bind_address).map_err(|err| io::Error::other(err))?;
    let actual_port = match server.server_addr().to_ip() {
        Some(addr) => addr.port(),
        None => {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "Unable to determine the xAI login server port",
            ));
        }
    };
    let server = Arc::new(server);

    let redirect_uri = format!(
        "http://{XAI_OAUTH_REDIRECT_HOST}:{actual_port}{XAI_OAUTH_REDIRECT_PATH}"
    );
    let auth_url = build_xai_authorize_url(&redirect_uri, &pkce, &state);

    if opts.open_browser {
        let _ = webbrowser::open(&auth_url);
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Request>(16);
    let _server_handle = {
        let server = Arc::clone(&server);
        thread::spawn(move || -> io::Result<()> {
            while let Ok(request) = server.recv() {
                match tx.blocking_send(request) {
                    Ok(()) => {}
                    Err(error) => {
                        eprintln!("Failed to send request to channel: {error}");
                        return Err(io::Error::other("Failed to send request to channel"));
                    }
                }
            }
            Ok(())
        })
    };

    let shutdown_notify = Arc::new(tokio::sync::Notify::new());
    let server_handle = {
        let shutdown_notify = shutdown_notify.clone();
        tokio::spawn(async move {
            let result = loop {
                tokio::select! {
                    _ = shutdown_notify.notified() => {
                        break Err(io::Error::other("xAI login was not completed"));
                    }
                    maybe_req = rx.recv() => {
                        let Some(req) = maybe_req else {
                            break Err(io::Error::other("xAI login was not completed"));
                        };

                        let url_raw = req.url().to_string();
                        let response = process_xai_request(
                            &url_raw,
                            &opts,
                            &redirect_uri,
                            &pkce,
                            &state,
                        )
                        .await;

                        let exit_result = match response {
                            HandledRequest::Response(response) => {
                                let _ = tokio::task::spawn_blocking(move || req.respond(response)).await;
                                None
                            }
                            HandledRequest::ResponseAndExit {
                                headers,
                                body,
                                result,
                            } => {
                                let _ = tokio::task::spawn_blocking(move || {
                                    send_response_with_disconnect(req, headers, body)
                                })
                                .await;
                                Some(result)
                            }
                            HandledRequest::RedirectWithHeader(header) => {
                                let redirect = Response::empty(302).with_header(header);
                                let _ = tokio::task::spawn_blocking(move || req.respond(redirect)).await;
                                None
                            }
                        };

                        if let Some(result) = exit_result {
                            break result;
                        }
                    }
                }
            };

            server.unblock();
            result
        })
    };

    Ok(XaiLoginServer {
        auth_url,
        actual_port,
        server_handle,
        shutdown_notify,
    })
}

async fn process_xai_request(
    url_raw: &str,
    opts: &XaiLoginServerOptions,
    redirect_uri: &str,
    pkce: &PkceCodes,
    state: &str,
) -> HandledRequest {
    let parsed_url = match url::Url::parse(&format!("http://localhost{url_raw}")) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("URL parse error: {e}");
            return HandledRequest::Response(
                Response::from_string("Bad Request").with_status_code(400),
            );
        }
    };
    let path = parsed_url.path().to_string();

    match path.as_str() {
        XAI_OAUTH_REDIRECT_PATH => {
            let params: std::collections::HashMap<String, String> =
                parsed_url.query_pairs().into_owned().collect();
            let has_code = params.get("code").is_some_and(|code| !code.is_empty());
            let has_state = params.get("state").is_some_and(|state| !state.is_empty());
            let has_error = params.get("error").is_some_and(|error| !error.is_empty());
            let state_valid = params.get("state").map(String::as_str) == Some(state);
            info!(
                path = %path,
                has_code,
                has_state,
                has_error,
                state_valid,
                "received xai login callback"
            );
            if !state_valid {
                warn!(
                    path = %path,
                    has_code,
                    has_state,
                    has_error,
                    "xai login callback state mismatch"
                );
                return HandledRequest::Response(
                    Response::from_string("State mismatch").with_status_code(400),
                );
            }
            if let Some(error_code) = params.get("error") {
                let error_description = params.get("error_description").map(String::as_str);
                let message = format!(
                    "xAI sign-in failed: {}",
                    error_description.unwrap_or(error_code)
                );
                eprintln!("{message}");
                return HandledRequest::ResponseAndExit {
                    headers: vec![],
                    body: message.clone().into_bytes(),
                    result: Err(io::Error::new(io::ErrorKind::PermissionDenied, message)),
                };
            }
            let code = match params.get("code") {
                Some(c) if !c.is_empty() => c.clone(),
                _ => {
                    return HandledRequest::ResponseAndExit {
                        headers: vec![],
                        body: b"Missing authorization code".to_vec(),
                        result: Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "Missing authorization code",
                        )),
                    };
                }
            };

            match exchange_xai_code_for_tokens(
                redirect_uri,
                pkce,
                &code,
                opts.auth_route_config.as_ref(),
            )
            .await
            {
                Ok(tokens) => {
                    if let Err(err) = persist_xai_tokens_async(
                        &opts.codex_home,
                        tokens.id_token,
                        tokens.access_token,
                        tokens.refresh_token,
                        opts.auth_credentials_store_mode,
                        opts.auth_keyring_backend_kind,
                    )
                    .await
                    {
                        eprintln!("xAI persist error: {err}");
                        return HandledRequest::ResponseAndExit {
                            headers: vec![],
                            body: b"Sign-in completed but credentials could not be saved locally".to_vec(),
                            result: Err(io::Error::other(format!(
                                "xAI persist failed: {err}"
                            ))),
                        };
                    }

                    let success_body = include_str!("assets/success.html");
                    HandledRequest::ResponseAndExit {
                        headers: match Header::from_bytes(
                            &b"Content-Type"[..],
                            &b"text/html; charset=utf-8"[..],
                        ) {
                            Ok(header) => vec![header],
                            Err(_) => Vec::new(),
                        },
                        body: success_body.as_bytes().to_vec(),
                        result: Ok(()),
                    }
                }
                Err(err) => {
                    eprintln!("xAI token exchange error: {err}");
                    HandledRequest::ResponseAndExit {
                        headers: vec![],
                        body: format!("xAI token exchange failed: {err}").into_bytes(),
                        result: Err(io::Error::other(format!(
                            "xAI token exchange failed: {err}"
                        ))),
                    }
                }
            }
        }
        "/cancel" => HandledRequest::ResponseAndExit {
            headers: Vec::new(),
            body: b"Login cancelled".to_vec(),
            result: Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "xAI login cancelled",
            )),
        },
        _ => HandledRequest::Response(Response::from_string("Not Found").with_status_code(404)),
    }
}

fn build_xai_authorize_url(redirect_uri: &str, pkce: &PkceCodes, state: &str) -> String {
    let query = vec![
        ("response_type".to_string(), "code".to_string()),
        ("client_id".to_string(), XAI_OAUTH_CLIENT_ID.to_string()),
        ("redirect_uri".to_string(), redirect_uri.to_string()),
        ("scope".to_string(), XAI_OAUTH_SCOPE.to_string()),
        (
            "code_challenge".to_string(),
            pkce.code_challenge.to_string(),
        ),
        ("code_challenge_method".to_string(), "S256".to_string()),
        ("state".to_string(), state.to_string()),
    ];
    let qs = query
        .into_iter()
        .map(|(k, v)| format!("{k}={}", urlencoding::encode(&v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{XAI_OAUTH_ISSUER}/oauth/authorize?{qs}")
}

fn generate_state() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Tokens returned by the xAI OAuth authorization-code exchange.
pub struct XaiExchangedTokens {
    pub id_token: String,
    pub access_token: String,
    pub refresh_token: String,
}

/// Exchanges an authorization code for xAI tokens.
pub(crate) async fn exchange_xai_code_for_tokens(
    redirect_uri: &str,
    pkce: &PkceCodes,
    code: &str,
    auth_route_config: Option<&AuthRouteConfig>,
) -> io::Result<XaiExchangedTokens> {
    #[derive(serde::Deserialize)]
    struct TokenResponse {
        id_token: String,
        access_token: String,
        refresh_token: String,
    }

    let client = build_raw_auth_reqwest_client(XAI_OAUTH_ISSUER, auth_route_config)?;
    let token_endpoint = format!("{XAI_OAUTH_ISSUER}/oauth/token");
    info!(
        token_endpoint = %token_endpoint,
        redirect_uri = %redirect_uri,
        "starting xai oauth token exchange"
    );
    let resp = client
        .post(token_endpoint)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!(
            "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}",
            urlencoding::encode(code),
            urlencoding::encode(redirect_uri),
            urlencoding::encode(XAI_OAUTH_CLIENT_ID),
            urlencoding::encode(&pkce.code_verifier)
        ))
        .send()
        .await
        .map_err(io::Error::other)?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.map_err(io::Error::other)?;
        return Err(io::Error::other(format!(
            "xAI token endpoint returned status {status}: {body}"
        )));
    }

    let tokens: TokenResponse = resp.json().await.map_err(io::Error::other)?;
    info!(%status, "xai oauth token exchange succeeded");
    Ok(XaiExchangedTokens {
        id_token: tokens.id_token,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
    })
}

/// Refreshes an xAI OAuth access token using the stored refresh token.
pub async fn refresh_xai_token(
    refresh_token: &str,
    auth_route_config: Option<&AuthRouteConfig>,
) -> io::Result<XaiExchangedTokens> {
    #[derive(serde::Deserialize)]
    struct TokenResponse {
        id_token: String,
        access_token: String,
        refresh_token: String,
    }

    let client = build_raw_auth_reqwest_client(XAI_OAUTH_ISSUER, auth_route_config)?;
    let token_endpoint = format!("{XAI_OAUTH_ISSUER}/oauth/token");
    let resp = client
        .post(token_endpoint)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!(
            "grant_type=refresh_token&refresh_token={}&client_id={}",
            urlencoding::encode(refresh_token),
            urlencoding::encode(XAI_OAUTH_CLIENT_ID),
        ))
        .send()
        .await
        .map_err(io::Error::other)?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.map_err(io::Error::other)?;
        return Err(io::Error::other(format!(
            "xAI token refresh returned status {status}: {body}"
        )));
    }

    let tokens: TokenResponse = resp.json().await.map_err(io::Error::other)?;
    Ok(XaiExchangedTokens {
        id_token: tokens.id_token,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
    })
}

/// Persists xAI OAuth tokens using the configured local auth store.
async fn persist_xai_tokens_async(
    codex_home: &Path,
    id_token: String,
    access_token: String,
    refresh_token: String,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
    keyring_backend_kind: AuthKeyringBackendKind,
) -> io::Result<()> {
    let codex_home = codex_home.to_path_buf();
    tokio::task::spawn_blocking(move || {
        // xAI id_tokens are JWTs but not ChatGPT-format; parse defensively.
        let id_token_info = parse_chatgpt_jwt_claims(&id_token).unwrap_or_default();
        let tokens = TokenData {
            id_token: id_token_info,
            access_token,
            refresh_token,
            account_id: None,
        };
        let auth = AuthDotJson {
            auth_mode: Some(AuthMode::XaiOAuth),
            openai_api_key: None,
            tokens: Some(tokens),
            last_refresh: Some(Utc::now()),
            agent_identity: None,
            personal_access_token: None,
            bedrock_api_key: None,
        };
        save_auth(
            &codex_home,
            &auth,
            auth_credentials_store_mode,
            keyring_backend_kind,
        )
    })
    .await
    .map_err(|e| io::Error::other(format!("xAI persist task failed: {e}")))?
}

fn send_response_with_disconnect(
    req: Request,
    mut headers: Vec<Header>,
    body: Vec<u8>,
) -> io::Result<()> {
    let status = StatusCode(200);
    let mut writer = req.into_writer();
    let reason = status.default_reason_phrase();
    write!(writer, "HTTP/1.1 {} {}\r\n", status.0, reason)?;
    headers.retain(|h| !h.field.equiv("Connection"));
    if let Ok(close_header) = Header::from_bytes(&b"Connection"[..], &b"close"[..]) {
        headers.push(close_header);
    }
    let content_length_value = format!("{}", body.len());
    if let Ok(content_length_header) =
        Header::from_bytes(&b"Content-Length"[..], content_length_value.as_bytes())
    {
        headers.push(content_length_header);
    }
    for header in headers {
        write!(
            writer,
            "{}: {}\r\n",
            header.field.as_str(),
            header.value.as_str()
        )?;
    }
    writer.write_all(b"\r\n")?;
    writer.write_all(&body)?;
    writer.flush()
}
