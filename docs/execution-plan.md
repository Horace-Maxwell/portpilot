# PortPilot Execution Plan

This document turns the current product roadmap into implementation phases that match the existing codebase.

## Phase 0 — Ship `v0.1.0` stable

**Goal:** publish the first stable release and make `latest.json` live.

### Current blocker

- macOS signing succeeds
- macOS notarization fails with Apple `401`
- Windows and Linux builds are already healthy

### Work items

- Restore Apple account access and replace `APPLE_PASSWORD`
- Re-run the `v0.1.0` tag until the GitHub Release exists
- Verify `latest.json` and updater detection from `v0.1.0-beta.1`

### Acceptance

- `PortPilot v0.1.0` exists as a non-prerelease GitHub Release
- `latest.json` is published
- macOS `.dmg`, Windows `.msi`, Linux `.AppImage` are all available

## Phase 1 — First-run success

**Goal:** make imported repos much more likely to run on the first attempt.

### Backend

- Extend `src-tauri/src/core/inference.rs` to read README snippets, version files, devcontainer files, and more framework config
- Add a `DoctorCheck` model to `src-tauri/src/core/models.rs`
- Add doctor commands in `src-tauri/src/lib.rs`
- Add runtime preflight checks in `src-tauri/src/runtime/manager.rs`

### Frontend

- Add a `Setup / Doctor` section to project detail screens
- Add a guided first-run flow after Git import
- Surface one-click fixes for missing runtimes, ports, and env values

### Acceptance

- Imported repos show setup blockers before run
- Common failures point to a concrete fix instead of only raw logs
- Monorepos can select a sub-app target before launch

## Phase 2 — Daily workflow acceleration

**Goal:** reduce repetitive local workflow steps.

### Backend

- Add `WorkspaceSession` storage in `src-tauri/src/storage/store.rs`
- Add richer `RunPhase` states in runtime execution records
- Add health-check detection from stdout and listening ports

### Frontend

- Add session save/restore
- Add batch actions: start all, stop all, restart failed
- Add log filters, error pinning, and better execution history grouping

### Acceptance

- Users can restore a working stack in one click
- Projects show a clear phase like installing, booting, ready, or failed
- Logs are searchable and easier to scan

## Phase 3 — Unified runtime and Docker view

**Goal:** make local processes and Compose services feel like one system.

### Backend

- Extend project/runtime models with service-level Compose metadata
- Track containers and process targets under one runtime surface
- Add recommended Docker actions for repos with Docker assets but weak scripts

### Frontend

- Add a `Runtime` page with local processes and Compose services together
- Show service trees, port mappings, and service-specific logs
- Allow targeted restart/stop actions per service

### Acceptance

- Docker Compose projects are inspectable without leaving PortPilot
- Users can tell which port or log line belongs to which service

## Phase 4 — Shareable projects and team portability

**Goal:** make setup portable between teammates and between machines.

### Backend

- Add `ProjectRecipe` support for repo-level overrides
- Support import/export of saved project recipes
- Add `TunnelSession` groundwork for preview sharing

### Frontend

- Add recipe import/export actions
- Add a share-preview action when tunnels are enabled
- Show repo-level overrides separately from inferred values

### Acceptance

- Teams can reuse a repo setup with fewer manual confirmations
- Shared preview links work as an optional layer on top of local routes

## Mapping To Current Modules

- `src-tauri/src/core`: inference, models, doctor inputs, recipe parsing
- `src-tauri/src/runtime`: run phases, health checks, process/service orchestration
- `src-tauri/src/storage`: sessions, recipes, richer execution persistence
- `src/lib/tauri.ts`: new typed Tauri commands
- `src/features` and `src/contexts`: doctor UI, sessions, runtime surface, recipe actions
