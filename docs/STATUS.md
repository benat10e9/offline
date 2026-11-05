# Status

Last updated: 2026-11-05 (session 1)

## Done (session 1)

- Cargo workspace, Rust 1.91, CI on Ubuntu, macOS and Windows (fmt, clippy -D warnings, tests, release `kb` artifacts).
- `kb-packages`: manifest schema v1, Ed25519 sign/verify, full-file hash verification, path-traversal guards. No clock or network use.
- `kb-retrieval`: Tantivy index build from passages, read-only multi-package search, lookup by passage and document, free-text query sanitising.
- `kb-evidence`: deterministic verified / partial / unsupported classification (equipment, supersession, jurisdiction, authority, relevance floor).
- `kb-agent`: prompt over citable passages only, llama-server (OpenAI-compatible) client, strict citation verifier, answer withholding, class can only be lowered.
- `kb-inference`: llama-server supervisor (backend fallback, health checks, restart), RAM-based tier selection, search-only below the floor.
- `kb` CLI and a fictional fixture corpus; 37 tests including 8 end-to-end scenarios through the real binary.
- ADRs 0001-0010, specs for package format, evidence classes and SSD layout.

## Not yet verified

- `kb ask` against a real llama-server and model (sandbox has no model or GPU). First thing to run on Ben's Mac.
- CI results on macOS and Windows (first run happens on this push).

## Next: session 2 — desktop app

1. Tauri + React shell calling the runtime crates (chat, sources panel with evidence badge, reader with passage highlight).
2. Provisioning script: pinned llama.cpp release binaries per platform/backend and one Apache-2.0 GGUF model into the SSD layout.
3. CI job producing unsigned app bundles for all three OSes.
4. Choose and record the first model (license verified per checkpoint; safety eval before `safety_approved: true`).

## Later sessions

3. Content factory v0 (public-domain FEMA/CDC/USDA/Army manuals → passages with source records → signed package), `catalog/sources.yaml` rights register.
4. Maps: Texas PMTiles (Planetiler), OSM gazetteer, Valhalla routing; Windows Valhalla build is the main risk.
5. Commerce: storefront + Stripe test mode, customizer, portal, asset registry, order → manifest → SSD composer.
6. 50-question eval (accuracy, citations, unsupported handling) and offline acceptance test on real machines.

## Open decisions

- Repo visibility: currently public.
- Signing root key custody (hardware key).
- First commercial edition and buyer.
