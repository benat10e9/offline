# Knowledge package format v1

A package is a directory:

```
<package>/
  manifest.json      signed metadata (below)
  manifest.sig       "ed25519 <public-key-hex> <signature-hex>"
  passages.jsonl     one Passage per line (source text, for the reader view)
  index/             Tantivy index over the passages (read-only at runtime)
  ...                future: tiles/, routing/, gazetteer/, vectors/
```

## manifest.json

| Field | Meaning |
| --- | --- |
| `format_version` | `1` |
| `id`, `version` | Package identity. `id@version` is what other manifests reference. |
| `title` | Human-readable name |
| `editions` | Editions allowed to mount it: `consumer`, `police`, `military`, `ems` |
| `depends_on` | `id@version` of required packages |
| `sources` | One `SourceRecord` per source document (below) |
| `files` | Every file except the manifest and signature: `path` (forward slashes), `sha256`, `bytes` |

`SourceRecord`: `id`, `title`, `publisher`, `license` (SPDX or `LicenseRef-*`), `authority`
(`official` · `government` · `reference` · `community`), `revision`, `superseded_by` (source id),
`applies_to` (exact equipment as `make:model`, lowercase), `jurisdictions` (e.g. `US`, `US-TX`), `url`.

## Passage

`passage_id` (unique in the package, stable across versions while text is unchanged), `doc_id`,
`source_id` (must exist in `sources`), `title`, `heading`, `text`. Citation key: `<package id>/<passage_id>`.

## Signing and verification

- The signed bytes are exactly the bytes of `manifest.json` as written by `kb build` (pretty JSON, files sorted by path).
- Launch: signature check only. Install, update, repair: signature plus every file's size and SHA-256, plus no unlisted files.
- Paths must be relative, forward-slash, with no `..`, `.`, or backslashes.
- Verification never reads the clock or the network.

## Not yet specified (planned)

Update bundles and rollback, revocation list, key hierarchy (offline root → edition keys),
map/routing/gazetteer payloads, optional dense vectors.
