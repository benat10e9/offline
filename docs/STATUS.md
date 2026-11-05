# Status

Last updated: 2026-11-05 (session 2)

## Done

**Session 1:** signed packages, Tantivy retrieval, evidence classes, citation verifier, llama-server supervisor, `kb` CLI, fixture corpus, CI on 3 OSes (passed).

**Session 2:**
- `kb-server`: app engine (mounts verified packages from a drive, starts the model in the background, restarts it, falls back to library search), local HTTP API, strict CSP, localhost-only Host check.
- Web UI (no build step): ask/search, evidence band on every answer, clickable citations, source reader with the cited passage highlighted, equipment and state settings, dark mode, dev-model warning.
- Tauri desktop shell (`app/`): finds the drive it runs from, starts the engine in-process, stops the model on exit.
- `kb serve` (browser mode) and `kb bench` (footprint, model load time, prompt and generation speed, full-pipeline answer times per model; writes bench-report.json).
- Provisioning scripts for macOS/Linux and Windows: pinned llama.cpp b11747 with SHA-256 checks, Qwen3 4B and 8B (Apache-2.0), content and app from releases.
- Thinking disabled for Qwen3; reasoning blocks stripped by the citation verifier.
- Content factory v0 (`factory/build_package.py`) and rights catalog `catalog/sources/medical-survival.json`: MedlinePlus (NLM pages only), Ready.gov, CDC Travelers' Health, 10 U.S. military/FEMA/DoD manuals. Public domain only; exclusions recorded.
- Workflows: `app` (Mac arm64, Windows, Linux builds → app-dev release), `content` (build, sign with dev key, full verify, size report, search smoke test, model-URL check → content-dev release).
- Dev signing key: public half in `trust/dev-publisher.pub`, secret only in the `KB_DEV_SIGNING_KEY` Actions secret.
- ADRs 0011 (local server UI) and 0012 (first models). 45 Rust tests + 4 factory tests.

## Not yet verified on real hardware (by design, per Ben)

- A real model answering through the app; actual speeds; 4B vs 8B quality.
- The macOS app bundle launching from a drive (ad-hoc signed; downloaded with curl so no quarantine prompt expected).
- The content workflow's full run (first run happens on this push; check its summary for package size and smoke-test results).

## Next: session 3

1. Read the `content` and `app` workflow results; fix anything they surface.
2. Evaluation set: 50 questions (medical, water, power, shelter) with expected sources, plus unsupported-answer cases; `kb eval` command and a CI job over the real package.
3. Maps: Texas PMTiles (Planetiler) + MapLibre view in the UI + OSM place search; Valhalla routing (Windows build is the main risk).
4. Compact Wikipedia package (CC BY-SA, attribution in reader) for the Essential edition sizing.

## Later

Commerce (storefront, Stripe test mode, customizer, portal, asset registry, order → manifest → drive composer); update bundles and rollback; code signing and notarization; production key ceremony; safety evaluation before any model is approved.

## Open decisions

- Repo visibility (public today).
- 4B vs 8B for the Essential edition: decided from bench reports.
- Signing root key custody.
