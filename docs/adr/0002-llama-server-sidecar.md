# ADR 0002: llama.cpp runs as a supervised llama-server process

Status: Accepted (2026-11-05)

## Decision

The model runs in llama.cpp's `llama-server`, launched and health-checked by `kb-inference`, bound to 127.0.0.1 only. It is never linked into the app. One build per backend (Metal, CUDA, Vulkan, CPU) ships per platform; the supervisor tries them best-first and falls back.

## Why

Crash and out-of-memory isolation; hardware-specific builds without rebuilding the app; no service installation.
