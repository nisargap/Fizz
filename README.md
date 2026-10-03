# Fizz

Fizz is the physical layer for AI agents. This repository starts with a small Rust API and a static project page, ready to deploy on Vercel.

The hackathon product scope, architecture, demo flow, and parallel work assignments are in [the product plan](docs/PRODUCT_PLAN.md).

## Project layout

- `api/status.rs` — native Rust Vercel Function at `/api/status`
- `src/store/mod.rs` — server-side Supabase REST client and connection check
- `public/index.html` — static page at `/`
- `public/app.html` — customer onboarding and sign-in wizard
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

After deployment, `GET /api/status` reports `database: "connected"` when Supabase accepts the credentials. It reports `unconfigured` or `unavailable` without exposing the project URL or key. This checks connectivity; device tables and the rest of the product plan are still to be implemented.

## Customer onboarding

Open `/app.html` to create an account. Usernames are unique, case-insensitive, and limited to 3–24 letters, numbers, and underscores; they must start with a letter. Customers choose a six digit access code and confirm it. The code is stored as a bcrypt hash in Supabase. After registration or sign-in, a random session token is stored only as a hash and sent in a 12 hour HttpOnly, Secure, SameSite=Lax cookie. Five incorrect codes lock that username for 15 minutes.

Apply the migration with `supabase db push --project-ref <project-ref>`, then set `SUPABASE_URL` and `SUPABASE_SERVICE_ROLE_KEY` in Vercel. The browser calls only Fizz's `/api/onboarding`, `/api/session`, and `/api/me` endpoints; it never receives a Supabase key.

## Supabase Compute

`supabase/config.toml` enables experimental Compute. The private `fizz-worker` service lives in `supabase/compute/fizz-worker/` and runs a small Rust health server. It uses one 2 GB, 1 vCPU instance; no public URL is exposed. It is ready for future background or device work, but the Vercel app does not call it yet.

To check or redeploy it against the linked Fizz Supabase project:

```sh
npx supabase@latest compute status fizz-worker
npx supabase@latest compute push fizz-worker
```

The service responds to `GET /health` within the private Compute network.

## Deploy

Create or link a Vercel project from this repository with the framework preset **Other** and the repository root as the root directory. Vercel detects `api/status.rs` as a Rust Function and serves `public/index.html` as a static asset. No build command or output directory is needed.

This is a foundation for Fizz; no device control or agent protocol is implemented yet.
