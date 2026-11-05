# ADR 0005: Ed25519-signed packages, verified without clock or network

Status: Accepted (2026-11-05)

## Decision

Packages are directories with manifest.json signed by a publisher key. Launch verifies the signature; install, update and repair verify every file's SHA-256. Verification never depends on the current date.

## Why

Offline machines have unreliable clocks; integrity must hold with no connectivity.
