# Evaluation sets

`medical-survival.jsonl` — 50 questions for the medical/survival package (40 in scope, 10 out of scope or
equipment/jurisdiction traps). Each line:

| Field | Meaning |
| --- | --- |
| `expect_class` | Acceptable evidence classes |
| `expect_sources` | At least one of these source ids should be among the citable passages (empty = no expectation) |
| `must_not` | Regexes that must not appear in a generated answer (unsafe or fabricated guidance) |
| `asset`, `jurisdiction` | Question context, as the asset registry would supply it |

Run: `kb eval --root <drive> --questions evals/medical-survival.jsonl` (retrieval only), add `--with-model` to
grade generated answers. Expected sources were written from the catalog, not yet checked against the built package;
the first run calibrates them. A passing eval is a release gate, not proof of safety.

`medical-survival-quick.jsonl` — 20 of those 50 for model runs on slow hardware: 15 high-stakes in-scope questions
(tourniquet, shock, burns, heat, hypothermia, bleach, generator CO, food after outage, floods, rescue breathing,
snakebite, chest wound, rabies, insulin, triage) and 5 traps (Python, an uninstalled generator manual, Texas
handgun law, child antibiotic dosing, adult acetaminophen). Same items and expectations, not new questions.

`procedures.jsonl` — 10 step-by-step emergency procedures (tourniquet, bleach, boiling, burns, shock, heat stroke,
hypothermia, carbon monoxide, choking, snakebite). `must_include` lists the critical elements an answer must
contain (from CDC, FEMA, Red Cross-aligned U.S. government guidance in the library); a missing element is reported as
an incomplete answer. Added after the physical USB test, where a tourniquet answer stopped at step 2 of the procedure.
These patterns are a screen, not a clinical review.

## Changes

- 2026-11-05: `oos-01` changed from "What is the capital of Peru?" to a literature question. CDC Travelers'
  Health has Peru destination pages, so the original was answerable from the library and was mislabeled as out of scope.
  No other expectation has been changed to make a result pass.

## Results log

| Date | Package build | Class correct | Expected source found | Out-of-scope verified | Notes |
| --- | --- | --- | --- | --- | --- |
| 2026-11-05 | first real build | 38/50 | 24/41 | 3 | exact-word relevance, filler words searched, genetics pages crowding results |
| 2026-11-05 | same build, new retrieval | 49/50 | 40/41 | 1 | stems + IDF coverage, dominant-term anchor, content-word search, light title boost |
