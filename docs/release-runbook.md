# Stable Release Runbook

This is the operating checklist for shipping the first stable PortPilot release.

## Goal

Ship `v0.1.0` as a real stable GitHub Release and publish `latest.json`.

## Current Reality

- `v0.1.0-beta.1` is already published
- Windows and Linux stable builds are healthy
- macOS stable signing is healthy
- macOS stable notarization is the remaining blocker

## Required Secrets

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
- `APPLE_CERTIFICATE`
- `APPLE_CERTIFICATE_PASSWORD`
- `APPLE_ID`
- `APPLE_PASSWORD`
- `APPLE_TEAM_ID`
- `KEYCHAIN_PASSWORD`

## When stable fails

1. Check `gh run view <run-id> --log-failed`
2. Confirm whether failure is:
   - signing identity
   - Apple account / app-specific password
   - notarization
   - publish release
   - `latest.json`
3. Fix the exact failing layer only
4. Re-tag `v0.1.0` to the latest `main`
5. Watch the new run to completion

## Current macOS blocker

The latest confirmed failure is Apple notarization returning `401` with:

- `Your Apple ID has been locked`

That means the next recovery step is not another workflow change. It is:

1. unlock the Apple account
2. generate a new app-specific password
3. replace `APPLE_PASSWORD`
4. re-run `v0.1.0`

## Stable acceptance checklist

- `gh release list` shows both `v0.1.0-beta.1` and `v0.1.0`
- `v0.1.0` is not marked prerelease
- `PortPilot-v0.1.0-macOS.dmg` exists
- `PortPilot-v0.1.0-Windows.msi` exists
- `PortPilot-v0.1.0-Linux-x86_64.AppImage` exists
- `latest.json` resolves from the GitHub Releases latest download URL
- a beta build can detect `v0.1.0` as an update
