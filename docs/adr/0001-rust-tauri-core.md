# ADR 0001: Rust core, Tauri + React desktop shell

Status: Accepted (2026-11-05)

## Decision

All customer-facing logic (packages, retrieval, evidence, agent, inference supervision, vault, editions) is Rust. The desktop UI is Tauri + React. Python is used only in the content factory, which never ships to customers. There is no Python-runtime fallback.

## Why

Single native binaries per OS, native Tantivy, no interpreter to package on an SSD. The work is integration of mature components, not new technology.
