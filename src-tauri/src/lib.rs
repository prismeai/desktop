//! Prisme.ai desktop — native layer.
//!
//! Thin shell that loads the SPA served by the customer's own self-hosted
//! server. This file wires the native features a web page can't get on its own
//! inside a system WebView:
//!   - server URL persistence
//!   - the remote "app" window, created here so we can attach:
//!       * a no-op Notification stub (WKWebView has no web Notification API)
//!       * a download handler (saves to the OS Downloads folder)
//!   - deep links (prisme://…) + single-instance
//!
//! Security: sensitive custom commands are guarded to the trusted "main"
//! window. The remote "app" window is granted no IPC capability, so remote
//! content (including third-party auth pages) cannot reach any command.

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

/// Open the remote server in its own window (native notifications + downloads),
/// then close the setup window. Created from Rust so we can attach handlers.
#[tauri::command]
fn open_app_window(webview: WebviewWindow, url: String) -> Result<(), String> {
    ensure_setup(&webview)?;
    let parsed = tauri::Url::parse(&url).map_err(|e| e.to_string())?;
    let app = webview.app_handle().clone();

    WebviewWindowBuilder::new(&app, "app", WebviewUrl::External(parsed))
        .title("Prisme.ai")
        .inner_size(1440.0, 900.0)
        .min_inner_size(800.0, 600.0)
        .initialization_script(NOTIFICATION_SHIM)
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
                app.deep_link().on_open_url(move |_event| {
                    if let Some(w) = handle
                        .get_webview_window("app")
                        .or_else(|| handle.get_webview_window("main"))
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
            open_app_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
