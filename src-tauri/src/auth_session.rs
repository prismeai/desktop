//! Cross-platform "system web-auth session" — ONE interface, per-OS impls.
//!
//! Given the authorization URL and the callback scheme, it drives the login in
//! a system-managed browser and returns the callback URL once the browser
//! redirects to `<scheme>://…`.
//!
//! By design, every platform routes through `authenticate()`:
//!   - macOS   → `ASWebAuthenticationSession` (native sheet, AUTO-CLOSES,
//!               ephemeral) — premium, mobile parity.
//!   - Windows → `WebAuthenticationBroker`.                             [PLANNED]
//!   - baseline (Windows/Linux today) → system browser (`open`) + the
//!     custom-scheme deep link delivered back via `AuthState`.

use crate::auth::AuthState;
use std::time::Duration;

/// Timeout for the whole interactive auth (password + MFA).
const AUTH_TIMEOUT: Duration = Duration::from_secs(300);

/// Sentinel returned when the user cancels the sheet (closes it without signing
/// in). The frontend treats it silently — no error banner, just reset.
pub const CANCELLED: &str = "__cancelled__";

/// Run the interactive login and return the raw callback URL
/// (`<scheme>://…?code=…&state=…`).
pub async fn authenticate(
    app: &tauri::AppHandle,
    state: &AuthState,
    start_url: &str,
    callback_scheme: &str,
) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        let _ = state;
        macos::run(app, start_url, callback_scheme).await
    }

    #[cfg(not(target_os = "macos"))]
    {
        // TODO(windows): WebAuthenticationBroker.
        let _ = (app, callback_scheme);
        baseline(state, start_url).await
    }
}

/// Shared baseline: open the system browser, await the deep-link callback that
/// `AuthState` receives from the OS. Cancellable via `cancel_sign_in`.
#[cfg(not(target_os = "macos"))]
async fn baseline(state: &AuthState, start_url: &str) -> Result<String, String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    *state.pending.lock().unwrap() = Some(tx);
    open::that(start_url).map_err(|e| format!("Could not open the browser: {e}"))?;
    tokio::time::timeout(AUTH_TIMEOUT, rx)
        .await
        .map_err(|_| "Sign-in timed out.".to_string())?
        .map_err(|_| CANCELLED.to_string())
}

#[cfg(target_os = "macos")]
mod macos {
    use super::AUTH_TIMEOUT;
    use block2::RcBlock;
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
    use objc2::{class, declare_class, msg_send, msg_send_id, mutability, ClassType, DeclaredClass};
    use objc2_foundation::{NSError, NSString, NSURL};
    use std::sync::{Arc, Mutex};
    use tauri::Manager;

    type Slot = Arc<Mutex<Option<tokio::sync::oneshot::Sender<Result<String, String>>>>>;

    struct AnchorIvars {
        /// The app's NSWindow pointer (used as the ASPresentationAnchor).
        anchor: usize,
    }

    declare_class!(
        struct AnchorProvider;

        unsafe impl ClassType for AnchorProvider {
            type Super = NSObject;
            type Mutability = mutability::InteriorMutable;
            const NAME: &'static str = "PrismeaiAnchorProvider";
        }

        impl DeclaredClass for AnchorProvider {
            type Ivars = AnchorIvars;
        }

        unsafe impl NSObjectProtocol for AnchorProvider {}

        unsafe impl AnchorProvider {
            #[method_id(presentationAnchorForWebAuthenticationSession:)]
            fn presentation_anchor(&self, _session: &AnyObject) -> Retained<AnyObject> {
                let ptr = self.ivars().anchor as *mut AnyObject;
                unsafe { Retained::retain(ptr) }.expect("anchor window is null")
            }
        }
    );

    impl AnchorProvider {
        fn new(anchor: usize) -> Retained<Self> {
            let this = Self::alloc().set_ivars(AnchorIvars { anchor });
            unsafe { msg_send_id![super(this), init] }
        }
    }

    /// Read the callback URL (or an error) from the completion args.
    /// `ASWebAuthenticationSessionErrorCodeCanceledLogin` (== 1) means the user
    /// closed the sheet — surfaced as the silent CANCELLED sentinel.
    unsafe fn extract(cb_url: *mut NSURL, cb_err: *mut NSError) -> Result<String, String> {
        if !cb_url.is_null() {
            if let Some(abs) = (*cb_url).absoluteString() {
                return Ok(abs.to_string());
            }
        }
        if !cb_err.is_null() {
            let err = &*cb_err;
            let code: isize = msg_send![err, code];
            if code == 1 {
                return Err(super::CANCELLED.to_string());
            }
            return Err(err.localizedDescription().to_string());
        }
        Err(super::CANCELLED.to_string())
    }

    pub(super) async fn run(
        app: &tauri::AppHandle,
        url: &str,
        scheme: &str,
    ) -> Result<String, String> {
        let window = app
            .get_webview_window("main")
            .or_else(|| app.get_webview_window("app"))
            .ok_or_else(|| "No window to present the sign-in sheet.".to_string())?;
        let anchor = window.ns_window().map_err(|e| e.to_string())? as usize;

        let url = url.to_string();
        let scheme = scheme.to_string();
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
        let slot: Slot = Arc::new(Mutex::new(Some(tx)));
        let slot_block = slot.clone();

        app.run_on_main_thread(move || {
            let send = move |r: Result<String, String>| {
                if let Some(tx) = slot_block.lock().unwrap().take() {
                    let _ = tx.send(r);
                }
            };

            let ns_scheme = NSString::from_str(&scheme);
            let ns_url_str = NSString::from_str(&url);
            let ns_url = match unsafe { NSURL::URLWithString(&ns_url_str) } {
                Some(u) => u,
                None => {
                    send(Err("Invalid authorization URL.".into()));
                    return;
                }
            };

            let send_cb = send.clone();
            let handler = RcBlock::new(move |cb_url: *mut NSURL, cb_err: *mut NSError| {
                let r = unsafe { extract(cb_url, cb_err) };
                send_cb(r);
            });

            let provider = AnchorProvider::new(anchor);

            let cls = class!(ASWebAuthenticationSession);
            let alloc: Allocated<AnyObject> = unsafe { msg_send_id![cls, alloc] };
            let session: Retained<AnyObject> = unsafe {
                msg_send_id![
                    alloc,
                    initWithURL: &*ns_url,
                    callbackURLScheme: &*ns_scheme,
                    completionHandler: &*handler,
                ]
            };

            unsafe {
                let _: () =
                    msg_send![&*session, setPresentationContextProvider: &*provider];
                let _: () = msg_send![&*session, setPrefersEphemeralWebBrowserSession: true];
                let started: bool = msg_send![&*session, start];
                if !started {
                    send(Err("Could not start the sign-in session.".into()));
                    return;
                }
            }

            // Keep the session + provider alive for the duration of the sheet.
            // (ASWebAuthenticationSession holds only a WEAK ref to the provider.)
            std::mem::forget(session);
            std::mem::forget(provider);
            std::mem::forget(handler);
        })
        .map_err(|e| format!("Could not schedule the sign-in sheet: {e}"))?;

        match tokio::time::timeout(AUTH_TIMEOUT, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(super::CANCELLED.to_string()),
            Err(_) => Err("Sign-in timed out.".into()),
        }
    }
}
