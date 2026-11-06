# Store (Next.js)

Storefront, configurator, Stripe Checkout, customer portal, factory queue.

```sh
npm ci && npm test && npx next build
SESSION_SECRET=… KB_PROFILE_SIGNING_KEY=<kb keygen secret hex> ADMIN_TOKEN=… npx next start
```

| Variable | Purpose |
| --- | --- |
| `SESSION_SECRET` | Signs session cookies (required in production) |
| `KB_PROFILE_SIGNING_KEY` | Ed25519 secret (hex) that signs drive profiles; its public key goes in each drive's `trust/` |
| `ADMIN_TOKEN` | Bearer token for the factory endpoints `/api/admin/queue` and `/api/admin/builds/:id` |
| `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET` | Stripe test (later live) keys. Without them the store simulates payment, only outside production or with `ALLOW_SIMULATED_PAYMENTS=1` |
| `PUBLIC_URL` | Origin for Stripe return URLs |
| `STORE_DIR` | Where the JSON data file lives (default `./data`) |

Flow: configure → `POST /api/orders` → Stripe Checkout (or simulated) → webhook `checkout.session.completed`
→ order paid → signed profile v1 + queued build → factory polls `/api/admin/queue`, runs
`scripts/build-drive.sh`, reports `/api/admin/builds/:id`. Changing configuration in the portal issues
profile v2 and queues a rebuild.

`npm test` runs unit tests; `node scripts/e2e.mjs` runs the customer journey against a running store.
The `journey` workflow runs that journey, builds the drive, removes the network and asks the real model.

Data is a single JSON file: fine for test mode, replace with Postgres before real volume.
