# ADR 0011: The UI is served by a local engine server; Tauri is a thin window

Status: Accepted (2026-11-05)

## Decision

All application logic and the web UI live in `kb-server`, which serves plain HTML/CSS/JS on 127.0.0.1.
The Tauri shell only finds the drive, starts the engine in-process, opens a window on it, and stops the
model on exit. There is no Tauri IPC. React is deferred: the UI is dependency-free JavaScript with no build step.
The API accepts requests only with a localhost Host header, and every page has a strict content-security policy.

## Why

One codebase serves the desktop app, browser mode (`kb serve`) and a future appliance that phones connect to.
The UI is testable over HTTP in CI without a display. No JavaScript toolchain is needed to build or ship it.
