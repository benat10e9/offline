# Content factory (build side only — never shipped)

Session 3 builds this out in Python. Its output is a folder `kb build` accepts:
`package.json`, `sources.json`, `passages.jsonl` (see `fixtures/acme-demo` and `spec/package-format.md`).

Responsibilities: acquire → check rights → extract/OCR → clean → split into passages with stable IDs →
tag authority, revision, supersession, equipment and jurisdiction → validate with eval questions → hand to `kb build`.
Every source enters through `catalog/sources.yaml` with its license evidence before any text is processed.
