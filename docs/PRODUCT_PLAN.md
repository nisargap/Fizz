# Fizz product plan

## Goal

Build a small, convincing Supabase Select hackathon demo of **Fizz, the physical layer for AI agents**. A user enters one six digit access code, sees connected devices, talks to Fizz, and triggers a simulated water leak. Fizz immediately closes a simulated main valve, records what happened, and explains the incident in chat. A documented API lets someone register another sensor and send readings.

The first release controls **simulated devices only**. The architecture leaves an adapter boundary for physical hardware later; the demo must label simulated actions clearly.

## Demo success criteria

1. Open the landing page and enter the six digit code to reach the dashboard.
2. See a seeded water leak sensor and main valve, with the valve initially open.
3. Add another sensor through the dashboard or `POST /api/sensors`; receive its one time ingest token.
4. Press **Simulate leak**, or send a leak reading through the new sensor's API token.
5. In one request, the rule engine records the reading, creates an incident, closes the simulated valve, and records the command. The UI shows the updated state and an in app alert without a reload.
6. Ask Fizz “What happened?” and get a short answer grounded in the stored reading, incident, and valve state.
7. Press **Reset demo** and repeat the sequence. Repeating the same reading ID must not create a second incident or command.

If the AI provider is unavailable, the safety action still happens and the UI shows a clear chat error. Fizz must never claim an action succeeded until the stored command says it did.

## Scope for the first demo

| Build now | Later |
| --- | --- |
| One shared six digit code and short lived session | User accounts, organizations, invitations |
| Leak sensors and one simulated water valve | General device types and real device drivers |
| Manual simulation button and authenticated sensor ingest API | MQTT, device discovery, firmware management |
| Deterministic leak to shutoff rule, incident log, in app alert | Custom rule editor, SMS, email, push notifications |
| Text chat with read only system tools | Autonomous model initiated actuator commands |
| Supabase persistence and repeatable seed/reset | Multi site history and analytics |

## Product and visual direction

- Keep the existing public landing page. Its CTA becomes **Enter Fizz** and opens a separate dashboard page.
- Use plain HTML, CSS, and a small amount of vanilla JavaScript for fetch and polling. HTMX is an option only if it removes code in a specific view; no frontend framework or build step.
- Carry forward VT323, the solid `#0b1722` background, light blue `#a9eaf7`, darker blue outlines, square or stepped panels, crisp pixel borders, and the animated mascot. Avoid gradients, blur, and pink accents.
- Desktop: chat is the primary column; device state, incident, and simulator controls sit beside it. Mobile: stack the same sections with the active incident first.
- Make simulation obvious: label the sensor and valve **Simulated**. The event timeline should use plain verbs: “Leak detected,” “Valve closed,” “Incident opened.”
- Use native form controls, visible focus states, high contrast text, and reduced motion support.

## Technical shape

```text
public/index.html     landing page
public/app.html       code entry + dashboard shell
public/app.css        shared pixel design tokens and dashboard styles
public/app.js         small fetch/polling/chat controller

api/*.rs              Vercel Rust Functions, each declared as a Cargo binary
src/lib.rs            shared Rust library for API handlers
src/auth/             code verification, sessions, device token checks
src/store/            Supabase REST/RPC client and persistence mapping
src/domain/           device types and deterministic leak rule
src/agent/            AI Gateway HTTP client, prompt, read only tools
supabase/migrations/  SQL schema, RPC transaction, and policies
```

Vercel Functions are stateless: no durable device state or conversation history in process memory. Supabase Postgres is the source of truth. Rust functions use Supabase's HTTP APIs; the browser only calls Fizz endpoints. Keep the Supabase service role credential server side.

The chat agent uses Vercel AI Gateway's OpenAI compatible HTTP endpoint from Rust. Rust owns the short tool loop and calls read only tools such as `get_system_state` and `get_recent_events`. This uses Vercel model routing while honoring the all Rust requirement. The TypeScript AI SDK agent helper is outside this scope. Confirm this interpretation against the hackathon judging requirements before implementation; if a named SDK is mandatory, revisit the language constraint deliberately.

The **leak rule is Rust code, not a model instruction**. Any authorized wet reading for an armed leak sensor produces a close valve decision. Supabase applies the reading and that decision in one database transaction. Chat can explain or suggest next steps, but it cannot bypass the rule, mark a command successful, or reopen the valve. Reset is an explicit authenticated demo action.

## Data contract

Use UUID primary keys and UTC timestamps. Start with these tables:

| Table | Essential fields | Purpose |
| --- | --- | --- |
| `devices` | `id`, `name`, `kind` (`leak_sensor` or `water_valve`), `mode` (`simulated`), `state`, `created_at` | Connected device registry and current state |
| `device_tokens` | `device_id`, `token_hash`, `created_at`, `revoked_at` | Per sensor ingest credentials; return raw token once |
| `readings` | `id`, `device_id`, `event_id` (unique per device), `wet`, `created_at` | Incoming sensor facts and retry deduplication |
| `incidents` | `id`, `sensor_id`, `reading_id` (unique), `status`, `created_at`, `resolved_at` | User visible leak incidents |
| `commands` | `id`, `incident_id` (unique), `device_id`, `action`, `status`, `created_at` | Auditable simulated valve actions |
| `messages` | `id`, `role`, `content`, `created_at` | Small shared chat transcript for the demo |

The Rust domain module owns the wet reading to close valve decision. The SQL migration owns an `apply_reading_decision` RPC that inserts the reading and atomically applies that validated decision by creating the incident, setting the valve state, and recording one command. Duplicate `(device_id, event_id)` calls return the existing outcome. Decide the exact JSON shape in the shared contract before writing the RPC and Rust client.

Seed one sensor and one valve. Store no raw device tokens in the database. Restrict browser access to the tables; only server side functions use privileged Supabase credentials.

## HTTP contract (v1)

All responses are JSON except the static pages. Errors use `{ "error": { "code": "...", "message": "..." } }` and an appropriate HTTP status. Session endpoints use an HttpOnly cookie. Device ingestion uses a per sensor bearer token. All other API routes require the session.

| Method and route | Request | Response / effect |
| --- | --- | --- |
| `POST /api/session` | `{ "code": "123456" }` | Sets a signed session cookie; returns `{ "ok": true }` |
| `DELETE /api/session` | None | Clears cookie |
| `GET /api/state` | None | `{ "devices": [], "active_incident": null, "recent_events": [] }` |
| `POST /api/sensors` | `{ "name": "Basement leak sensor" }` | Creates simulated leak sensor; returns its ID and one time `ingest_token` |
| `POST /api/readings` | `{ "event_id": "client-generated-id", "wet": true }` | With sensor bearer token, records reading and rule outcome |
| `POST /api/demo` | `{ "action": "leak" }` or `{ "action": "reset" }` | Uses the same rule path as ingest, or restores seeded demo state |
| `POST /api/chat` | `{ "message": "What happened?" }` | Persists turn, runs bounded read only tool loop, returns `{ "reply": "..." }` |

The frontend polls `GET /api/state` every 2–3 seconds while visible, and refreshes immediately after a mutation. No WebSocket or background worker is needed for the demo. Cap request sizes, chat turns, and tool iterations so a bad prompt cannot create an unbounded run.

## Authentication and configuration

- `FIZZ_ACCESS_CODE`: six digit secret set in Vercel, never in source or browser JavaScript.
- `FIZZ_SESSION_SECRET`: separate random secret for a signed, HttpOnly, Secure, SameSite=Lax cookie with a short expiry.
- `SUPABASE_URL` and `SUPABASE_SERVICE_ROLE_KEY`: server side Supabase access.
- `AI_GATEWAY_API_KEY` and `FIZZ_MODEL`: model access and chosen model ID. Check the selected model supports tool calls before hardcoding it.
- Rate limit code attempts at the Vercel edge or through a small shared counter; a six digit code cannot be exposed to unlimited guessing. Do not log the code, device tokens, or service role key.
- Keep Vercel deployment protection enabled during construction. Revisit its setting when the code screen is ready and the intended audience is known.

The user can connect the existing Supabase project when implementation starts. The integration owner then applies the migration, sets Vercel environment variables, and verifies preview and production use the intended database. Do not commit credentials or a local `.env` file.

## Parallel work plan

**Phase 0: one short integration pass before parallel work.** The integrator adds `src/lib.rs`, shared request/response types, the exact `apply_reading_decision` RPC contract, a route/binary naming convention, and the needed `[[bin]]` entries in `Cargo.toml`. This is the only shared contract others should build against. Parallel agents work in separate branches or worktrees and own disjoint paths.

| Owner | File ownership | Deliverable | Can start after |
| --- | --- | --- | --- |
| Agent A — interface | `public/app.html`, `public/app.css`, `public/app.js`, small landing CTA edit | Code entry, device cards, simulator, incident timeline, chat; responsive pixel style and fetch calls matching the HTTP contract | Phase 0 API shapes |
| Agent B — Supabase | `supabase/migrations/**`, `src/store/**` | Schema, seed/reset SQL, transactional leak RPC, HTTP store client, idempotency proof | Phase 0 RPC and types |
| Agent C — rules and simulation | `src/domain/**`, `src/simulator/**` | Typed leak rule, device state transitions, demo fixtures, unit tests for wet/dry/duplicate readings | Phase 0 types |
| Agent D — chat agent | `src/agent/**` | AI Gateway client, bounded read only tool loop, grounded prompt, unavailable/error behavior | Phase 0 types and tool interface |
| Integrator — auth and API | `Cargo.toml`, `src/lib.rs`, `src/auth/**`, `api/**`, `README.md`, Vercel/Supabase settings | PIN/session/device auth, route wiring, end to end deployment and demo verification | Runs alongside agents; merges after each contract check |

Agents must not edit another owner's paths. Shared contract changes go through the integrator first. If only four workers are available, the integrator plus Agents A–C start together; Agent D starts when one slot frees. Agent A can use mocked contract responses until the API is ready.

## Delivery order and gates

1. **Vertical slice:** Supabase connected, schema applied, login works, seeded state renders, leak button closes valve and opens one incident. Verify this before AI work is merged.
2. **Open API:** register sensor, obtain token, send wet and dry readings, retry an event ID, confirm no duplicate action.
3. **Agent:** ask about current state and leak history; verify answers cite current stored facts and failed model calls do not affect valve state.
4. **Polish:** pixel UI, mobile view, loading/error states, in app alert, reset, clear simulation labels.
5. **Demo rehearsal:** run the seven success criteria from a fresh browser session, then test wrong code, missing token, dry reading, duplicate wet reading, and model failure.

## Decisions to confirm before build

1. Which Supabase project should back preview and production? One project is simplest for the hackathon; a separate preview database avoids test data in the live demo.
2. Which AI Gateway model and spending limit should Fizz use? Choose a tool capable model after checking the current model list.
3. Does the hackathon require a specific Vercel AI SDK or Agent implementation, or is a Rust agent using AI Gateway acceptable?
4. Should the demo site be public behind Fizz's six digit code, or restricted to invited Vercel accounts as it is today?

## References

- [Vercel Rust Functions](https://vercel.com/docs/functions/runtimes/rust)
- [Vercel AI Gateway HTTP and tool calling](https://vercel.com/docs/ai-gateway/sdks-and-apis/openai-chat-completions/tool-calling)
- [Supabase database migrations](https://supabase.com/docs/guides/deployment/database-migrations)
