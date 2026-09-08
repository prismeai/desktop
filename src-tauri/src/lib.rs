//! Prisme.ai desktop — native layer.
//!
//! Thin shell that loads the SPA served by the customer's own self-hosted
//! server. This file wires the native features a web page can't get on its own
//! inside a system WebView:
//!   - server URL persistence
//!   - the remote "app" window, created here so we can attach:
//!       * a no-op Notification stub (WKWebView has no web Notification API)
//!       * a download handler (saves to the OS Downloads folder)
//!   - native OIDC sign-in via the system browser (see `auth`)
//!   - deep links (ai.prisme.app://oauth/callback) + single-instance
//!
//! Security: sensitive custom commands are guarded to the trusted "main"
//! window. The remote "app" window is granted no IPC capability, so remote
//! content (including third-party auth pages) cannot reach any command.

mod auth;

use std::fs;
use std::path::PathBuf;
use tauri::webview::DownloadEvent;
use tauri::{Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Config {
    server_url: Option<String>,
}

/// Injected into the remote app window BEFORE its page loads.
///
/// WKWebView (macOS) does not implement the web Notification API, so we provide
/// a harmless no-op stub for `window.Notification` (reports "granted", does
/// nothing). It deliberately performs NO IPC: this same webview navigates to
/// third-party auth pages (e.g. accounts.google.com), where any `ipc://` call
/// is blocked by WebKit as insecure mixed content. Native notifications will be
/// re-introduced with an origin-scoped capability bound to the customer's
/// server origin only.
const NOTIFICATION_SHIM: &str = r#"
(function () {
  if (window.__prismeNotifShim) return;
  window.__prismeNotifShim = true;
  function PrismeNotification() {}
  PrismeNotification.permission = 'granted';
  PrismeNotification.requestPermission = function () { return Promise.resolve('granted'); };
  PrismeNotification.prototype.close = function () {};
  try {
    Object.defineProperty(window, 'Notification', {
      value: PrismeNotification, configurable: true, writable: true,
    });
  } catch (e) {}
})();
"#;

fn config_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("config.json"))
}

/// Only the trusted local setup window may call the privileged commands.
fn ensure_setup(webview: &WebviewWindow) -> Result<(), String> {
    if webview.label() == "main" {
        Ok(())
    } else {
        Err("not allowed from this window".into())
    }
}

#[tauri::command]
fn get_server_url(webview: WebviewWindow) -> Option<String> {
    ensure_setup(&webview).ok()?;
    let path = config_path(&webview.app_handle()).ok()?;
    let data = fs::read_to_string(path).ok()?;
    let cfg: Config = serde_json::from_str(&data).ok()?;
    cfg.server_url
}

#[tauri::command]
fn set_server_url(webview: WebviewWindow, url: String) -> Result<(), String> {
    ensure_setup(&webview)?;
    let path = config_path(&webview.app_handle())?;
    let cfg = Config {
        server_url: Some(url),
    };
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

/// Native OIDC sign-in via the system browser. Returns the Bearer access token
/// and the console URL to load. See `auth` for the full flow.
#[tauri::command]
async fn sign_in(
    webview: WebviewWindow,
    state: tauri::State<'_, auth::AuthState>,
    api_root: String,
) -> Result<auth::SignInResult, String> {
    ensure_setup(&webview)?;
    let client = reqwest::Client::new();

    let boot = auth::bootstrap(&client, &api_root).await?;
    let (authorization_endpoint, token_endpoint) =
        auth::discovery(&client, &boot.provider_url).await?;
    let (verifier, challenge) = auth::pkce();
    let expected_state = auth::random_b64url(16);
    let authorize_url = auth::build_authorize_url(
        &authorization_endpoint,
        &boot.client_id,
        &boot.scopes,
        &boot.api_url,
        &expected_state,
        &challenge,
    )?;

    // Arm the callback receiver, then open the system browser.
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    *state.pending.lock().unwrap() = Some(tx);
    open::that(&authorize_url).map_err(|e| format!("Could not open the browser: {e}"))?;

    // Wait for ai.prisme.app://oauth/callback (5 min).
    let callback = tokio::time::timeout(std::time::Duration::from_secs(300), rx)
        .await
        .map_err(|_| "Sign-in timed out.".to_string())?
        .map_err(|_| "Sign-in was cancelled.".to_string())?;

    let (code, returned_state) =
        auth::parse_callback(&callback).ok_or("Malformed authorization callback.")?;
    if returned_state != expected_state {
        return Err("State mismatch — sign-in aborted for safety.".into());
    }

    let (access_token, refresh_token) = auth::exchange(
        &client,
        &token_endpoint,
        &code,
        &verifier,
        &boot.client_id,
        &boot.api_url,
    )
    .await?;

    Ok(auth::SignInResult {
        access_token,
        console_url: boot.console_url,
        refresh_token,
    })
}

/// Abort an in-flight sign-in: dropping the pending callback sender makes the
/// awaiting `sign_in` resolve with a "cancelled" error, so the UI can reset
/// instead of spinning forever (e.g. the browser handoff never returned).
#[tauri::command]
fn cancel_sign_in(state: tauri::State<'_, auth::AuthState>) {
    let _ = state.pending.lock().unwrap().take();
}

/// Open the remote server in its own window and hand it the Bearer token (the
/// SPA reads `platform-token` when `window.__PRISME_DESKTOP__` is set), then
/// close the setup window. Created from Rust so we can attach handlers.
#[tauri::command]
fn open_app_window(
    webview: WebviewWindow,
    url: String,
    token: Option<String>,
) -> Result<(), String> {
    ensure_setup(&webview)?;
    let parsed = tauri::Url::parse(&url).map_err(|e| e.to_string())?;
    let app = webview.app_handle().clone();

    let mut init = NOTIFICATION_SHIM.to_string();
    if let Some(token) = token {
        let literal = serde_json::to_string(&token).unwrap_or_else(|_| "\"\"".into());
        init.push_str(&format!(
            "\nwindow.__PRISME_DESKTOP__ = true; try {{ localStorage.setItem('platform-token', {literal}); }} catch (e) {{}}"
        ));
    }

    WebviewWindowBuilder::new(&app, "app", WebviewUrl::External(parsed))
        .title("Prisme.ai")
        .inner_size(1440.0, 900.0)
        .min_inner_size(800.0, 600.0)
        .initialization_script(&init)
        .on_download(|webview, event| {
            // Redirect downloads to the OS Downloads folder, keeping the name.
            if let DownloadEvent::Requested { destination, .. } = event {
                if let (Ok(dir), Some(name)) =
                    (webview.path().download_dir(), destination.file_name())
                {
                    *destination = dir.join(name);
                }
            }
            true
        })
        .build()
        .map_err(|e| e.to_string())?;

    if let Some(main) = app.get_webview_window("main") {
        let _ = main.close();
    }
    Ok(())
}

/// Check the release feed for a newer signed build; download, install, restart.
#[cfg(desktop)]
async fn check_for_updates(app: tauri::AppHandle) {
    use tauri_plugin_updater::UpdaterExt;
    let updater = match app.updater() {
        Ok(u) => u,
        Err(_) => return,
    };
    if let Ok(Some(update)) = updater.check().await {
        if update
            .download_and_install(|_chunk, _total| {}, || {})
            .await
            .is_ok()
        {
            app.restart();
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(auth::AuthState::default())
        // single-instance MUST be registered first so a second launch (e.g. a
        // deep link) is routed to the running app instead of starting a new one.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app
                .get_webview_window("app")
                .or_else(|| app.get_webview_window("main"))
            {
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            #[cfg(desktop)]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let handle = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    for url in event.urls() {
                        // Route the OAuth callback to the waiting sign-in;
                        // otherwise just bring the app to the front.
                        auth::deliver_callback(
                            handle.state::<auth::AuthState>().inner(),
                            url.as_str(),
                        );
                    }
                    if let Some(w) = handle
                        .get_webview_window("main")
                        .or_else(|| handle.get_webview_window("app"))
                    {
                        let _ = w.set_focus();
                    }
                });

                // Silent self-update on startup. In dev (or with no reachable
                // release feed) check() just errors out and is ignored.
                let updater_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    check_for_updates(updater_handle).await;
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_server_url,
            set_server_url,
            sign_in,
            cancel_sign_in,
            open_app_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
