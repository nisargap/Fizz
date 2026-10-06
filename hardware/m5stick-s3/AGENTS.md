# M5StickS3 firmware — notes for agents

`fizz_stick.py` is a UiFlow2 MicroPython program that pairs an M5StickS3 with Fizz and sends readings. `main.py` starts it at boot. Everything here was verified on a StickS3 running UiFlow 2.4.8 (MicroPython 1.27.0, ESP32-S3-PICO-1, about 8 MB free RAM).

## What the program does

1. Waits for Wi-Fi (UiFlow connects it) and syncs the clock over NTP if the year looks wrong.
2. If `/flash/fizz.json` is missing, pairs with the same flow as `fizz pair` in `cli/src/main.rs`:
   `POST /api/devices` with `action` `start` → shows the code → polls `status` every 3 s → when `claimed`, shows the account and agent and waits for BtnA (approve) or BtnB (reject) → `approve` returns `sensor_id` and `api_key`, saved to `/flash/fizz.json`.
3. Every 15 s posts `acceleration_ms2` (peak magnitude in the window, gravity included, so about 9.8 at rest), `sound_db`, and `battery_pct` to `POST /api/ingest` with `Authorization: Bearer <api_key>`.
4. `main()` retries every 30 s after any error, because nobody watches the console at boot. A 401 means the key was revoked; it stops and says so on screen.

Paired devices become `custom` sensors, which accept any metric name matching `^[a-z][a-z0-9_]{0,39}$`. The server adds units for known names (see `fizz_ingest_event` in `supabase/migrations/20261005000000_agent_connections.sql`); `battery_pct` has no unit there yet. Do not send more often than every 15 s; the CLI enforces the same floor. API errors look like `{"error": {"code": ..., "message": ...}}`.

## Files on the stick

The writable filesystem is `/flash` (about 1.6 MB free). `/` is a virtual root; writing there fails with `ENODEV`.

| Path | What |
|---|---|
| `/flash/libs/fizz_stick.py` | This program. `/flash/libs` is on `sys.path`. |
| `/flash/main.py` | Copy of `main.py`. Stock UiFlow ships it empty. |
| `/flash/fizz_ca.der` | ISRG Root X1 in DER: `openssl x509 -in /etc/ssl/certs/ISRG_Root_X1.pem -outform DER` |
| `/flash/fizz.json` | The pairing, including the device key. Never print or commit its contents. |

`boot.py` belongs to UiFlow; leave it alone. Startup is controlled by the `u8` key `boot_option` in NVS namespace `uiflow`: `1` = startup menu (default), `2` = connect Wi-Fi then run `main.py` (what we use), `0` = "run main.py directly" per `boot.py` (untested; it also skips UiFlow's cloud sync). Set it with `esp32.NVS("uiflow").set_u8("boot_option", 2)` and `commit()`. Holding BtnA while powering on brings back the menu and resets the option to 1.

## Talking to the stick over USB

- It enumerates as `303a:832b` "StickS3(UiFlow2)" on `/dev/ttyACM0` (stable path: `/dev/serial/by-id/usb-M5Stack_StickS3_UiFlow2_*`). A charge-only cable shows a USB-C plug event in the kernel log but no device.
- The port is `root:uucp`. If the user was just added to `uucp`, the running session will not have the group yet; on systems without `sg`, pipe commands into `newgrp uucp`.
- **`mpremote run` / `exec` are unreliable here.** Their soft reset reboots UiFlow, USB re-enumerates, and the next connection lands mid-boot. Instead, open the port with pyserial keeping DTR and RTS high, send Ctrl-C (plus Ctrl-B in case a previous run left raw REPL) until `>>> ` appears, then use raw REPL (Ctrl-A, code, Ctrl-D) and read stdout up to `\x04`. Closing the port this way does not reset the board.
- If the stick ignores Ctrl-C, reboot it (side button or replug) and send Ctrl-C right as it re-enumerates; that interrupts `startup/sticks3.py` with a `KeyboardInterrupt`.
- Interrupting stops whatever was running, including the Fizz program. `machine.reset()` restarts it through `main.py`.

## Hardware quirks

- **TLS clock bug.** This firmware's mbedTLS reads the clock as seconds since 2000 while the clock counts from 1970, so every certificate fails with "The certificate validity has expired" even though `time.gmtime()` is correct. Confirmed by setting the RTC back 30 years, after which full verification passes. `_handshake()` shifts the clock back by `946684800` s for the handshake only, and only after a normal handshake fails with that error, so a fixed firmware never shifts. Chain and hostname are still verified: trusting ISRG Root X2 instead gives "not correctly signed by the trusted CA". Do not "fix" TLS by switching to `CERT_NONE`.
- **Mic and speaker share one codec.** Call `Speaker.end()` before `Mic.begin()` (and the reverse). A speaker left on while the mic runs produces audible crackling.
- **`Mic.record(buf, rate, stereo)` is asynchronous**, and `Mic.isRecording()` is unreliable: it can report idle before recording starts and can stay busy afterwards. Wait for the buffer's duration plus a margin instead of polling it. Same for `Speaker.playRaw` and `isPlaying()`.
- `M5.update()` must run in any loop that reads `BtnA` / `BtnB`.
- The mic is not calibrated. `sound_db` is `20*log10(rms/32768) + 90`, chosen so speech lands around 60–70 dB; treat it as relative. A loud test clip peaked at 32752 (clipping), so expect saturation up close.

## Checking it works

Without touching the stick, read its sensor through the Fizz MCP tools (`list_sensors`, then `get_readings`) and confirm new readings arrive about every 15 s. After a reboot, the first readings arrived about 20 s later.

To unpair: `import fizz_stick; fizz_stick.unpair()` revokes the key and deletes `/flash/fizz.json`; the next boot starts pairing again.
