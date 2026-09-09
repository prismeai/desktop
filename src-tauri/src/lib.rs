//! Prisme.ai desktop — native layer.
//!
//! Thin shell that loads the SPA served by the customer's own self-hosted
//! server. This file wires the native features a web page can't get on its own
//! inside a system WebView:
//!   - server URL + console URL persistence (session reuse across launches)
//!   - native OIDC sign-in via the system auth session (see `auth_session`)
//!   - the remote "app" window (notification stub, download handler, logout
//!     detection)
//!   - deep links (ai.prisme.app://oauth/callback) + single-instance + updater
//!
//! Security: sensitive custom commands are guarded to the trusted "main"
//! window. The remote "app" window is granted no IPC capability, so remote
//! content (including third-party auth pages) cannot reach any command.

mod auth;
mod auth_session;

use std::fs;
use std::path::PathBuf;
use tauri::webview::DownloadEvent;
use tauri::{Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Config {
    server_url: Option<String>,
    /// Console URL of the last successful sign-in. When set, the next launch
    /// reuses the existing session by opening the app directly.
    console_url: Option<String>,
}

/// Injected into the remote app window BEFORE its page loads.
///
/// WKWebView (macOS) does not implement the web Notification API, so we provide
/// a harmless no-op stub for `window.Notification`. It performs NO IPC.
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

fn read_config(app: &tauri::AppHandle) -> Config {
    config_path(app)
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_config(app: &tauri::AppHandle, cfg: &Config) -> Result<(), String> {
    let path = config_path(app)?;
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

fn get_console_url(app: &tauri::AppHandle) -> Option<String> {
    read_config(app).console_url
}

fn set_console_url(app: &tauri::AppHandle, url: &str) {
    let mut cfg = read_config(app);
    cfg.console_url = Some(url.to_string());
    let _ = write_config(app, &cfg);
}

fn clear_console_url(app: &tauri::AppHandle) {
    let mut cfg = read_config(app);
    cfg.console_url = None;
    let _ = write_config(app, &cfg);
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
    read_config(webview.app_handle()).server_url
}

#[tauri::command]
fn set_server_url(webview: WebviewWindow, url: String) -> Result<(), String> {
    ensure_setup(&webview)?;
    let app = webview.app_handle();
    let mut cfg = read_config(app);
    cfg.server_url = Some(url);
    write_config(app, &cfg)
}

/// Native OIDC sign-in via the system auth session; returns the web-session
/// exchange URL to open in the app window. See `auth`.
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

    // Cross-platform system auth session (see `auth_session`).
    let callback = auth_session::authenticate(
        webview.app_handle(),
        state.inner(),
        &authorize_url,
        "ai.prisme.app",
    )
    .await?;

    let (code, returned_state) =
        auth::parse_callback(&callback).ok_or("Malformed authorization callback.")?;
    if returned_state != expected_state {
        return Err("State mismatch — sign-in aborted for safety.".into());
    }

    let (access_token, _refresh_token) = auth::exchange(
        &client,
        &token_endpoint,
        &code,
        &verifier,
        &boot.client_id,
        &boot.api_url,
    )
    .await?;

    // Turn the Bearer into an httpOnly session cookie for the webview: mint a
    // single-use ticket and return its exchange URL. The token never touches JS.
    let exchange_url =
        auth::web_session_url(&client, &boot.api_url, &access_token, &boot.console_url).await?;

    // Remember the console URL so the next launch can reuse the session.
    set_console_url(webview.app_handle(), &boot.console_url);

    Ok(auth::SignInResult { exchange_url })
}

/// Abort an in-flight sign-in (baseline path): dropping the pending callback
/// sender makes the awaiting `sign_in` resolve with the cancel sentinel.
#[tauri::command]
fn cancel_sign_in(state: tauri::State<'_, auth::AuthState>) {
    let _ = state.pending.lock().unwrap().take();
}

/// Build the remote "app" window: loads `url`, injects the notification stub,
/// routes downloads to the OS Downloads folder, and returns to the connection
/// screen when the SPA logs out. Shared by the sign-in command and the
/// session-reuse path at startup.
fn build_app_window(app: &tauri::AppHandle, url: &str) -> Result<(), String> {
    let parsed = tauri::Url::parse(url).map_err(|e| e.to_string())?;
    let nav_app = app.clone();

    WebviewWindowBuilder::new(app, "app", WebviewUrl::External(parsed))
        .title("Prisme.ai")
        .inner_size(1440.0, 900.0)
        .min_inner_size(800.0, 600.0)
        .initialization_script(NOTIFICATION_SHIM)
        .on_navigation(move |url| {
            // When the SPA logs out it navigates to the web login (which can't
            // do Google SSO in a webview). Intercept it and return to the native
            // connection screen so the user re-signs in via the system session.
            let path = url.path();
            let logged_out = path.contains("/oidc/session/end")
                || path.ends_with("/signin")
                || path.ends_with("/logout");
            if logged_out {
                let app_cb = nav_app.clone();
                let _ = nav_app.run_on_main_thread(move || show_connect_window(&app_cb));
                return false; // don't load the web login inside the app window
            }
            true
        })
        .on_new_window(|url, _features| {
            // target="_blank" / window.open → open in the user's real browser,
            // never a blank in-app window.
            let href = url.as_str();
            if href.starts_with("http://") || href.starts_with("https://") {
                let _ = open::that(href);
            }
            tauri::webview::NewWindowResponse::Deny
        })
        .on_download(|webview, event| {
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
    Ok(())
}

/// Open the remote server in its own window, then close the setup window.
/// `url` is the web-session exchange URL (sets the httpOnly cookie + redirects).
#[tauri::command]
fn open_app_window(webview: WebviewWindow, url: String) -> Result<(), String> {
    ensure_setup(&webview)?;
    let app = webview.app_handle().clone();
    build_app_window(&app, &url)?;
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.close();
    }
    Ok(())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StartupState {
    mode: String,
    server: Option<String>,
}

/// Is the server reachable right now? Any HTTP response counts as reachable;
/// only a connection/timeout error means offline.
async fn server_reachable(url: &str) -> bool {
    match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(6))
        .build()
    {
        Ok(client) => client.get(url).send().await.is_ok(),
        Err(_) => false,
    }
}

/// Resolve what to show at launch, driven by the connection window:
///   - "app"     — a stored session; if the server is reachable, open the app.
///   - "offline" — a stored session but the server is unreachable.
///   - "connect" — no stored session; show the sign-in form.
#[tauri::command]
async fn resolve_startup(webview: WebviewWindow) -> Result<StartupState, String> {
    ensure_setup(&webview)?;
    let app = webview.app_handle().clone();
    match get_console_url(&app) {
        Some(url) => {
            if server_reachable(&url).await {
                let app2 = app.clone();
                let url2 = url.clone();
                let _ = app.run_on_main_thread(move || {
                    if build_app_window(&app2, &url2).is_ok() {
                        if let Some(main) = app2.get_webview_window("main") {
                            let _ = main.close();
                        }
                    }
                });
                Ok(StartupState {
                    mode: "app".into(),
                    server: Some(url),
                })
            } else {
                Ok(StartupState {
                    mode: "offline".into(),
                    server: Some(url),
                })
            }
        }
        None => Ok(StartupState {
            mode: "connect".into(),
            server: None,
        }),
    }
}

/// Return to the native connection screen: (re)create the setup window and
/// close the remote app window. Clears the stored console URL so the next
/// launch does not try to reuse the (now ended) session.
fn show_connect_window(app: &tauri::AppHandle) {
    clear_console_url(app);
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.set_focus();
    } else {
        let _ = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
            .title("Prisme.ai")
            .inner_size(520.0, 640.0)
            .resizable(false)
            .build();
    }
    if let Some(appw) = app.get_webview_window("app") {
        let _ = appw.close();
    }
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

                // Windows/Linux use the browser + deep-link baseline, so the
                // custom scheme must be registered at runtime (macOS uses
                // ASWebAuthenticationSession + Info.plist, no registration).
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = app.deep_link().register("ai.prisme.app");
                }

                let handle = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    for url in event.urls() {
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

                // Startup routing (session reuse / offline / connect) is driven
                // by the connection window via the `resolve_startup` command.

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
            resolve_startup,
            sign_in,
            cancel_sign_in,
            open_app_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
