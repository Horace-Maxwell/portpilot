# PortPilot Product Roadmap

PortPilot should become the fastest path from "I found a repo" to "it's running locally and easy to manage."

This roadmap is based on patterns from local environment tools, developer dashboards, reverse proxies, container UIs, and dev environment products such as Portainer, Homepage, DevPod, GitHub Codespaces, DDEV, ServBay, and Local.

## P0 — Make first run effortless

- Add a `Doctor` pass before first run to detect missing runtimes, version mismatches, occupied ports, and missing system tools.
- Turn repo import into a guided flow: clone, install, env setup, run, route, open.
- Improve monorepo detection so PortPilot can pick the correct app target instead of guessing one root command.
- Read more source signals during inference: README snippets, devcontainer files, `.nvmrc`, `.node-version`, `.python-version`, Docker files, and common framework config files.
- Add one-click recovery actions for common failures such as missing deps, bad env values, and port conflicts.

## P1 — Make daily use frictionless

- Save and restore workspace sessions so a full stack can be reopened with one click.
- Support batch actions such as start all, stop all, restart unhealthy, and reopen last session.
- Improve runtime visibility with clearer phases: installing, booting, waiting for health, running, degraded, failed.
- Upgrade logs with search, error filtering, pinned warnings, and per-run history slices.
- Merge process and Compose views into one runtime surface so local apps and containers feel like one workspace.

## P1 — Make routes and previews more useful

- Auto-detect the actual served URL from logs or listeners instead of relying mainly on hints.
- Add stronger route health checks and automatic readiness polling.
- Show a single primary entrypoint for each project, with backup URLs only when needed.
- Add quick share support for previews so a local run can be shown externally with a short-lived tunnel.

## P2 — Make projects portable across teammates

- Introduce a project recipe file such as `.portpilot.json` to pin preferred actions, ports, env hints, and health checks.
- Allow teams to export and import reusable setup recipes for common repos.
- Support shareable env templates with redacted or required-field handling.
- Add richer support for devcontainer or DevPod-style workspace metadata when repos already describe how they should boot.

## P2 — Make the platform extensible

- Add pluggable action inference providers for more ecosystems.
- Add custom health-check and log-parser hooks.
- Add template packs for common stacks like Next.js, Vite, Django, FastAPI, Tauri, and full-stack Docker apps.
- Add optional integrations for remote runtimes and cloud workspaces without changing the local-first core.

## Proposed New Types

- `DoctorCheck`
- `WorkspaceSession`
- `ProjectRecipe`
- `DetectedAppTarget`
- `TunnelSession`

## Product Standard

- Every new feature should remove at least one repeated manual step.
- Every core workflow should stay viable on macOS, Windows, and Linux.
- Import success and runtime clarity matter more than feature count.
