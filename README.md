# Fizz

Fizz is the physical layer for AI agents. This repository contains a Rust/Vercel API, Supabase data model, and a small static app for sensor streams, phone pairing, chat, and alerts.

The hackathon product scope, architecture, demo flow, and parallel work assignments are in [the product plan](docs/PRODUCT_PLAN.md).

## Project layout

- `api/status.rs` — native Rust Vercel Function at `/api/status`
- `src/store/mod.rs` — server-side Supabase REST client and connection check
- `public/index.html` — static page at `/`
- `public/app.html` — account onboarding and live sensor dashboard
- `public/sensors.html` — add, configure, and remove sensors
- `public/phone.html` — phone pairing and browser sensor sharing
- `supabase/migrations/` — customer and session schema
- `Cargo.toml` — Rust package and function binary

## Local development

Install Rust and the [Vercel CLI](https://vercel.com/docs/cli), then run:

```sh
cargo check
vercel dev
```

Open `/` for the project page or `/api/status` for the JSON status response. The function is built by Vercel's Rust runtime; `cargo check` verifies its Rust code locally.

## Supabase connection

Connect an existing Supabase project to Fizz by setting these variables in the linked Vercel project for each deployment environment you use:

| Variable | Value |
| --- | --- |
| `SUPABASE_URL` | Project URL, such as `https://<project-ref>.supabase.co` |
| `SUPABASE_SERVICE_ROLE_KEY` | The project's **service_role** secret from Supabase API settings |

The service role key is only read by Rust Functions. Do not put it in `public/`, commit it, or prefix it with `NEXT_PUBLIC_`. For local development, put both values in `.env.local` (already ignored by Git) or pull the Vercel development environment with `vercel env pull .env.local --yes`.

After deployment, `GET /api/status` reports `database: "connected"` when Supabase accepts the credentials. It reports `unconfigured` or `unavailable` without exposing the project URL or key.

## Customer onboarding

Open `/app.html` to create an account. Usernames are unique, case-insensitive, and limited to 3–24 letters, numbers, and underscores; they must start with a letter. Customers choose a six digit access code and confirm it. The code is stored as a bcrypt hash in Supabase. After registration or sign-in, a random session token is stored only as a hash and sent in a 12 hour HttpOnly, Secure, SameSite=Lax cookie. Five incorrect codes lock that username for 15 minutes.

Apply the migration with `supabase db push --project-ref <project-ref>`, then set `SUPABASE_URL` and `SUPABASE_SERVICE_ROLE_KEY` in Vercel. The browser calls only Fizz's `/api/onboarding`, `/api/session`, and `/api/me` endpoints; it never receives a Supabase key.

After sign-in, new accounts choose sensor types and press **Start with sample data**. Fizz creates simulated sensors and opens the dashboard. Existing choices migrate into sensor instances. Simulated values advance when an authenticated dashboard or Sensors page requests data, at most once per 15 seconds per sensor; the browser polls while open.

## Sensor API

The Sensors page can create additional sensors, switch a sensor to API mode, rotate its key, create a phone link, and remove a sensor. API keys are scoped to one sensor, stored only as hashes, and shown once. The main routes are:

| Route | Purpose |
| --- | --- |
| `GET`, `POST`, `PATCH`, `DELETE /api/sensors` | List, create, update, remove customer sensors (session cookie) |
| `GET /api/readings?sensor_id=<uuid>&limit=100` | Recent readings for an owned sensor |
| `POST /api/ingest` | Send readings using a sensor API key |
| `GET`, `POST`, `DELETE /api/phone` | Pair and revoke phone devices; phone posts motion, voice, and location data |
| `GET`, `POST`, `PATCH`, `DELETE /api/alerts` | View and manage threshold rules |
| `POST /api/chat` | Ask Fizz about readings and propose alerts |

For API ingestion, use a unique `event_id` for each sample; retries with the same ID are idempotent. Metrics are validated against the sensor type. For example, a Temperature sensor accepts `temperature_c`:

```sh
curl -X POST https://fizz-zeta.vercel.app/api/ingest \
  -H 'Authorization: Bearer YOUR_SENSOR_API_KEY' \
  -H 'Content-Type: application/json' \
  -d '{"sensor_id":"YOUR_SENSOR_ID","event_id":"sample-001","metrics":{"temperature_c":27.4}}'
```

Phone pairing links expire after 15 minutes and can be claimed once. A claimed phone can share motion and orientation while its page is open, and can optionally share location, a short audio clip, or a transcript after separate consent. Browser support and permissions vary by phone. Owners can revoke paired phones from the Sensors page.

Alerts are evaluated in Supabase whenever a numeric reading is inserted. Chat uses Vercel AI Gateway to answer from stored customer readings and can propose an alert, which only becomes active after the customer confirms it. An AI outage does not stop ingestion or alert evaluation.

## Supabase Compute

`supabase/config.toml` enables experimental Compute. The private `fizz-worker` service lives in `supabase/compute/fizz-worker/` and runs a small Rust health server. It uses one 2 GB, 1 vCPU instance; no public URL is exposed. It is ready for future background or device work, but the Vercel app does not call it yet.

To check or redeploy it against the linked Fizz Supabase project:

```sh
npx supabase@latest compute status fizz-worker
npx supabase@latest compute push fizz-worker
```

The service responds to `GET /health` within the private Compute network. Sample streams currently tick on authenticated API reads; the Compute service is not required for them.

## Deploy

Create or link a Vercel project from this repository with the framework preset **Other** and the repository root as the root directory. Vercel detects `api/status.rs` as a Rust Function and serves `public/index.html` as a static asset. No build command or output directory is needed.

The sensor API accepts data from clients with a scoped key. No physical actuator control is implemented.
