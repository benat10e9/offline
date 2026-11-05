# Offline knowledge platform

A fully offline AI knowledge system that runs from an external SSD on Windows, macOS and Linux.
A local model answers only from signed knowledge packages, cites the exact passages it used,
and labels every answer **verified**, **partially supported** or **unsupported**.

Proprietary. All rights reserved.

## Start here

- [`docs/STATUS.md`](docs/STATUS.md) — what works, what's next. Every build session starts by reading it.
- [`docs/adr/`](docs/adr) — locked architecture decisions.
- [`spec/`](spec) — package format, evidence classes, SSD layout.

## Layout

| Path | What | Ships? |
| --- | --- | --- |
| `runtime/packages` | Manifests, Ed25519 signing, offline verification | Yes |
| `runtime/retrieval` | Tantivy index build and multi-package search | Yes |
| `runtime/evidence` | Verified / partial / unsupported classification | Yes |
| `runtime/agent` | Prompting, llama-server client, citation verifier | Yes |
| `runtime/inference` | llama-server supervisor, hardware tiers | Yes |
| `runtime/server` | App engine, local HTTP API and web UI, benchmark | Yes |
| `runtime/cli` | `kb` command: keygen, build, verify, search, ask, serve, bench, tier | Yes |
| `app/` | Tauri desktop shell (Mac, Windows, Linux) | Yes |
| `scripts/` | Drive provisioning for macOS/Linux and Windows | Tooling |
| `trust/` | Development publisher public key | Yes (dev drives) |
| `fixtures/` | Fictional test corpus | No |
| `factory/` | Content pipeline (Python): fetch, extract, split, rights report | No |
| `catalog/` | Rights catalogs per package, model catalog example | Data |

## Set up a test drive

```sh
scripts/provision.sh ~/OfflineLibrary          # macOS or Linux; Windows: scripts\provision.ps1 E:\
open ~/OfflineLibrary/Offline\ Library.app
~/OfflineLibrary/bin/macos-aarch64/kb bench --root ~/OfflineLibrary --allow-unevaluated
```

Builds come from the `app-dev` and `content-dev` releases, produced by the `app` and `content` workflows.

## Try it

```sh
cargo test --workspace
cargo run -p kb-cli -- keygen --out /tmp/pub
cargo run -p kb-cli -- build --src fixtures/acme-demo --out /tmp/pkg --key /tmp/pub.key
cargo run -p kb-cli -- search --pkg /tmp/pkg --trust /tmp/pub.pub --asset acme:gx-2000 "How often do I change the oil?"
# with a llama-server already running on :8080:
cargo run -p kb-cli -- ask --pkg /tmp/pkg --trust /tmp/pub.pub --model-url http://127.0.0.1:8080 "Can I run a generator in the garage?"
```
