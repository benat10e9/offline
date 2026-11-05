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
