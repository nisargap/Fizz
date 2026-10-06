# Fizz for the M5StickS3 (UiFlow2 / MicroPython).
#
# Pairs the stick with a Fizz account using the same flow as `fizz pair`, then sends
# motion, sound level, and battery readings every 15 seconds.
#
# Files on the stick:
#   /flash/libs/fizz_stick.py   this program
#   /flash/fizz_ca.der          ISRG Root X1 (Let's Encrypt) in DER form, used to verify fizzlayer.com
#   /flash/fizz.json            the pairing (sensor ID and key), written after you approve
#
# Run:     import fizz_stick; fizz_stick.main()
#          (/flash/main.py does this at boot; hold BtnA while powering on for the UiFlow menu)
# Unpair:  import fizz_stick; fizz_stick.unpair()

import json, math, os, socket, ssl, time
import M5
from M5 import BtnA, BtnB, Display, Imu, Mic, Power, Speaker
import machine, network

HOST = "fizzlayer.com"
CONFIG = "/flash/fizz.json"
CA_FILE = "/flash/fizz_ca.der"
EVERY_S = 15          # the API accepts at most one reading batch per 15 seconds per device
MIC_RATE = 16000
MIC_CHUNK_MS = 200
# The mic is not calibrated; this offset maps its digital level (dBFS) to a rough room
# sound level so normal speech lands around 60-70 dB.
SPL_OFFSET_DB = 90

_ctx = None
# UiFlow 2.4.8's TLS library reads the clock as seconds since 2000 while the clock counts
# from 1970, so every certificate looks 30 years expired. When that happens we shift the
# clock back by the difference for the handshake only; the chain and hostname are still
# fully verified. A normal handshake is tried first, so a fixed firmware never shifts.
_EPOCH_SKEW_S = 946684800
_skew_tls = False


# ---- screen -----------------------------------------------------------------

def show(lines, color=0xFFFFFF, size=2):
    Display.clear(0x000000)
    Display.setTextColor(color, 0x000000)
    Display.setTextSize(size)
    Display.setCursor(4, 8)
    Display.print(lines)


# ---- HTTPS ------------------------------------------------------------------

def _tls():
    global _ctx
    if _ctx is None:
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        ctx.verify_mode = ssl.CERT_REQUIRED
        with open(CA_FILE, "rb") as f:
            ctx.load_verify_locations(cadata=f.read())
        _ctx = ctx
    return _ctx


def _dechunk(body):
    out = b""
    while body:
        size_line, _, body = body.partition(b"\r\n")
        size = int(size_line.split(b";")[0], 16)
        if size == 0:
            break
        out += body[:size]
        body = body[size + 2:]
    return out


def _set_clock(secs):
    tm = time.gmtime(secs)
    machine.RTC().datetime((tm[0], tm[1], tm[2], tm[6], tm[3], tm[4], tm[5], 0))


def _handshake(sock):
    if not _skew_tls:
        return _tls().wrap_socket(sock, server_hostname=HOST)
    real, started = time.time(), time.ticks_ms()
    _set_clock(real - _EPOCH_SKEW_S)
    try:
        return _tls().wrap_socket(sock, server_hostname=HOST)
    finally:
        _set_clock(real + time.ticks_diff(time.ticks_ms(), started) // 1000)


def _connect():
    global _skew_tls
    addr = socket.getaddrinfo(HOST, 443, 0, socket.SOCK_STREAM)[0][-1]
    for _ in range(2):
        sock = socket.socket()
        sock.settimeout(20)
        try:
            sock.connect(addr)
            return _handshake(sock)
        except ValueError as e:
            sock.close()
            if _skew_tls or "expired" not in str(e):
                raise
            _skew_tls = True
            print("TLS: compensating for the firmware's certificate date bug")
        except Exception:
            sock.close()
            raise


def post(path, payload, key=None):
    """POST JSON to Fizz over verified TLS. Returns (status, parsed JSON body)."""
    data = json.dumps(payload).encode()
    sock = _connect()
    try:
        head = (
            "POST %s HTTP/1.0\r\nHost: %s\r\nContent-Type: application/json\r\n"
            "Content-Length: %d\r\nUser-Agent: fizz-sticks3/0.1\r\n" % (path, HOST, len(data))
        )
        if key:
            head += "Authorization: Bearer %s\r\n" % key
        sock.write(head.encode() + b"\r\n" + data)
        resp = b""
        while True:
            chunk = sock.read(1024)
            if not chunk:
                break
            resp += chunk
    finally:
        sock.close()
    header, _, body = resp.partition(b"\r\n\r\n")
    status = int(header.split(b" ", 2)[1])
    if b"transfer-encoding: chunked" in header.lower():
        body = _dechunk(body)
    try:
        return status, json.loads(body) if body else {}
    except ValueError:
        return status, {"error": "bad_response"}


def message(body):
    err = body.get("error")
    if isinstance(err, dict):
        return err.get("message") or err.get("code") or "unknown error"
    return body.get("message") or err or "unknown error"


# ---- config -----------------------------------------------------------------

def load_config():
    try:
        with open(CONFIG) as f:
            cfg = json.load(f)
        return cfg if cfg.get("sensor_id") and cfg.get("api_key") else None
    except (OSError, ValueError):
        return None


def save_config(cfg):
    with open(CONFIG, "w") as f:
        json.dump(cfg, f)


# ---- pairing ----------------------------------------------------------------

def wait_wifi(timeout_s=30):
    wlan = network.WLAN(network.STA_IF)
    end = time.ticks_add(time.ticks_ms(), timeout_s * 1000)
    while not wlan.isconnected():
        if time.ticks_diff(end, time.ticks_ms()) <= 0:
            raise RuntimeError("Wi-Fi is not connected")
        show("Waiting for\nWi-Fi...")
        time.sleep_ms(500)
    if time.gmtime()[0] < 2025:  # certificate checks need a real clock
        try:
            import ntptime
            ntptime.settime()
        except Exception as e:
            print("clock sync failed:", e)


def wait_button(timeout_ms):
    """Return 'A', 'B', or None if neither was pressed in time."""
    end = time.ticks_add(time.ticks_ms(), timeout_ms)
    while time.ticks_diff(end, time.ticks_ms()) > 0:
        M5.update()
        if BtnA.wasPressed():
            return "A"
        if BtnB.wasPressed():
            return "B"
        time.sleep_ms(20)
    return None


def pair(name="M5StickS3"):
    show("Fizz\nStarting pairing...", 0x7FFFD4)
    status, start = post("/api/devices", {"action": "start", "name": name, "platform": "m5stack-sticks3 uiflow2"})
    if status != 201:
        raise RuntimeError("pairing start failed: %s" % message(start))
    code, secret = start["code"], start["poll_secret"]
    print("FIZZ_PAIR_CODE", code)
    show("Fizz pairing code\n\n%s\n\nTell your agent or\nuse the Agents page" % code, 0x7FFFD4)

    while True:
        time.sleep(3)
        try:
            status, reply = post("/api/devices", {"action": "status", "poll_secret": secret})
        except OSError as e:
            print("status retry:", e)
            continue
        if status != 200:
            raise RuntimeError("pairing status failed: %s" % message(reply))
        state = reply.get("status")
        if state == "waiting":
            continue
        if state != "claimed":
            show("Pairing ended:\n%s\nRestart to retry" % state, 0xFF5555)
            raise RuntimeError("pairing ended: %s" % state)
        break

    account = str(reply.get("account", "?"))[:20]
    who = "Agent " + str(reply.get("agent_name"))[:16] if reply.get("claimed_via") == "agent" else "Dashboard"
    print("FIZZ_CLAIMED account=%s by=%s" % (account, who))
    show("%s on account\n%s\nwants this stick.\n\nA = approve\nB = reject" % (who, account), 0xFFD700)
    choice = wait_button(5 * 60 * 1000)
    action = "approve" if choice == "A" else "reject"
    status, decision = post("/api/devices", {"action": action, "poll_secret": secret})
    if status != 200:
        raise RuntimeError("pairing %s failed: %s" % (action, message(decision)))
    if action == "reject":
        show("Pairing rejected.\nNothing connected.", 0xFF5555)
        print("FIZZ_REJECTED")
        return None
    cfg = {
        "sensor_id": decision["sensor_id"],
        "api_key": decision["api_key"],
        "device_name": decision.get("device_name", name),
        "account": decision.get("account", account),
    }
    save_config(cfg)
    print("FIZZ_PAIRED sensor_id=%s" % cfg["sensor_id"])
    show("Paired with\n%s" % cfg["account"], 0x55FF55)
    time.sleep(2)
    return cfg


def unpair():
    cfg = load_config()
    if not cfg:
        print("not paired")
        return
    wait_wifi()
    status, body = post("/api/devices", {"action": "unpair", "sensor_id": cfg["sensor_id"]}, cfg["api_key"])
    print("unpair:", status, body)
    if status in (200, 401, 404):
        os.remove(CONFIG)
        show("Unpaired from Fizz")


# ---- readings ---------------------------------------------------------------

def sound_db(buf):
    total, n = 0, len(buf) // 2
    for i in range(0, n, 4):  # every 4th sample is plenty for a level
        v = buf[2 * i] | (buf[2 * i + 1] << 8)
        if v >= 32768:
            v -= 65536
        total += v * v
    rms = math.sqrt(total / (n // 4)) if n else 0
    if rms < 1:
        return 0.0
    return max(0.0, min(160.0, 20 * math.log10(rms / 32768) + SPL_OFFSET_DB))


def collect(seconds):
    """Sample motion and sound for `seconds`; return the metrics for one reading."""
    buf = bytearray(MIC_RATE * 2 * MIC_CHUNK_MS // 1000)
    peak_g, levels = 0.0, []
    end = time.ticks_add(time.ticks_ms(), seconds * 1000)
    while time.ticks_diff(end, time.ticks_ms()) > 0:
        Mic.record(buf, MIC_RATE, False)
        chunk_end = time.ticks_add(time.ticks_ms(), MIC_CHUNK_MS + 30)
        while time.ticks_diff(chunk_end, time.ticks_ms()) > 0:
            x, y, z = Imu.getAccel()
            peak_g = max(peak_g, math.sqrt(x * x + y * y + z * z))
            time.sleep_ms(20)
        levels.append(sound_db(buf))
    metrics = {
        "acceleration_ms2": round(peak_g * 9.80665, 2),
        "sound_db": round(sum(levels) / len(levels), 1) if levels else 0.0,
    }
    try:
        metrics["battery_pct"] = Power.getBatteryLevel()
    except Exception:
        pass
    return metrics


def run(cfg):
    # The mic and speaker share one codec; a live speaker turns the mic into static.
    Speaker.end()
    Mic.begin()
    boot = os.urandom(4).hex()
    count = 0
    while True:
        metrics = collect(EVERY_S)
        count += 1
        event = "s3-%s-%d" % (boot, count)
        try:
            status, body = post("/api/ingest", {"sensor_id": cfg["sensor_id"], "event_id": event, "metrics": metrics}, cfg["api_key"])
        except OSError as e:
            status, body = 0, {"error": str(e)}
        ok = 200 <= status < 300 and "error" not in body
        print("FIZZ_READING", status, json.dumps(metrics), "" if ok else message(body))
        if status == 401:
            show("Key revoked.\nRun unpair, then\npair again.", 0xFF5555)
            return
        show(
            "Fizz %s\n%s\nmove %.1f m/s2\nsound %.0f dB\nbatt %s%%"
            % ("live" if ok else "retrying", cfg["device_name"][:16], metrics["acceleration_ms2"],
               metrics["sound_db"], metrics.get("battery_pct", "?")),
            0x55FF55 if ok else 0xFFD700,
        )


def main():
    # Runs unattended at boot, so recover from network or server errors instead of stopping.
    while True:
        try:
            wait_wifi()
            cfg = load_config() or pair()
            if not cfg:
                return  # pairing was rejected at the device
            run(cfg)
            return  # key revoked; run() already said so on screen
        except Exception as e:
            print("FIZZ_ERROR", repr(e))
            show("Fizz error:\n%s\n\nRetrying in 30s" % str(e)[:60], 0xFF5555)
            time.sleep(30)
