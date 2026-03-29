# PortPilot

PortPilot is a cross-platform desktop control console for local-first GitHub repositories.

It is built for workflows like:

- import `calesthio/Crucix`
- import `koala73/worldmonitor`
- detect install / run / build / deploy commands
- generate `.env` from `.env.example`
- start or stop local services from one UI
- route projects through a unified local gateway
- ship updates through GitHub Releases and the built-in updater

## What PortPilot does

- clone a repository directly from GitHub or register an existing local repo
- infer actions for Node, Python, Rust, Go, and Docker Compose projects
- persist project definitions, action metadata, env profiles, and execution history in SQLite
- stream stdout/stderr into the app
- expose managed apps on `.localhost` routes
- check for updates and install them from GitHub Releases

## Stack

- Tauri 2
- Rust
- SQLite via `rusqlite`
- Preact + TypeScript + Vite
- GitHub Releases + `latest.json` updater feed

## Development

```bash
npm install
npm run tauri:dev
```

## Verification

```bash
npm run typecheck
npm test
npm run build
npm run tauri build -- --debug
```

## Release outputs

Primary release targets:

- macOS universal: `.dmg`, `.zip`, updater `.tar.gz`
- Windows x64: `.msi`, optional portable `.zip`
- Linux x64: `.AppImage`, `.deb`, `.rpm`

Updater targets:

- macOS: signed updater `.tar.gz`
- Windows: signed `.msi`
- Linux: signed `.AppImage`

## Auto update

PortPilot uses the Tauri updater and GitHub Releases.

- release assets are uploaded to GitHub Releases
- `latest.json` is generated in CI and attached to the release
- the desktop app checks `https://github.com/Horace-Maxwell/portpilot/releases/latest/download/latest.json`
- installed builds can download and install new versions inside the app

## GitHub Actions secrets

The release workflow expects these secrets:

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if the key is password protected
- `APPLE_CERTIFICATE`
- `APPLE_CERTIFICATE_PASSWORD`
- `APPLE_ID`
- `APPLE_PASSWORD`
- `APPLE_TEAM_ID`
- `KEYCHAIN_PASSWORD`

Windows code signing is optional for v1. macOS signing and notarization are expected for the public installer flow.
