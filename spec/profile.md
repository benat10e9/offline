# Drive profile v1

The signed link between a store order and the drive the factory builds. Issued by the store
on payment (version 1) and on every configuration change (version 2, 3, …).

Files at the drive root: `profile.json` and `profile.sig`. The signature covers the exact bytes
of `profile.json` (Ed25519, line format `ed25519 <public key hex> <signature hex>`), checked
against the keys in `trust/`. A profile that fails verification is ignored by the app and
reported; `kb verify-drive` fails.

| Field | Meaning |
| --- | --- |
| `format` | `1` |
| `profile_id`, `order_id` | The order |
| `version` | Increments on each configuration change |
| `edition` | `essential`, `standard`, `family` |
| `os` | Runtimes to install: `macos`, `windows`, `linux` |
| `model` | Model catalog id, e.g. `qwen3-4b-q4km` |
| `packages` | Package ids to install; others are removed on rebuild |
| `jurisdiction` | Default state for jurisdiction checks, e.g. `US-TX` |
| `assets` | Customer equipment `{label, make, model, year}`; the app offers these as the question context (`make:model`) |
| `requests` | Things the customer asked for that no package provides yet (map regions, equipment manuals, curricula) |

Producers: `commerce/lib/profile.mjs` (store), `kb profile-sign` (factory/manual).
Consumers: `scripts/build-drive.sh`, `kb verify-drive`, the app (`/api/profile`).
