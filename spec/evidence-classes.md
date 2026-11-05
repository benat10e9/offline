# Evidence classes

Every answer carries exactly one class, decided by `kb-evidence` and `kb-agent` in code.

| Class | Meaning | Example |
| --- | --- | --- |
| **Verified** | A current, authoritative document that applies to this exact equipment or jurisdiction was retrieved and is cited. | Asset `acme:gx-2000`; Rev B service manual's oil procedure cited. |
| **Partially supported** | Relevant material exists, but not the exact, current, applicable document, or the answer has uncited statements. | Asset `cat:3208`; only general generator maintenance guidance installed. |
| **Unsupported** | Nothing installed establishes a reliable answer. The model is not called, or its answer is withheld. | "What is the capital of Peru?" against a generator library. |

## Inputs

- The question text.
- `QuestionContext` from the asset registry and user settings: `asset` (`make:model`) and `jurisdiction`. Never guessed by the model.
- Retrieved hits and the signed `SourceRecord` for each.

## Rules, in order

1. **Relevance floor.** A hit must share at least two content terms with the question (or half of them for short questions). Otherwise excluded `not_relevant`.
2. **Equipment.** A source with non-empty `applies_to` is usable only when the question's asset is in that list. Otherwise excluded `wrong_equipment`. With no asset selected, equipment-specific manuals are not applied.
3. **Jurisdiction.** If the question has a jurisdiction and the source lists jurisdictions, one must equal it or be its parent (`US` covers `US-TX`). Otherwise excluded `wrong_jurisdiction`.
4. **Supersession.** A superseded source is excluded when its replacement is among the candidates. If the replacement is not installed, it stays citable but cannot be verified, and a note tells the user to look for a newer revision.
5. **Verified bar.** With an asset: an `official` source whose `applies_to` contains it. With only a jurisdiction: `official` or `government`, matching the jurisdiction. With neither: `official` or `government`.
6. **Class.** No usable hits → unsupported. Any hit meeting the verified bar → verified. Otherwise → partial.

## After generation (`kb-agent`)

- The model sees only citable passages, numbered `[S1]…`.
- Markers not in that set, and other bracketed citation styles, are removed and counted.
- A model "Sources:" section is stripped; the app renders sources itself.
- No valid citation → answer withheld, class unsupported, consulted passages shown.
- Verified answers drop to partial if they cite no verified passage or contain uncited statements.
- Verification can lower the class, never raise it.

## Known limits (v0)

Relevance is lexical; paraphrased questions can miss. BM25 scores are merged across packages without calibration. Both are addressed with the evaluation set before broadening content.
