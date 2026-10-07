# Fizz product plan: sensors, phone, chat, alerts

## Product outcome

A customer signs in, picks one or more sensor types, and immediately sees useful sample data. Every sensor can instead accept readings through a documented API key. Phone sensors use a separate link that the customer can send to a phone. Fizz can answer questions about stored data and help create alerts. A dedicated Sensors page lets the customer add, configure, and remove sensors later.

The nine types are Water, Gas, Radio, Temperature, Pressure, Humidity, Sound, Phone, and Custom. A selected type becomes a **sensor instance** with its own name, mode, status, latest reading, and credential. A type alone is not a connected sensor.

## Current state and decisions

- The live app has email and password, Google, and passkey sign-in through Supabase Auth, a session cookie, and a multi-select grid of sensor type **choices**. Those choices are stored in `fizz_sensor_choices`; no instances or readings exist yet.
- The frontend is static HTML/CSS/JavaScript. API routes are Rust Vercel Functions. Supabase is the source of truth. A private experimental Supabase Compute service exists but currently serves only `/health`.
- Keep Rust for API and agent orchestration. Call Vercel AI Gateway from the Rust chat function, as agreed in the earlier plan. The model never receives Supabase credentials or sensor API keys.
- Use one scoped API key per API sensor instance, shown only at creation or rotation. Do not use one account-wide `API_KEY` for all sensors; revoking one device should leave others working.
- Store normalized numeric, boolean, and text readings plus units and timestamps. Retain raw payload only when needed for a custom sensor. Audio is a separate, consented media path, not a JSON reading.

## Wedge 1 — sensor instances and a simple onboarding handoff

**Experience:** The current type grid remains the first signed-in screen for accounts without sensors. Selecting types and pressing **Start with sample data** creates one simulated instance for each selected type and opens the dashboard. The customer does not have to configure units, keys, or phone permissions first. A small **Connect real data** action on each instance exposes API setup later. Phone gets **Send phone link** instead of a generic API setup prompt. Existing `fizz_sensor_choices` become the migration input so previous customers keep their selections.

**Build:** Add `fizz_sensors` with customer ownership, `kind`, `name`, `mode` (`simulated`, `api`, or `phone`), status, timestamps, and soft deletion. Add `fizz_sensor_readings` with instance ID, event ID, observed time, received time, metric name, value, unit, and source. Enforce ownership through server-side session checks and database functions. Add `GET`, `POST`, `PATCH`, and `DELETE /api/sensors` with IDs in JSON request bodies. Keep onboarding choices separate until migration and backfill are verified.

**Done when:** A new account reaches a dashboard in one action, each selected type has a named instance, reload preserves the instances, and an existing customer's choices migrate without duplication.

## Wedge 2 — real ingest and sample streams

**Experience:** Each sensor card shows its current mode, latest value, a small history chart, and a clear **Simulated** or **Live** label. The Sensors page offers **Try sample stream**, **Connect via API**, rotate key, pause, and remove. Sample mode includes visible example values and a short description of what each metric means.

**Ingest contract:** `POST /api/ingest` accepts `Authorization: Bearer <sensor API key>` and a compact JSON body such as:

```json
{"sensor_id":"YOUR_SENSOR_ID","event_id":"reading-001","observed_at":"2026-10-03T20:00:00Z","metrics":{"temperature_c":27.4}}
```

Validate that the key belongs to that instance, metric names and units match its type, values are bounded, and `(sensor_id, event_id)` is unique so retries do not duplicate readings or alerts. Return the accepted reading ID and server timestamp. Hash keys in Supabase, show the raw key once, allow rotation and revocation, limit payload size and rate, and document `curl` examples for each type. A Custom sensor starts with a simple metric name, unit, and numeric or text value schema.

**Sample fixtures:** Water: flow and leak state; Gas: concentration and alarm state; Radio: signal strength; Temperature: degrees C; Pressure: kPa; Humidity: percent; Sound: decibels; Phone: acceleration magnitude and orientation; Custom: a sample numeric metric. Show units and realistic value ranges in the UI and docs. Use the same reading pipeline for simulated and live data, while labeling source on every reading. Simulated readings advance on authenticated API reads at most once per 15 seconds; the dashboard polls while open.

**Done when:** A sample stream for each of the nine types produces readable history, an external client can send authenticated readings, duplicate event IDs have one effect, and revoking a key stops further ingest.

## Wedge 3 — send a phone link and capture browser sensors

**Experience:** From a Phone sensor, the customer clicks **Create phone link**, copies or shares a unique HTTPS URL, and sees whether that phone is connected. Opening the URL shows the sensor owner and a plain list of available signals. The phone user explicitly starts each capability and can stop sharing at any time. The dashboard can revoke the connection.

**Pairing:** The link contains a short-lived, single-use pairing token, scoped to one Phone instance. Exchange it on the phone page for a device session; do not put a permanent API key in the URL. Allow expiry, regeneration, and revocation. A paired phone publishes through the same authenticated ingest pipeline as other instances.

**Browser capabilities:** Start with accelerometer/device motion and orientation where supported. Add microphone capture through `getUserMedia` only after a separate user gesture and browser permission; show a visible recording state. Store short audio clips only if the user elects to share audio, and create a transcript for chat queries. Geolocation can be an optional permission later. Show unsupported and denied capabilities individually. Mobile browser access requires HTTPS, varies by browser, and streaming stops when the tab or browser is closed or suspended; communicate this on the phone page.

**Done when:** A second phone can open a fresh link, grant motion permission, send readings visible on the dashboard, optionally record a short voice clip, and revoke access from either device. An expired or reused link cannot pair another phone.

## Wedge 4 — Fizz chat grounded in sensor data

**Experience:** Add a chat window beside sensor status on desktop and beneath it on mobile. Fizz can answer questions such as “What is the temperature now?”, “Was there a gas spike today?”, and “What did the phone recording say?” Answers identify the sensor, value or transcript, and time. Show a clear unavailable state if the model fails.

**Build:** The Rust `/api/chat` function uses Vercel AI Gateway with bounded tool calls. Read-only tools fetch the customer's sensor list, latest values, time windows, alert history, and consented transcripts. Scope every tool query by the authenticated customer; keep provider keys on the server. Persist chat turns with bounded history. The model can propose actions, but data reads and rule writes stay in typed server code.

**Done when:** Chat answers are grounded in stored readings, cannot query another customer, report missing or stale data honestly, and cannot invent an alert or sensor connection.

## Wedge 5 — alerts by form and chat

**Experience:** A customer can create an alert from a sensor card or say “Tell me when the temperature exceeds 30 °C.” Fizz displays an editable confirmation with sensor, metric, comparator, threshold, unit, and delivery choice. The customer confirms before the rule becomes active. Start with in-app notifications and an alert timeline; add email or push after that flow works.

**Build:** Store typed `alert_rules` and `alert_events`. Parse chat requests into a proposed rule, validate against the selected sensor's metric schema, then create the rule through an authenticated server action only after confirmation. Evaluate rules deterministically as readings are accepted. Record threshold crossings, use a cooldown or re-arm condition to avoid repeated alerts, and make `(rule_id, reading_id)` idempotent. The model explains alerts; it does not decide whether a threshold fired.

**Done when:** A temperature reading below the threshold is quiet, a crossing above it creates one visible alert, a duplicate reading creates none, and disabling or deleting the rule stops future alerts. Chat-created and form-created rules behave identically.

## Wedge 6 — dedicated Sensors page and operational polish

The Sensors page lists every instance with type, mode, last reading, freshness, alert count, and connection state. Add and remove instances there without returning to onboarding. Removing an instance revokes its API key or phone session immediately, stops simulation, disables its alerts, and retains historical readings as an archived record unless the customer explicitly deletes history. Offer key rotation, phone link regeneration, and a copyable API example in context.

Set retention limits for high-frequency readings and audio, monitor ingest errors and worker health, and show when data is stale. Test desktop and mobile, permission denial, missing data, expired credentials, repeated events, and a model outage. Production Vercel deployment protection currently gates access, so a sendable phone link also needs an intentional public route or protection bypass design before the phone wedge is released.

## Release order

| Release | Customer-visible result | Gate |
| --- | --- | --- |
| 1 | One-click simulated sensors and dashboard | Existing choices migrate; no duplicate instances |
| 2 | API keys, real ingest, nine sample streams, Sensors page basics | Auth, retry, rate, and revocation checks |
| 3 | Unique phone link with motion and optional voice clip | Consent, expiry, revocation, and real phone test |
| 4 | Vercel AI Gateway chat over readings | Grounding, tenant isolation, failure behavior |
| 5 | Threshold alerts by form and chat | Deterministic crossing and idempotency checks |
| 6 | Full sensor management and retention polish | Remove/restore behavior and stale-data checks |

## Choices to settle during implementation

1. Whether phone voice means short clips plus transcripts for the first demo, or continuous audio while the page is open. Short clips are the proposed first release because consent, storage, and browser behavior are easier to make clear.
2. Whether the production Fizz site should be reachable by anyone with a Fizz account. Vercel deployment protection currently blocks ordinary visitors, including recipients of phone links.
3. Which Vercel AI Gateway model and spending limit to use when chat is built. Verify current tool support and pricing before selecting one.
