# Prisme.ai Desktop (Tauri POC)

Standalone desktop shell for the Prisme.ai platform (**macOS & Windows**), built
with **Tauri v2**. A **thin native wrapper** that connects to a customer's
**self-hosted Prisme.ai server URL** and loads the web app served by that server.

> This is the **Tauri** counterpart to the Electron POC (which lived in the
> platform monorepo under `services/platform-desktop`). Kept as its **own repo**
> on purpose — a desktop client is a distinct, distributable product with its
> own release cadence, signing/store pipeline and macOS+Windows CI. This is how
> Mattermost ships its desktop client (`mattermost/desktop`, separate from the
> server/webapp repo).

## Why thin shell

Prisme.ai is sold self-hosted: every customer runs their own server on their own
release cadence. The shell **loads the SPA from the customer's server** (they
enter `https://prisme.their-company.com` on first launch) instead of bundling
it, so the UI is always in sync with their backend and **one installer works for
every customer, whatever their version**. Auth (OIDC + httpOnly cookie) is
same-domain navigation inside the window, so login works like in a browser.

## Architecture

```
src/                 setup screen (Vite + vanilla TS) — the trusted local UI
  main.ts            connect logic + local-filesystem demo
  styles.css
index.html
src-tauri/           Rust side
  src/lib.rs         commands: server URL persistence + local read/write
  tauri.conf.json    product name, window, bundle/icons
  capabilities/      ACL — granted to the "main" (setup) window ONLY
  icons/             app icons generated from the Prisme emblem
```

**Security model.** On connect, the app opens a **second window** (label `app`)
pointed at the remote server URL and closes the setup window. That remote window
is **not listed in any capability**, so under Tauri's deny-by-default ACL the
remote content gets **no access to any Rust command or plugin** — the same
isolation as the Electron build's "no preload on remote content".

## Local filesystem access (desktop-only capability)

The setup screen has two buttons proving what a desktop app can do that the web
app cannot:

- **Read a document…** — native file picker → reads the file locally, shows its
  size (`read_file_info` in Rust, `std::fs`).
- **Write a test file…** — native save dialog → writes a file to disk
  (`write_text_file`).

These run in the trusted local window. To let the **remote** SPA use them, you
would explicitly grant that origin a capability and expose a narrow, audited API
(never raw fs) — a deliberate, reviewable decision (good fit for banking). Note
that basic file **upload** (`<input type=file>`) and **download** already work in
the remote webview with no native code.

## Run locally

Requires Node 22 and the Rust toolchain (`rustup`). Then:

```bash
npm install
npm run tauri dev      # launches the app (compiles Rust on first run)
```

Enter a server URL (e.g. `https://studio.prisme.ai`) and sign in as usual.

Build installers (unsigned):

```bash
npm run tauri build    # .dmg/.app (macOS), .msi/.exe (Windows)
```

## Tauri vs Electron (why this POC exists)

| | Electron POC | **Tauri v2 POC (this repo)** |
|---|---|---|
| Runtime | Bundles Chromium + Node (~150 MB) | Native OS WebView, Rust core (~5–10 MB) |
| RAM | Higher | ~1/3 of Electron |
| Backend language | TypeScript (main process) | Rust (commands) |
| Security ACL | Manual (preload isolation) | Capability system, deny-by-default |
| Toolchain | Node only | Node + Rust |

Both implement the exact same thin-shell topology; the app behaviour is
identical. Run `npm run tauri build` in each to compare real installer sizes.

## Production TODO

- Code signing + notarization (Apple Developer ID, Windows EV cert).
- Auto-update (`tauri-plugin-updater` + release feed).
- Native polish: notifications, tray, deep links, single-instance.
- External-IdP auth (RFC 8252 system-browser flow) if customers federate to
  Azure AD / Okta that block embedded webviews.
