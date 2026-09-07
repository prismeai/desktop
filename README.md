# Prisme.ai Desktop

Native desktop application for **macOS and Windows**. It connects to your
Prisme.ai server and gives you the full platform in a dedicated app window,
with native notifications, downloads, voice input and automatic updates.

## How it works

On first launch, enter the URL of your Prisme.ai server
(e.g. `https://prisme.your-company.com`). The app loads the platform from that
server and remembers the URL for next launches. Because the interface is served
by your own server, the app always stays in sync with your platform version.

Sign-in works exactly as it does in the browser (OIDC + secure session).

## Features

- Connect to any Prisme.ai server by URL
- Native desktop notifications
- File downloads saved to your system Downloads folder
- Microphone access for voice input
- Deep links (`prisme://`)
- Automatic, signed updates

## Requirements

- [Node.js](https://nodejs.org) 22
- [Rust](https://www.rust-lang.org/tools/install) (stable toolchain)

## Development

```bash
npm install
npm run tauri dev
```

## Building installers

```bash
npm run tauri build
```

Produces a `.dmg`/`.app` on macOS and a `.msi`/`.exe` on Windows under
`src-tauri/target/release/bundle/`. Signed release builds and the update feed
are produced by CI (see `.github/workflows/release.yml`).

## Security

The app window that loads your server runs sandboxed and isolated: remote
content is granted a single, explicit capability (native notifications) and
cannot reach the filesystem or other system APIs. Privileged operations are
restricted to the local connection screen.

## Project layout

```
src/            Connection screen (Vite + TypeScript)
src-tauri/      Native layer (Rust): windows, notifications, downloads,
                deep links, updater, app icons
```

## License

MIT — see [LICENSE](./LICENSE).
