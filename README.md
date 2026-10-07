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
- `public/agents.html` — agent tokens and device pairing
- `public/install.sh` — installer for the Fizz CLI
- `cli/` — the `fizz` device CLI (Rust)
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

Fizz is invite-only. The landing page collects emails for a waitlist through `POST /api/waitlist`, which stores lowercased addresses in `fizz_waitlist`. It answers the same way for new and duplicate addresses, accepts at most 20 sign-ups per client IP per hour and 5,000 overall per hour, and returns `429` past those limits. IPs come from Vercel's `x-real-ip` header and are stored only as SHA-256 hashes in `fizz_rate_limits`.

Businesses use the Enterprise form on the landing page, which posts to the same endpoint with `"kind": "business"`, a company name (up to 120 characters), and an optional note (up to 500). Entries go to `fizz_business_interest`, one row per email; submitting again updates the company and note. That form allows 10 submissions per client IP per hour and 1,000 overall per hour.

New accounts need a single-use invite code such as `FIZZ-ABCD-EFGH-JKMN` (60 random bits). Mint codes in the Supabase SQL editor; each plaintext code is shown only once and only its hash is stored:

```sql
select * from public.fizz_create_invites(5, 'Hackathon judges', interval '14 days');
```

Codes are case-insensitive, and dashes, spaces, and the `FIZZ` prefix are optional. A code is consumed in the same transaction that creates the account, so a failed registration leaves it unused. Registration allows 10 attempts per client IP per hour. Existing accounts sign in as before. To see who has used a code, query `fizz_invite_codes.used_by`.

Open `/app.html` with an invite code to create an account. Usernames are unique, case-insensitive, and limited to 3–24 letters, numbers, and underscores; they must start with a letter. Customers choose a six digit access code and confirm it. The code is stored as a bcrypt hash in Supabase. After registration or sign-in, a random session token is stored only as a hash and sent in a 12 hour HttpOnly, Secure, SameSite=Lax cookie. Five incorrect codes lock that username for 15 minutes.

### Google sign-in

Customers can also create an account or sign in with Google. Supabase Auth verifies the Google identity, and Fizz then issues its normal session cookie, so the rest of the API is unchanged:

1. **Continue with Google** on `/app.html` opens `GET /api/auth?provider=google`, adding `&invite=...` when creating an account. The function stores a PKCE verifier (and the invite) in a 10 minute HttpOnly cookie scoped to `/api/auth`, then redirects to Supabase's `/auth/v1/authorize`.
2. Supabase signs the user in with Google and redirects back to `GET /api/auth?code=...`. The function exchanges the code for the Supabase user using the verifier. A code minted for a different browser fails.
3. `fizz_oauth_sign_in` finds the customer linked to that `auth.users` id. If there is none, it needs an unused invite code (consumed as with regular sign-up) and creates a customer whose username comes from the email address, such as `janedoe` or `janedoe_4821`. Accounts are never matched by email, so Google sign-in cannot take over an existing username/code account.
4. The browser lands on `/app.html` with a `fizz_session` cookie. Failures come back as `/app.html#auth_error=<code>` (`no_account`, `invalid_invite`, `rate_limited`, `cancelled`, `expired`, or `failed`).

Google-only customers have no access code and cannot sign in with one. The browser never receives a Supabase key or token.

To enable it:

1. In Google Cloud Console, create an OAuth client ID of type **Web application**. Add `https://<project-ref>.supabase.co/auth/v1/callback` as an authorized redirect URI.
2. In Supabase, open **Authentication → Sign In / Providers → Google**, turn it on, and paste the client ID and secret.
3. In Supabase, open **Authentication → URL Configuration** and add `https://fizzlayer.com/api/auth` to **Redirect URLs**. For local development, also add `http://localhost:3000/api/auth`. The callback is built from the request's host, so each domain you sign in from needs its own entry.

No new Vercel variables are needed; the function uses `SUPABASE_URL` and `SUPABASE_SERVICE_ROLE_KEY`.

Apply the migration with `supabase db push --project-ref <project-ref>`, then set `SUPABASE_URL` and `SUPABASE_SERVICE_ROLE_KEY` in Vercel. The browser calls only Fizz's `/api/waitlist`, `/api/onboarding`, `/api/session`, `/api/auth`, and `/api/me` endpoints; it never receives a Supabase key.

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
| `POST /api/chat` | Ask Fizz about readings and create alerts directly |
| `GET`, `POST /api/voice` | Check voice availability, transcribe microphone audio, and speak agent replies (session cookie) |

For API ingestion, use a unique `event_id` for each sample; retries with the same ID are idempotent. Metrics are validated against the sensor type. For example, a Temperature sensor accepts `temperature_c`:

```sh
curl -X POST https://fizzlayer.com/api/ingest \
  -H 'Authorization: Bearer YOUR_SENSOR_API_KEY' \
  -H 'Content-Type: application/json' \
  -d '{"sensor_id":"YOUR_SENSOR_ID","event_id":"sample-001","metrics":{"temperature_c":27.4}}'
```

Phone pairing links expire after 15 minutes and can be claimed once. A claimed phone can share motion and orientation while its page is open, and can optionally share location, a short audio clip, or a transcript after separate consent. Browser support and permissions vary by phone. Owners can revoke paired phones from the Sensors page.

Alerts are evaluated in Supabase whenever a numeric reading is inserted. An alert can also call a US phone number when it triggers. Voice calls are the only enabled phone notification. The database queues the notification on the alert event, and the next Fizz request that stores readings (API ingest, phone readings, or a sample-stream tick) sends it through [AgentPhone](https://docs.agentphone.ai/). Each rule notifies at most once per 5 minutes, each account at most 20 times per hour, and transient connection or rate-limit rejections are retried up to three times within an hour. Sends with an uncertain outcome (timeouts or provider server errors) stop for review because AgentPhone does not support idempotency keys.

| Variable | Value |
| --- | --- |
| `AGENTPHONE_API_KEY` | AgentPhone API key, stored as a Vercel Production Secret |
| `AGENTPHONE_AGENT_ID` | Optional agent ID; required when the AgentPhone account has multiple agents. With exactly one agent, Fizz selects it automatically. |

Create an AgentPhone agent and attach a voice-enabled phone number to it. Calls use the agent's first attached number. Existing SMS rules are kept as dashboard alerts, and queued texts are skipped. Calls speak the alert as their greeting and use a short hosted conversation to repeat it if needed; Fizz disables audio recording for these calls. The delivery status means the provider accepted the send, rather than confirming receipt on the recipient's device.

Notifications are only sent where the AgentPhone key is set, so preview deployments sharing the database leave queued alerts for production. Chat uses Vercel AI Gateway to answer from stored customer readings and creates alerts directly when the customer clearly asks. It asks for missing details, validates the owned sensor and reported numeric metric/unit, and reports success only after the database saves the rule. Identical active alerts are reused. Phone calls are enabled only when requested with a US number supplied in the conversation. The browser includes up to 12 recent messages so follow-up instructions work; history is cleared at sign-out. An AI outage does not stop ingestion or alert evaluation.

## Talk to Fizz

The dashboard's **Talk to Fizz** button records one spoken turn (up to 30 seconds). Tap **Send recording** to submit it. ElevenLabs transcribes it, the same Fizz chat agent answers or creates an alert, and ElevenLabs speaks the reply. Replies remain visible in chat, and audio controls provide playback if the browser blocks autoplay. The microphone stops after each recording and on sign-out. Fizz does not save these microphone recordings; audio is sent to ElevenLabs for transcription, subject to the provider's retention policy.

| Variable | Value |
| --- | --- |
| `ELEVENLABS_API_KEY` | Server-only ElevenLabs API key with speech-to-text and text-to-speech permissions; store it as a Vercel Production Secret |
| `ELEVENLABS_VOICE_ID` | Optional voice ID; defaults to George (`JBFqnCBsd6RMkjVDRZzb`) |

Speech uses ElevenLabs `scribe_v2` transcription and `eleven_multilingual_v2` synthesis. This is voice chat inside Fizz; outbound threshold-alert calls continue to use AgentPhone. Typed chat stays available when speech or microphone access is unavailable.

## Agent connections (MCP)

AI agents reach a customer's data through Fizz's MCP server at `POST /api/mcp`. On the **Agents** page (`/agents.html`), the customer creates a token for each agent. The token looks like `fizz_agent_…`, is shown once, and is stored only as a hash. Each token is read-only unless the customer allows it to create dashboard alerts, and it can be revoked at any time. Agents never receive device keys, phone numbers, or the session cookie.

The server is stateless and speaks both MCP eras on the same endpoint. Clients on 2026-07-28 send the protocol version in `_meta` along with the `MCP-Protocol-Version`, `Mcp-Method`, and `Mcp-Name` headers. The server validates these headers and implements `server/discover`. Older clients (2025-03-26 through 2025-11-25) open with `initialize`. Every request needs `Authorization: Bearer <token>`. Tool calls are limited to 120 per minute per token. A request carrying a foreign `Origin` header is rejected.

| Tool | What it does |
| --- | --- |
| `list_sensors` | Sensors and paired devices with their latest reading |
| `get_readings` | Recent readings for one sensor, optionally one metric (up to 100) |
| `list_alerts` | Alert rules and the 20 most recent triggers |
| `create_alert` | Dashboard-only threshold alert; repeats return the existing rule (only for tokens allowed to create alerts) |
| `pair_device` | Claim a pairing code shown by `fizz pair` |

Hermes Agent (`config.yaml`):

```yaml
mcp_servers:
  fizz:
    url: "https://fizzlayer.com/api/mcp"
    headers:
      Authorization: "Bearer fizz_agent_..."
```

Claude Code: `claude mcp add --transport http fizz https://fizzlayer.com/api/mcp --header "Authorization: Bearer fizz_agent_..."`

## Device pairing and the Fizz CLI

The `fizz` CLI (`cli/`) is a small static Rust binary for Raspberry Pi and other Linux devices. On the device:

```sh
curl -fsSL https://fizzlayer.com/install.sh | sh
fizz pair
```

`fizz pair` shows a code such as `7K3P-Q9DM`. An agent claims it with `pair_device`, or the customer enters it on the Agents page. The CLI then asks the person at the device to approve the request, showing which account (and which agent) asked. After approval, Fizz creates an API-mode Custom sensor for the device and returns its key once. The CLI saves the key to `~/.config/fizz/device.json` with mode 600. Codes carry 40 random bits, expire after 15 minutes, are single use, and are rate-limited: 10 new codes per IP and 20 claims per account per hour. Rejected or expired codes connect nothing.

| Command | Purpose |
| --- | --- |
| `fizz run [--every 60] [--once]` | Send `cpu_temperature_c`, `load_1m`, `memory_used_pct`, and `disk_used_pct` |
| `fizz send METRIC VALUE …` | Send your own readings (numbers, `true`/`false`, or short text) |
| `fizz status` / `fizz unpair` | Show the pairing, or revoke this device's key and forget it |
| `fizz service` | Print a systemd unit that keeps `fizz run` going after reboot |

Readings go through the same `/api/ingest` path as any API sensor, so alerts, the dashboard, and chat work unchanged. Microcontrollers such as ESP32 can't run the CLI; create an API sensor on the Sensors page and post to `/api/ingest` with its key.

The installer detects the CPU (`aarch64`, `armv7`, `armv6`, or `x86_64`) and downloads the matching binary from this repository's latest GitHub Release. It verifies the binary against the release's `SHA256SUMS`, then installs it to `/usr/local/bin` or `~/.local/bin`. Set `FIZZ_VERSION` to pin a release, or `FIZZ_INSTALL_DIR` to choose the directory.

To release the CLI, bump `version` in `cli/Cargo.toml`, then push a matching tag:

```sh
git tag cli-v0.1.0 && git push origin cli-v0.1.0
```

The `CLI release` workflow tests the CLI and cross-compiles it with `cargo-zigbuild` for the four targets. It then publishes `fizz-<target>` binaries and `SHA256SUMS` as a GitHub Release. Releases must stay publicly downloadable for the installer to work, so if this repository becomes private, publish the binaries from a public repository instead and update `REPO` in `public/install.sh`.

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
