//! Prisme.ai desktop shell (Tauri POC) — Rust side.
//!
//! Mirrors the Electron POC: a THIN SHELL that loads the SPA served by the
//! customer's own self-hosted server. This file provides:
//!   - server URL persistence (config.json under the OS app-config dir)
//!   - local filesystem read/write commands (the desktop-only capability the
//!     web app cannot do on its own) — demonstrated from the setup screen.
//!
//! The window that loads the REMOTE server URL is created at runtime with a
//! label that is NOT listed in any capability, so remote content gets no access
//! to these commands (Tauri's deny-by-default ACL) — same isolation as the
//! Electron build's "no preload on remote content".

use std::fs;
use std::path::{Path, PathBuf};
use tauri::Manager;

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Config {
    server_url: Option<String>,
}

#[derive(serde::Serialize)]
struct FileInfo {
    name: String,
    bytes: u64,
    path: String,
}

fn config_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("config.json"))
}

/// Return the last-used server URL, if any.
#[tauri::command]
fn get_server_url(app: tauri::AppHandle) -> Option<String> {
    let path = config_path(&app).ok()?;
    let data = fs::read_to_string(path).ok()?;
    let cfg: Config = serde_json::from_str(&data).ok()?;
    cfg.server_url
}

/// Persist the server URL for next launches.
#[tauri::command]
fn set_server_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    let path = config_path(&app)?;
    let cfg = Config {
        server_url: Some(url),
    };
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

/// Read a local file the user picked and return its name + size. Proves the
/// app can READ arbitrary local documents (subject to OS permissions).
#[tauri::command]
fn read_file_info(path: String) -> Result<FileInfo, String> {
    let data = fs::read(&path).map_err(|e| e.to_string())?;
    let name = Path::new(&path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    Ok(FileInfo {
        name,
        bytes: data.len() as u64,
        path,
    })
}

/// Write text to a local path the user chose. Proves the app can WRITE locally.
#[tauri::command]
fn write_text_file(path: String, contents: String) -> Result<(), String> {
    fs::write(&path, contents).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_server_url,
            set_server_url,
            read_file_info,
            write_text_file
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
