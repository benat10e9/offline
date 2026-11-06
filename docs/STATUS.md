# Status

Last updated: 2026-11-05 (session 3, part 2)

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

**Session 3 (part 1):**
- `kb eval` and `evals/medical-survival.jsonl` (50 questions: 40 in scope, 10 out-of-scope/equipment/jurisdiction traps, unsafe-answer patterns). Runs report-only in the content workflow; `--strict` fails on wrongly verified out-of-scope answers or unsafe text.
- Offline maps: map packages (`maps/map.json`, PMTiles, gazetteer, fonts, icons), HTTP range serving, `/api/maps`, `/api/places`; Map view in the UI (MapLibre 5.24 CSP build, pmtiles 4.5, Protomaps basemaps 5.7.2, vendored with licenses); place search over towns, hospitals, clinics, pharmacies, fire/police stations, shelters, water, fuel and groceries.
- `maps` workflow: Texas tiles from the latest Protomaps build (go-pmtiles 1.31.2, pinned SHA-256), places from Geofabrik via osmium, fonts/icons pinned to basemaps-assets 028c18f → maps-dev release.
- Content workflow takes any catalog; `catalog/sources/wikipedia-top.json` (CC BY-SA, reference authority so it can never produce Verified). Packages over 1.9 GB are split into parts and reassembled by the provisioning scripts.
- `kb build` handles asset-only packages (maps) and mixed packages.

**Session 3 (part 2):**
- Offline driving directions in-process (ADR 0013, Valhalla dropped): `kb-routing` builds a junction graph from OSM (car profile, one-ways, largest connected component, contraction hierarchies via fast_paths) and routes with road geometry; `kb route-build`; `/api/route`; From/To buttons, route line, distance and time in the Map view. Turn restrictions not modelled; the UI says so.
- maps workflow now builds `maps/routing/` and reports build time and peak memory.
- 50 Rust tests.

**Session 3 (part 3) — first real-content results and fixes:**
- ZIM extraction was broken (libzim renamed `_get_entry_by_id`); every page was silently skipped. Fixed, sources with 0 documents now fail loudly, a test builds and reads a real ZIM. Internet Archive fallback for blocked PDFs.
- Medical/survival package: 13 sources, ~46,000 passages, 84 MB on disk, 41 MB compressed.
- First eval on real content: 38/50 class correct, 24/41 expected sources, 3 out-of-scope questions wrongly verified. Causes: exact-word relevance (missed "apply" vs "applying") and every word weighted equally ("write a list" matched "Python list comprehension").
- Fix: relevance now uses stems and rarity weights (IDF) — a passage must cover ≥55% of the question's distinctive weight; question-framing words ignored; 40 candidates retrieved instead of 20; MedlinePlus genetics and lab-test sections excluded (noise, and the source of a genetics page answering "severe bleeding").
- Texas map package with routing: 1.6 GB on disk (1.0 GB compressed): tiles ~650 MB, routing ~950 MB (3.5 M junctions, 8.5 M edges; build 2 min 39 s, 4.5 GB peak RAM in CI); 27,657 places.

**Session 3 (part 4) — retrieval quality:**
- Eval on the real medical/survival package went 38/50 → **50/50**, expected source found 24/41 → **40/41**, out-of-scope verified 3 → **0**, unsafe 0. Fixes: search on content words only (titles like "How do I…" were outranking answers), light title boost and no separate heading field, stemmed IDF-weighted coverage, and a dominant-term anchor (a passage about another drug's dose no longer counts as evidence for an amoxicillin question).
- One eval item corrected and logged (CDC covers Peru; the "capital of Peru" item was mislabeled). Results log in evals/README.md.
- The eval is now a release gate in the content workflow (`--strict`).
- `kb build` streams passages into the index (Wikipedia scale); failed content builds publish `<catalog>-failure.txt` to the release.
- `cargo run -p kb-cli --example relevance_probe -- <pkg> "<question>"` shows scores and term weights.

**Session 3 (part 5):**
- First real model run exposed: llama-server launched with relative paths from its own folder (never would have started on a drive) — fixed, with a test; `kb eval --with-model` had graded search fallback as model answers — now fails loudly; a hard-link experiment corrupted a fixture via a test — replaced with no linking.
- `model-eval` workflow: Qwen3 4B and 8B through the full pipeline on a CPU runner (correctness real, speeds not representative).
- Wikipedia top 50,000 articles built: 1.73 M passages, 7.0 GB on disk / 5.1 GB compressed with the old format.
- Package format: index stores term frequencies without positions; passages.jsonl no longer shipped (text is stored in the index). Medical package 84 MB → 31 MB with identical eval (50/50, 40/41). Projected Wikipedia ~2.6 GB.

**Session 3 (part 6):**
- Wikipedia (50,000 articles) now 1.5 GB on disk / 1.2 GB compressed (was 7.0 GB).
- Mixed libraries: up to N candidates per package; citations ordered by authority, then score. Content CI evaluates every non-medical package mounted together with the medical library (strict gate).
- Road graph unloads after 10 idle minutes.
- `kb eval` saves after every question, prints progress, `--limit`; model-eval step timeout keeps partial results.
- docs/editions.md: measured component sizes and which drive each edition needs.

**Session 3 (part 7) — first real model results (Qwen3 4B, 50 questions, CPU runner):**
- 0 unsafe patterns; 10 answers withheld for lack of valid citations, 9 of them the out-of-scope/trap questions (correct); retrieval found the right source 40/41; ~97 s per answer on CPU.
- Class 19/50 because the verifier demanded a marker on every sentence. Now a marker covers the preceding sentences in its paragraph; list items need their own; lead-ins ending in ":" are not claims. Re-checking the stored answers: 39/40 fully cited (the one flag is a genuine uncited claim).
- Real answers exposed content problems no citation check can catch: the tourniquet answer blended current guidance with older doctrine (periodic loosening) from AFH 10-644 / ATP 3-50.21; the rabies answer gave clinician-only steps from CDC's clinician guide. Fixes: per-source passage exclusions with recorded reasons (tourniquet passages of those two manuals, pending clinician review); `audience: clinician` on JTS PCC and CDC Travelers' Health, labelled in prompts, rule to name steps that need a professional, note shown to the user.
- 20-question quick set (`evals/medical-survival-quick.jsonl`, exact subset) is the model-eval default.

**Session 4 — the customer journey:**
- Signed drive profiles (spec/profile.md): issued by the store, verified by the app and `kb verify-drive`; Node-signed profiles verify in Rust (tested), any edit is rejected.
- App: Library tab (every source with license, authority, audience; medical disclaimer), "My equipment" picker and default state from the profile.
- `kb profile-sign`, `kb verify-drive` (packages full-hash, profile, model checksums, runtimes per OS).
- `scripts/build-drive.sh`: builds a drive from a profile for macOS/Windows/Linux; idempotent (re-run repairs); removes packages/models the profile no longer selects; ends with verify-drive.
- Store (`commerce/`, Next.js 16.4.2): editions, configurator (OS, model, state, equipment, grades, delivery), accounts, Stripe Checkout + webhook (needs keys) or simulated test payment, portal with build status and signed profile downloads, configuration changes → new profile version, factory queue API. Unit tests and a scripted end-to-end journey pass locally.
- `journey` workflow: store journey → drive from profile v1 → network namespace with no network → real 4B answers, search, routing → profile v2 rebuild removes the Texas map.

**Physical USB test (2026-11-06), Intel MacBook Pro, SanDisk Ultra 32 GB exFAT, Wi-Fi off:** app launched from the drive, 4B model on CPU, four answers averaging ~27 s, citations open the source passage, Austin→Houston route 163 mi / 2 h 40 min. **Passed.**
Findings fixed in this round:
- Tourniquet answer stopped at step 2 and was labelled Verified. Root cause: in the CERT manual the remaining steps are in a figure that PDF text extraction does not capture. That passage is now excluded (reason recorded) and ATP 4-02.11's complete procedure answers instead. The factory now flags any procedure that ends at a page footer (`review_truncated_procedures` in the build report).
- Procedures split across passages are joined (next passages of the same document, up to two). Deterministic completeness check: if cited sources number steps to N and the answer stops earlier, the answer cannot be Verified and the user is told to open the source.
- Prompt requires full numbered steps for procedures. 8 passages again (accuracy over speed).
- UI shows only cited sources; other passages checked are collapsed.
- `kb bench` prints progress, elapsed time and time remaining.
- `evals/procedures.jsonl`: 10 emergency procedures with required elements (`must_include`); eval reports incomplete answers.

**Measured:** desktop app + kb tool download = 9 MB (macOS arm64), 10 MB (Linux). llama.cpp b11747: 16 MB (macOS), 36 MB (Windows Vulkan).

## Not yet verified on real hardware (by design, per Ben)

- A real model answering through the app; actual speeds; 4B vs 8B quality.
- The macOS app bundle launching from a drive (ad-hoc signed; downloaded with curl so no quarantine prompt expected).
- The content workflow's full run (first run happens on this push; check its summary for package size and smoke-test results).

## Next

1. Wikipedia package: read the build result (or its failure file) and size it.
1b. Routing data is large (~950 MB, ~1 GB RAM when loaded): compress geometry and memory-map before it ships on 32 GB drives.
2. Check the Texas routing build's memory/time and route quality on real trips.
3. Drive composer: edition manifest → which packages and models go on a 32/64/128 GB drive.
4. Commerce: storefront + Stripe test mode, customizer, portal, asset registry.

## Later

Commerce (storefront, Stripe test mode, customizer, portal, asset registry, order → manifest → drive composer); update bundles and rollback; code signing and notarization; production key ceremony; safety evaluation before any model is approved.

## Open decisions

- Repo visibility (public today).
- 4B vs 8B for the Essential edition: decided from bench reports.
- Signing root key custody.
