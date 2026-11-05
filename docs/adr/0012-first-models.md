# ADR 0012: First models are Qwen3 4B and 8B (Apache-2.0), unevaluated

Status: Accepted for development (2026-11-05)

## Decision

Development drives carry Qwen3-4B and Qwen3-8B as 4-bit GGUF (Q4_K_M), from the Qwen organisation on Hugging Face,
under Apache-2.0. Both are `safety_approved: false` until the safety evaluation passes. Drives provisioned for
development carry a `DEV_ALLOW_UNEVALUATED` marker; the app then allows them and shows a warning on every screen.
Thinking mode is disabled (`enable_thinking: false`), reasoning text is extracted by llama-server and also stripped
by the citation verifier. llama.cpp is pinned to build b11747 (2026-10-19) with SHA-256 checksums.

## Why

Apache-2.0 has no field-of-use limits (military, police). 4B vs 8B is the key cost/compatibility question for the
consumer edition and is decided by `kb bench` on real machines. Thinking output would be uncited text.
