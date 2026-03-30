[简体中文](./README.zh-CN.md) | **English**

<div align="center">
  <h1>PortPilot</h1>
  <p><strong>The desktop control plane for local-first GitHub repos.</strong></p>
  <p>Import a repo, fill env values, run the right command, route it to a clean <code>.localhost</code> URL, and keep logs, ports, and health in one place.</p>

  <p>
    <a href="https://github.com/Horace-Maxwell/portpilot/releases/tag/v0.1.0-beta.1">
      <img src="https://img.shields.io/badge/Download-Beta-0f172a?style=for-the-badge&logo=github" alt="Download Beta" />
    </a>
    <a href="https://github.com/Horace-Maxwell/portpilot/releases">
      <img src="https://img.shields.io/badge/Browse-Releases-1d4ed8?style=for-the-badge&logo=github" alt="Browse Releases" />
    </a>
    <a href="./README.zh-CN.md">
      <img src="https://img.shields.io/badge/Read%20in-Chinese-0f766e?style=for-the-badge" alt="Read in Chinese" />
    </a>
  </p>

  <p>
    <img src="https://img.shields.io/github/v/release/Horace-Maxwell/portpilot?include_prereleases&display_name=tag&style=for-the-badge" alt="Release" />
    <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-0f766e?style=for-the-badge" alt="Platforms" />
    <img src="https://img.shields.io/badge/Tauri-2.x-24C8DB?style=for-the-badge&logo=tauri" alt="Tauri" />
    <img src="https://img.shields.io/badge/Rust%20%2B%20TypeScript-core%20stack-111827?style=for-the-badge&logo=rust" alt="Rust and TypeScript" />
    <img src="https://img.shields.io/github/license/Horace-Maxwell/portpilot?style=for-the-badge" alt="License" />
  </p>
</div>

![PortPilot hero](./docs/media/hero-banner.svg)

## Why PortPilot

| Import once | Control from one place | Observe without guesswork |
| --- | --- | --- |
| Turn GitHub URLs and local folders into runnable project profiles. | Start, stop, build, deploy, and route repos without juggling terminals. | Keep routes, logs, ports, and runtime status inside one desktop surface. |

## Works With Real Repos

PortPilot is built around the kinds of projects people actually clone and try to run:

| Repository | What PortPilot surfaces |
| --- | --- |
| [`calesthio/Crucix`](https://github.com/calesthio/Crucix) | `npm install`, `npm run dev`, env generation, compose actions, unified route opening |
| [`koala73/worldmonitor`](https://github.com/koala73/worldmonitor) | web run targets, `desktop:dev`, `build:*`, `desktop:build:*`, local runtime visibility |

## Product Preview

### One dashboard, multiple repos

![Dashboard preview](./docs/media/dashboard-preview.png)

### Action-centric project pages

![Actions preview](./docs/media/actions-preview.png)

### Routes, logs, and ports in one place

![Observability preview](./docs/media/observability-preview.png)

## Feature Map

| Area | What you get |
| --- | --- |
| Import | Clone from GitHub URL or register an existing local project |
| Detection | Infer actions for Node, Python, Rust, Go, and Docker Compose |
| Environment | Parse `.env.example` and save editable env profiles |
| Runtime | Start, stop, restart, build, deploy, and watch execution history |
| Routing | Map projects onto clean `.localhost` URLs through a local gateway |
| Observability | View logs, ports, status, and route bindings inside one UI |
| Updates | Release through GitHub Releases and prepare in-app updater flow |

## Quick Start

```bash
npm install
npm run tauri:dev
```

Then:

1. Add or confirm your workspace root.
2. Paste a GitHub URL like `https://github.com/calesthio/Crucix.git`.
3. Import the repo, review inferred actions, save env values, and hit `Run`.

## Downloads

PortPilot is available now as a public GitHub beta release.

- Go to [Releases](https://github.com/Horace-Maxwell/portpilot/releases)
- Download the package for your platform: macOS `.dmg`, Windows `.msi`, or Linux `.AppImage`
- On the first macOS beta launch, you may need to manually allow the app in System Settings because the beta build is not notarized yet

### Beta asset set

- macOS: `.dmg`, `.zip`, updater `.tar.gz`
- Windows x64: `.msi`, optional `.zip`
- Linux x64: `.AppImage`, `.deb`, `.rpm`

## Beta Notes

- `v0.1.0-beta.1` is intended as the first public validation build
- macOS beta packages may show Gatekeeper warnings before the notarized channel is ready
- Windows and Linux packages are beta-quality validation builds
- stable in-app auto-update is reserved for the signed/notarized stable channel

## Roadmap

- P0: guided first-run flow, Doctor checks, stronger monorepo detection, and one-click recovery actions
- P1: session restore, batch actions, better runtime states, richer logs, and stronger route health
- P1: deeper Docker / Compose orchestration with one unified runtime surface
- P2: shareable project recipes, preview tunnels, and extensible stack-specific inference

Already landing in the app:
- first-run setup checklist with install / env / run / open guidance
- Doctor checks for tooling, env gaps, install state, routes, and port readiness
- monorepo target detection for workspace-based Node repos

## Development

```bash
npm run typecheck
npm test
npm run build
npm run tauri build -- --debug
```

## Contributing

Issues and PRs are welcome, especially around repo detection, action inference, runtime orchestration, and cross-platform packaging.

## License

MIT. See [LICENSE](./LICENSE).
