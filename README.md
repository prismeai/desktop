# Prisme.ai Desktop

Native desktop app for **macOS and Windows**. It connects to your self-hosted
Prisme.ai server and runs the platform in a dedicated window — with native
sign-in, downloads to your Downloads folder, session reuse across launches, and
signed automatic updates.

## How it works

The app is a **thin native shell**: it loads the web app served by *your*
Prisme.ai server, so the UI always matches your server's version. On first
launch you enter your server's **API URL** (e.g. `https://api.your-company.com`);
it's remembered for next time.

Sign-in is native and standards-based (**OAuth 2.0 + PKCE, RFC 8252**):

- **macOS** — a system authentication sheet (`ASWebAuthenticationSession`): full
  SSO (Google / Microsoft / …) and MFA, and it closes itself when done.
- **Windows** — your system browser + a secure app callback (`ai.prisme.app://`).

No token is ever exposed to the web page: after sign-in the shell establishes an
**httpOnly session cookie** in the app window (via the server's web-session
endpoint). Logging out returns you to the native connection screen.

## Features

- Connect to any self-hosted Prisme.ai server
- Native SSO + MFA sign-in via the OS auth session
- Session reuse across launches (no re-login until the session expires)
- Downloads saved to your system Downloads folder
- Signed automatic updates

## Requirements

- [Node.js](https://nodejs.org) 22
- [Rust](https://www.rust-lang.org/tools/install) (stable toolchain)

## Development

```bash
npm install
npm run tauri dev
```

## Building

```bash
npm run tauri build
```

Produces a `.dmg`/`.app` (macOS) and `.msi`/`.exe` (Windows) under
`src-tauri/target/release/bundle/`. Signed, notarized release builds and the
update feed are produced by CI on a version tag — see **[RELEASING.md](./RELEASING.md)**.

## Security

- The window that loads your server runs sandboxed/isolated with **no IPC
  capability**: remote content cannot reach the filesystem or any native command.
- Privileged commands are restricted to the local connection screen.
- Auth runs in the OS-managed session; the access token **never touches page JS**
  (httpOnly cookie).

## Platform status

- **macOS** — complete: native auth sheet, notarization-ready.
- **Windows** — functional via system browser + app callback; native broker
  (`WebAuthenticationBroker`) and code signing are being finalized (see RELEASING.md).

## Project layout

```
src/            Connection screen (Vite + TypeScript)
src-tauri/      Native layer (Rust):
  auth.rs           OIDC (bootstrap, discovery, PKCE, token, webSession)
  auth_session.rs   per-OS system auth session (macOS ASWebAuthenticationSession)
  lib.rs            windows, commands, session reuse, downloads, updater
```

## License

MIT — see [LICENSE](./LICENSE).
