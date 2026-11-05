# ADR 0013: Routing runs in-process on a prebuilt road graph; Valhalla deferred

Status: Accepted (2026-11-05). Supersedes the routing part of the architecture evaluation.

## Decision

Driving and walking routes are computed inside the Rust engine with contraction hierarchies (`fast_paths`,
MIT/Apache-2.0) over a road graph built once per region in CI from OpenStreetMap and shipped inside the map package.
No routing binary ships. v0 ignores turn restrictions and time-dependent rules and says so in the UI.

## Why

Valhalla has no official Windows builds; shipping it portably on three operating systems means bundling a C++
dependency tree per platform, the fragile packaging this product avoids. An in-process router adds one Rust crate,
works identically on every platform, and reuses the signed-package pipeline. Valhalla stays an option for a premium
or agency edition that needs turn restrictions, truck profiles or isochrones.
