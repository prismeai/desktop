//! Cross-platform "system web-auth session" — ONE interface, per-OS impls.
//!
//! Given the authorization URL and the callback scheme, it drives the login in
//! a system-managed browser and returns the callback URL once the browser
//! redirects to `<scheme>://…`.
//!
//! By design, every platform routes through `authenticate()`:
//!   - macOS   → `ASWebAuthenticationSession` (native sheet, AUTO-CLOSES,
//!               ephemeral) — premium, mobile parity.                 [PLANNED]
//!   - Windows → `WebAuthenticationBroker`.                            [PLANNED]
//!   - baseline (works on every OS today) → system browser (`open`) + the
//!     custom-scheme deep link delivered back via `AuthState`. The tab stays
//!     open (Slack-style) until the native sessions above replace it.
//!
//! The premium native sessions plug in behind the SAME signature, so the rest
//! of the shell (OIDC, token exchange, webSession) never changes.

use crate::auth::AuthState;
use std::time::Duration;

/// Timeout for the whole interactive auth (user typing password + MFA).
const AUTH_TIMEOUT: Duration = Duration::from_secs(300);

/// Run the interactive login and return the raw callback URL
/// (`<scheme>://…?code=…&state=…`).
pub async fn authenticate(
    state: &AuthState,
    start_url: &str,
    callback_scheme: &str,
) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        // TODO(premium): ASWebAuthenticationSession here (auto-close, ephemeral).
        // Until then, fall through to the shared browser + deep-link baseline.
        let _ = callback_scheme;
        baseline(state, start_url).await
    }

    #[cfg(not(target_os = "macos"))]
    {
        // TODO(windows): WebAuthenticationBroker here.
        let _ = callback_scheme;
        baseline(state, start_url).await
    }
}

/// Shared baseline: open the system browser, await the deep-link callback that
/// `AuthState` receives from the OS. Cancellable via `cancel_sign_in` (drops
/// the sender → this resolves with a "cancelled" error).
async fn baseline(state: &AuthState, start_url: &str) -> Result<String, String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    *state.pending.lock().unwrap() = Some(tx);
    open::that(start_url).map_err(|e| format!("Could not open the browser: {e}"))?;
    tokio::time::timeout(AUTH_TIMEOUT, rx)
        .await
        .map_err(|_| "Sign-in timed out.".to_string())?
        .map_err(|_| "Sign-in was cancelled.".to_string())
}
