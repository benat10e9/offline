# ADR 0010: Personalization is a layer over shared packages

Status: Accepted (2026-11-05)

## Decision

An asset registry (equipment, location, education needs) drives a resolver that selects shared, content-addressed packages into a per-customer manifest. Packages are never rebuilt per customer. The runtime stays generic; the asset registry also supplies the evidence classifier's asset and jurisdiction.

## Why

Reuse across customers controls cost; per-customer builds do not scale.
