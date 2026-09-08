//! Native OIDC sign-in (Authorization Code + PKCE), mirroring the Prisme.ai
//! mobile client. The IdP step runs in the SYSTEM BROWSER (never an embedded
//! webview — Google blocks those), the callback returns via the
//! `ai.prisme.app://oauth/callback` deep link, and the code is exchanged for a
//! Bearer access token. No cookies: the token is handed to the app webview,
//! which authenticates with `Authorization: Bearer`.
//!
//! Contract discovered from the mobile app / api-gateway:
//!   - `GET {apiRoot}/v2/login/providers` → `oidc { apiUrl, providerUrl,
//!     consoleUrl, externalClientIds[], scopes[] }`. nativeClientID =
//!     externalClientIds[0].
//!   - discovery lives under `/oidc`: `{providerUrl}/oidc/.well-known/openid-configuration`.
//!   - the `resource` indicator (= apiUrl) is REQUIRED, else the token is
//!     opaque and every write 401s.
//!   - public client, PKCE only (no secret).

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Mutex;
use tokio::sync::oneshot;

pub const REDIRECT_URI: &str = "ai.prisme.app://oauth/callback";

/// Resource scopes the token must carry, or writes are refused. Mirrors the web
/// SDK's request (see mobile `InstanceLoginProviders.requestedScopes`).
const RESOURCE_SCOPES: [&str; 7] = [
    "settings",
    "events:write",
    "events:read",
    "webhooks",
    "pages:read",
    "files:write",
    "files:read",
];

/// Holds the sender for the in-flight authorization callback.
#[derive(Default)]
pub struct AuthState {
    pub pending: Mutex<Option<oneshot::Sender<String>>>,
}

/// Deliver an incoming deep-link URL to a waiting sign-in. Returns true when the
/// URL is the OAuth callback (and was consumed).
pub fn deliver_callback(state: &AuthState, url: &str) -> bool {
    if !url.starts_with(REDIRECT_URI) {
        return false;
    }
    if let Some(tx) = state.pending.lock().unwrap().take() {
        let _ = tx.send(url.to_string());
    }
    true
}

#[derive(Deserialize)]
struct OidcConfig {
    #[serde(rename = "apiUrl")]
    api_url: String,
    #[serde(rename = "providerUrl")]
    provider_url: String,
    #[serde(rename = "consoleUrl")]
    console_url: String,
    #[serde(rename = "externalClientIds", default)]
    external_client_ids: Vec<String>,
    #[serde(default)]
    scopes: Vec<String>,
}

#[derive(Deserialize)]
struct LoginProviders {
    oidc: Option<OidcConfig>,
}

#[derive(Deserialize)]
struct DiscoveryDoc {
    #[serde(rename = "authorization_endpoint")]
    authorization_endpoint: String,
    #[serde(rename = "token_endpoint")]
    token_endpoint: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    #[serde(rename = "access_token")]
    access_token: String,
    #[serde(rename = "refresh_token")]
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct WebSessionTicketResponse {
    url: String,
}

/// What the connection screen needs after a successful sign-in: the single-use
/// exchange URL to open in the webview. Loading it burns the ticket, sets the
/// httpOnly `access-token` cookie IN THE WEBVIEW, and redirects to the console.
/// No token ever reaches the page's JS.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignInResult {
    pub exchange_url: String,
}

pub struct Bootstrap {
    pub client_id: String,
    pub api_url: String,
    pub provider_url: String,
    pub console_url: String,
    pub scopes: Vec<String>,
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn random_b64url(n: usize) -> String {
    let mut buf = vec![0u8; n];
    getrandom::getrandom(&mut buf).expect("system RNG unavailable");
    b64url(&buf)
}

/// (verifier, challenge) — S256, base64url no padding (RFC 7636).
pub fn pkce() -> (String, String) {
    let verifier = random_b64url(32);
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let challenge = b64url(&hasher.finalize());
    (verifier, challenge)
}

fn requested_scopes(advertised: &[String]) -> Vec<String> {
    let mut scopes: Vec<String> = if advertised.is_empty() {
        ["openid", "profile", "email", "offline_access"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        advertised.to_vec()
    };
    for s in RESOURCE_SCOPES {
        if !scopes.iter().any(|x| x == s) {
            scopes.push(s.to_string());
        }
    }
    scopes
}

/// Extract (code, state) from `ai.prisme.app://oauth/callback?code=…&state=…`.
/// Parsed manually because the `url` crate treats custom-scheme authorities
/// inconsistently.
pub fn parse_callback(raw: &str) -> Option<(String, String)> {
    let query = raw.split_once('?').map(|(_, q)| q)?;
    let mut code = None;
    let mut state = None;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            let val = urlencoding::decode(v).ok()?.into_owned();
            match k {
                "code" => code = Some(val),
                "state" => state = Some(val),
                _ => {}
            }
        }
    }
    Some((code?, state?))
}

fn discovery_url(provider_url: &str) -> String {
    format!(
        "{}/oidc/.well-known/openid-configuration",
        provider_url.trim_end_matches('/')
    )
}

/// `GET {apiRoot}/v2/login/providers` (public).
pub async fn bootstrap(client: &reqwest::Client, api_root: &str) -> Result<Bootstrap, String> {
    let url = format!("{}/v2/login/providers", api_root.trim_end_matches('/'));
    let resp = client
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Could not reach the server: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Server returned HTTP {} for {url}", resp.status()));
    }
    let providers: LoginProviders = resp
        .json()
        .await
        .map_err(|e| format!("Unreadable server config: {e}"))?;
    let oidc = providers
        .oidc
        .ok_or_else(|| "This server does not offer native sign-in.".to_string())?;
    let client_id = oidc
        .external_client_ids
        .into_iter()
        .next()
        .ok_or_else(|| "This server declares no native OIDC client.".to_string())?;
    let scopes = requested_scopes(&oidc.scopes);
    Ok(Bootstrap {
        client_id,
        api_url: oidc.api_url,
        provider_url: oidc.provider_url,
        console_url: oidc.console_url,
        scopes,
    })
}

/// OIDC discovery (endpoints).
pub async fn discovery(
    client: &reqwest::Client,
    provider_url: &str,
) -> Result<(String, String), String> {
    let url = discovery_url(provider_url);
    let resp = client
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Could not reach the identity provider: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Discovery returned HTTP {}", resp.status()));
    }
    let doc: DiscoveryDoc = resp
        .json()
        .await
        .map_err(|e| format!("Unreadable discovery document: {e}"))?;
    Ok((doc.authorization_endpoint, doc.token_endpoint))
}

/// Build the authorization URL to open in the system browser.
pub fn build_authorize_url(
    authorization_endpoint: &str,
    client_id: &str,
    scopes: &[String],
    resource: &str,
    state: &str,
    challenge: &str,
) -> Result<String, String> {
    let mut url = tauri::Url::parse(authorization_endpoint)
        .map_err(|e| format!("Bad authorization endpoint: {e}"))?;
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("scope", &scopes.join(" "))
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("resource", resource);
    Ok(url.to_string())
}

/// Exchange the authorization code for tokens. Public client: PKCE verifier is
/// the only proof, no client secret. `resource` must match the authorize step.
pub async fn exchange(
    client: &reqwest::Client,
    token_endpoint: &str,
    code: &str,
    verifier: &str,
    client_id: &str,
    resource: &str,
) -> Result<(String, Option<String>), String> {
    let resp = client
        .post(token_endpoint)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT_URI),
            ("client_id", client_id),
            ("code_verifier", verifier),
            ("resource", resource),
        ])
        .send()
        .await
        .map_err(|e| format!("Token exchange failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Token exchange rejected (HTTP {status}): {body}"));
    }
    let tokens: TokenResponse = resp
        .json()
        .await
        .map_err(|e| format!("Unreadable token response: {e}"))?;
    Ok((tokens.access_token, tokens.refresh_token))
}

/// Mint a single-use web-session ticket from the Bearer token and return the
/// exchange URL (`{apiUrl}/user/webSession?ticket=…`). Opening that URL in the
/// webview sets the httpOnly session cookie and redirects to `redirect`.
/// Mirrors the mobile client (`PrismeClient.webSessionURL`, platform #154).
/// The bearer never transits in a URL — only the opaque ticket does.
pub async fn web_session_url(
    client: &reqwest::Client,
    api_url: &str,
    access_token: &str,
    redirect: &str,
) -> Result<String, String> {
    let endpoint = format!("{}/user/webSessionTicket", api_url.trim_end_matches('/'));
    let resp = client
        .post(&endpoint)
        .bearer_auth(access_token)
        .json(&serde_json::json!({ "redirect": redirect }))
        .send()
        .await
        .map_err(|e| format!("Web session ticket request failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Web session ticket rejected (HTTP {status}): {body}"));
    }
    let ticket: WebSessionTicketResponse = resp
        .json()
        .await
        .map_err(|e| format!("Unreadable web session ticket response: {e}"))?;
    Ok(ticket.url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_b64url(s: &str) -> bool {
        !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    }

    #[test]
    fn pkce_is_s256_and_url_safe() {
        let (verifier, challenge) = pkce();
        assert!(is_b64url(&verifier), "verifier must be base64url");
        assert!(is_b64url(&challenge), "challenge must be base64url");
        // 32 random bytes -> 43 base64url chars (no padding).
        assert_eq!(verifier.len(), 43);
        // challenge == base64url(SHA256(verifier))
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        assert_eq!(challenge, b64url(&hasher.finalize()));
    }

    #[test]
    fn random_b64url_has_expected_shape() {
        let s = random_b64url(16);
        assert!(is_b64url(&s));
        assert_eq!(s.len(), 22); // 16 bytes -> 22 base64url chars
        assert_ne!(random_b64url(16), random_b64url(16)); // not constant
    }

    #[test]
    fn requested_scopes_defaults_when_empty() {
        let scopes = requested_scopes(&[]);
        for s in ["openid", "profile", "email", "offline_access"] {
            assert!(scopes.contains(&s.to_string()), "missing default {s}");
        }
        for s in RESOURCE_SCOPES {
            assert!(scopes.contains(&s.to_string()), "missing resource {s}");
        }
        assert_eq!(scopes.len(), 4 + RESOURCE_SCOPES.len());
    }

    #[test]
    fn requested_scopes_dedupes_and_keeps_advertised_first() {
        let advertised = vec!["openid".to_string(), "settings".to_string()];
        let scopes = requested_scopes(&advertised);
        assert_eq!(&scopes[0], "openid");
        assert_eq!(&scopes[1], "settings");
        // "settings" is both advertised and a resource scope — must appear once.
        assert_eq!(scopes.iter().filter(|s| *s == "settings").count(), 1);
        // advertised(2) + resource(7) - overlap(1) = 8
        assert_eq!(scopes.len(), 8);
    }

    #[test]
    fn parse_callback_extracts_code_and_state() {
        let (code, state) =
            parse_callback("ai.prisme.app://oauth/callback?code=abc123&state=xyz789").unwrap();
        assert_eq!(code, "abc123");
        assert_eq!(state, "xyz789");
    }

    #[test]
    fn parse_callback_url_decodes_and_ignores_order_and_extras() {
        let (code, state) = parse_callback(
            "ai.prisme.app://oauth/callback?state=a%2Fb&iss=whatever&code=c%3Dd",
        )
        .unwrap();
        assert_eq!(code, "c=d");
        assert_eq!(state, "a/b");
    }

    #[test]
    fn parse_callback_rejects_missing_fields() {
        assert!(parse_callback("ai.prisme.app://oauth/callback?code=only").is_none());
        assert!(parse_callback("ai.prisme.app://oauth/callback?state=only").is_none());
        assert!(parse_callback("ai.prisme.app://oauth/callback").is_none());
    }

    #[test]
    fn discovery_url_lives_under_oidc_prefix() {
        assert_eq!(
            discovery_url("https://api.sandbox.prisme.ai"),
            "https://api.sandbox.prisme.ai/oidc/.well-known/openid-configuration"
        );
        // trailing slash must not double up
        assert_eq!(
            discovery_url("https://api.sandbox.prisme.ai/"),
            "https://api.sandbox.prisme.ai/oidc/.well-known/openid-configuration"
        );
    }

    #[test]
    fn authorize_url_carries_all_pkce_params() {
        let url = build_authorize_url(
            "https://api.sandbox.prisme.ai/oidc/auth",
            "ai.prisme.app",
            &["openid".to_string(), "settings".to_string()],
            "https://api.sandbox.prisme.ai/v2",
            "STATE",
            "CHALLENGE",
        )
        .unwrap();
        let parsed = tauri::Url::parse(&url).unwrap();
        let q: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(q["client_id"], "ai.prisme.app");
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["redirect_uri"], REDIRECT_URI);
        assert_eq!(q["scope"], "openid settings");
        assert_eq!(q["state"], "STATE");
        assert_eq!(q["code_challenge"], "CHALLENGE");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["resource"], "https://api.sandbox.prisme.ai/v2");
    }

    #[test]
    fn deliver_callback_routes_only_matching_urls() {
        // matching URL is delivered to the waiting receiver
        let state = AuthState::default();
        let (tx, mut rx) = oneshot::channel::<String>();
        *state.pending.lock().unwrap() = Some(tx);
        let cb = "ai.prisme.app://oauth/callback?code=c&state=s";
        assert!(deliver_callback(&state, cb));
        assert_eq!(rx.try_recv().unwrap(), cb);

        // an unrelated deep link is not consumed
        let state2 = AuthState::default();
        let (tx2, _rx2) = oneshot::channel::<String>();
        *state2.pending.lock().unwrap() = Some(tx2);
        assert!(!deliver_callback(&state2, "https://example.com/whatever"));
        assert!(state2.pending.lock().unwrap().is_some());
    }
}
