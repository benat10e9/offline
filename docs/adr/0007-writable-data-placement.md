# ADR 0007: Writable data placement per edition

Status: Accepted (2026-11-05)

## Decision

Large public packages live on the SSD, read-only by convention. Private data defaults to the host machine in installed mode, and to an SSD vault built for abrupt removal (append-only writes, checkpoints) in portable mode. Pull-the-plug durability tests are a release gate.

## Why

exFAT is the only filesystem all three OSes can write, and it has no journal.
