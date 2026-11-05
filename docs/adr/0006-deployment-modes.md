# ADR 0006: Portable and installed deployment modes

Status: Accepted (2026-11-05)

## Decision

Two supported modes. Portable: everything runs from the SSD, nothing written to the host (consumer default). Installed: app installed to a fixed, signed path for managed or government machines (agency default). Portable execution is a compatibility target, not a guarantee on locked-down endpoints.

## Why

Managed endpoints commonly block unsigned or removable-media executables.
