//! `fizz`: pair a device with Fizz and stream its readings.
//!
//! Pairing shows a short code. An agent (through Fizz's MCP server) or the Fizz dashboard claims
//! it, and the person at this device approves the request here. Fizz then issues this device its
//! own sensor key, stored only in the local config file with owner-only permissions.

use std::{
    env, fs,
    io::{self, BufRead, Write},
    path::PathBuf,
    process::ExitCode,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::{Map, Value, json};

const DEFAULT_API: &str = "https://fizzlayer.com";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const HELP: &str = "fizz — connect this device to Fizz

Usage:
  fizz pair [--name NAME] [--force]   Show a pairing code and approve the request here
  fizz run [--every SECONDS] [--once] Send CPU temperature, load, memory, and disk usage
  fizz send METRIC VALUE [...]        Send your own readings, e.g. fizz send temperature_c 22.4
  fizz status                         Show this device's pairing
  fizz unpair                         Revoke this device's key and forget the pairing
  fizz service                        Print a systemd unit that keeps `fizz run` going
  fizz --version

Environment:
  FIZZ_API      Fizz base URL (default https://fizzlayer.com)
  FIZZ_CONFIG   Config file (default ~/.config/fizz/device.json)";

type Result<T> = std::result::Result<T, String>;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("pair") => pair(&args[1..]),
        Some("run") => run(&args[1..]),
        Some("send") => send(&args[1..]),
        Some("status") => status(),
        Some("unpair") => unpair(),
        Some("service") => service(),
        Some("--version" | "-V" | "version") => {
            println!("fizz {VERSION}");
            Ok(())
        }
        None | Some("help" | "--help" | "-h") => {
            println!("{HELP}");
            Ok(())
        }
        Some(other) => Err(format!(
            "Unknown command `{other}`. Run `fizz help` for usage."
        )),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("fizz: {message}");
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------------------------------------
// HTTP

fn api_base() -> String {
    env::var("FIZZ_API")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_API.to_owned())
        .trim_end_matches('/')
        .to_owned()
}

/// Paired devices keep talking to the Fizz they paired with, unless FIZZ_API overrides it.
fn config_api(config: &Map<String, Value>) -> String {
    match (
        env::var("FIZZ_API").ok().filter(|v| !v.is_empty()),
        config.get("api").and_then(Value::as_str),
    ) {
        (None, Some(api)) => api.trim_end_matches('/').to_owned(),
        _ => api_base(),
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(20)))
        .user_agent(format!("fizz-cli/{VERSION}"))
        .build()
        .into()
}

struct Reply {
    status: u16,
    body: Value,
}

impl Reply {
    fn message(&self) -> String {
        self.body
            .pointer("/error/message")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Fizz answered with HTTP {}.", self.status))
    }
}

fn post(base: &str, path: &str, body: &Value, bearer: Option<&str>) -> Result<Reply> {
    let url = format!("{base}{path}");
    let mut request = agent()
        .post(&url)
        .header("content-type", "application/json");
    if let Some(token) = bearer {
        request = request.header("authorization", &format!("Bearer {token}"));
    }
    let mut response = request
        .send(body.to_string())
        .map_err(|e| format!("Could not reach Fizz at {url}: {e}"))?;
    let status = response.status().as_u16();
    let text = response.body_mut().read_to_string().unwrap_or_default();
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);
    Ok(Reply { status, body })
}

// ---------------------------------------------------------------------------------------------
// Config

fn config_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("FIZZ_CONFIG").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let base = env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or("Set HOME or FIZZ_CONFIG so fizz knows where to keep its config.")?;
    Ok(base.join("fizz").join("device.json"))
}

fn load_config() -> Result<Option<Map<String, Value>>> {
    let path = config_path()?;
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.as_object().cloned())
            .map(Some)
            .ok_or_else(|| {
                format!(
                    "{} is not valid. Remove it and run fizz pair.",
                    path.display()
                )
            }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Could not read {}: {e}", path.display())),
    }
}

fn require_config() -> Result<Map<String, Value>> {
    load_config()?.ok_or_else(|| "This device is not paired yet. Run `fizz pair` first.".to_owned())
}

/// The config holds this device's key, so it is written with owner-only permissions.
fn save_config(config: &Value) -> Result<PathBuf> {
    let path = config_path()?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    file.write_all(
        serde_json::to_string_pretty(config)
            .unwrap_or_default()
            .as_bytes(),
    )
    .map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    Ok(path)
}

fn field<'a>(config: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    config.get(key).and_then(Value::as_str).ok_or_else(|| {
        format!("The config is missing `{key}`. Run `fizz unpair`, then `fizz pair`.")
    })
}

// ---------------------------------------------------------------------------------------------
// Device details

fn hostname() -> String {
    fs::read_to_string("/etc/hostname")
        .ok()
        .map(|h| h.trim().to_owned())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "My device".to_owned())
}

/// Board model where the kernel reports one (Raspberry Pi does), plus the CPU architecture.
fn platform() -> String {
    let model = fs::read_to_string("/proc/device-tree/model")
        .ok()
        .map(|m| m.trim_end_matches('\0').trim().to_owned())
        .filter(|m| !m.is_empty());
    let platform = match model {
        Some(model) => format!("{model} ({})", env::consts::ARCH),
        None => format!("{} {}", env::consts::OS, env::consts::ARCH),
    };
    printable(&platform, 80)
}

/// Keep names to printable characters so nothing sent to Fizz can carry terminal escapes.
fn printable(text: &str, max_chars: usize) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn event_id() -> String {
    let mut random = [0u8; 6];
    let _ =
        fs::File::open("/dev/urandom").and_then(|mut f| io::Read::read_exact(&mut f, &mut random));
    let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
    format!("cli-{}-{suffix}", now_millis())
}

// ---------------------------------------------------------------------------------------------
// Commands

fn flag_value(args: &[String], name: &str) -> Result<Option<String>> {
    match args.iter().position(|a| a == name) {
        Some(i) => args
            .get(i + 1)
            .cloned()
            .map(Some)
            .ok_or_else(|| format!("{name} needs a value.")),
        None => Ok(None),
    }
}

fn pair(args: &[String]) -> Result<()> {
    if let Some(config) = load_config()?
        && !args.iter().any(|a| a == "--force")
    {
        return Err(format!(
            "This device is already paired as \"{}\" with account {}. Run `fizz unpair` first, or pass --force.",
            field(&config, "device_name").unwrap_or("?"),
            field(&config, "account").unwrap_or("?"),
        ));
    }
    let name = printable(&flag_value(args, "--name")?.unwrap_or_else(hostname), 80);
    let base = api_base();
    let start = post(
        &base,
        "/api/devices",
        &json!({"action": "start", "name": name, "platform": platform()}),
        None,
    )?;
    if start.status != 201 {
        return Err(start.message());
    }
    let code = start.body["code"]
        .as_str()
        .ok_or("Fizz did not return a pairing code.")?
        .to_owned();
    let secret = start.body["poll_secret"]
        .as_str()
        .ok_or("Fizz did not return a pairing secret.")?
        .to_owned();

    println!("\n  Fizz pairing code:  {code}\n");
    println!("  Tell your agent:  \"Pair my device with Fizz code {code}\"");
    println!("  Or enter it on the Agents page:  {base}/agents.html\n");
    println!("  Waiting for the code to be claimed (it expires in 15 minutes)…");

    let claim = loop {
        thread::sleep(Duration::from_secs(3));
        let reply = match post(
            &base,
            "/api/devices",
            &json!({"action": "status", "poll_secret": secret}),
            None,
        ) {
            Ok(reply) => reply,
            Err(e) => {
                eprintln!("  {e} Retrying…");
                continue;
            }
        };
        if reply.status != 200 {
            return Err(reply.message());
        }
        match reply.body["status"].as_str() {
            Some("waiting") => continue,
            Some("claimed") => break reply.body,
            Some("expired") => {
                return Err("The pairing code expired. Run `fizz pair` again.".into());
            }
            Some(other) => {
                return Err(format!(
                    "Pairing ended unexpectedly ({other}). Run `fizz pair` again."
                ));
            }
            None => return Err("Fizz sent an unexpected pairing status.".into()),
        }
    };

    let account = printable(claim["account"].as_str().unwrap_or("unknown"), 40);
    let device_name = printable(claim["device_name"].as_str().unwrap_or(&name), 80);
    let requester = match claim["agent_name"].as_str() {
        Some(agent) if claim["claimed_via"] == "agent" => {
            format!("The agent \"{}\"", printable(agent, 60))
        }
        _ => "The Fizz dashboard".to_owned(),
    };
    println!(
        "\n  {requester} on Fizz account \"{account}\" wants to pair this device as \"{device_name}\"."
    );
    println!("  Only approve if \"{account}\" is your Fizz username.");
    print!("  Approve? [y/N] ");
    io::stdout().flush().ok();
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer).ok();
    let approve = matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes");

    let action = if approve { "approve" } else { "reject" };
    let decision = post(
        &base,
        "/api/devices",
        &json!({"action": action, "poll_secret": secret}),
        None,
    )?;
    if decision.status != 200 {
        return Err(decision.message());
    }
    if !approve {
        println!("  Pairing rejected. Nothing was connected.");
        return Ok(());
    }
    let body = &decision.body;
    let (Some(sensor_id), Some(api_key)) = (body["sensor_id"].as_str(), body["api_key"].as_str())
    else {
        return Err("Fizz approved the pairing but did not return this device's key.".into());
    };
    let path = save_config(&json!({
        "api": base,
        "sensor_id": sensor_id,
        "api_key": api_key,
        "device_name": body["device_name"].as_str().unwrap_or(&device_name),
        "account": body["account"].as_str().unwrap_or(&account),
        "paired_at_ms": now_millis().to_string(),
    }))?;
    println!(
        "\n  Paired with \"{account}\". This device's key is saved in {}.",
        path.display()
    );
    println!("  Start sending readings with:  fizz run");
    Ok(())
}

fn send_metrics(config: &Map<String, Value>, metrics: &Map<String, Value>) -> Result<()> {
    let reply = post(
        &config_api(config),
        "/api/ingest",
        &json!({"sensor_id": field(config, "sensor_id")?, "event_id": event_id(), "metrics": metrics}),
        Some(field(config, "api_key")?),
    )?;
    match reply.status {
        200..=299 if reply.body.get("error").is_none() => Ok(()),
        401 => Err("This device's key was revoked. Run `fizz unpair`, then `fizz pair`.".into()),
        _ => Err(reply.message()),
    }
}

fn parse_value(raw: &str) -> Result<Value> {
    match raw {
        "true" => return Ok(json!(true)),
        "false" => return Ok(json!(false)),
        _ => {}
    }
    if let Ok(number) = raw.parse::<f64>()
        && number.is_finite()
    {
        return Ok(json!(number));
    }
    if raw.chars().count() > 512 {
        return Err("Text values must be 512 characters or fewer.".into());
    }
    Ok(json!(raw))
}

fn valid_metric(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=40).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

fn send(args: &[String]) -> Result<()> {
    if args.is_empty() || !args.len().is_multiple_of(2) || args.len() > 32 {
        return Err(
            "Use `fizz send METRIC VALUE [METRIC VALUE ...]` with up to 16 metrics.".into(),
        );
    }
    let mut metrics = Map::new();
    for pair in args.chunks(2) {
        if !valid_metric(&pair[0]) {
            return Err(format!(
                "`{}` is not a metric name. Use lowercase letters, digits, and underscores, starting with a letter.",
                pair[0]
            ));
        }
        metrics.insert(pair[0].clone(), parse_value(&pair[1])?);
    }
    send_metrics(&require_config()?, &metrics)?;
    println!(
        "Sent {} reading{}.",
        metrics.len(),
        if metrics.len() == 1 { "" } else { "s" }
    );
    Ok(())
}

fn read_number(path: &str) -> Option<f64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn round(value: f64, places: i32) -> f64 {
    let factor = 10f64.powi(places);
    (value * factor).round() / factor
}

/// Built-in readings from the Linux kernel. Missing sources are simply skipped.
fn system_metrics() -> Map<String, Value> {
    let mut metrics = Map::new();
    if let Some(milli) = read_number("/sys/class/thermal/thermal_zone0/temp") {
        metrics.insert("cpu_temperature_c".into(), json!(round(milli / 1000.0, 1)));
    }
    if let Some(load) = fs::read_to_string("/proc/loadavg").ok().and_then(|l| {
        l.split_whitespace()
            .next()
            .and_then(|v| v.parse::<f64>().ok())
    }) {
        metrics.insert("load_1m".into(), json!(round(load, 2)));
    }
    if let Ok(meminfo) = fs::read_to_string("/proc/meminfo") {
        let kb = |key: &str| {
            meminfo
                .lines()
                .find(|l| l.starts_with(key))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<f64>().ok())
        };
        if let (Some(total), Some(available)) = (kb("MemTotal:"), kb("MemAvailable:"))
            && total > 0.0
        {
            metrics.insert(
                "memory_used_pct".into(),
                json!(round((total - available) / total * 100.0, 1)),
            );
        }
    }
    if let Some(used) = disk_used_pct("/") {
        metrics.insert("disk_used_pct".into(), json!(round(used, 1)));
    }
    metrics
}

#[cfg(unix)]
fn disk_used_pct(path: &str) -> Option<f64> {
    let path = std::ffi::CString::new(path).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `path` is a valid C string and `stat` is a properly sized, writable struct.
    if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    let total = stat.f_blocks as f64 * stat.f_frsize as f64;
    let free = stat.f_bavail as f64 * stat.f_frsize as f64;
    (total > 0.0).then(|| (total - free) / total * 100.0)
}

#[cfg(not(unix))]
fn disk_used_pct(_path: &str) -> Option<f64> {
    None
}

fn summary(metrics: &Map<String, Value>) -> String {
    let get = |k: &str| metrics.get(k).and_then(Value::as_f64);
    let mut parts = Vec::new();
    if let Some(v) = get("cpu_temperature_c") {
        parts.push(format!("cpu {v}°C"));
    }
    if let Some(v) = get("load_1m") {
        parts.push(format!("load {v}"));
    }
    if let Some(v) = get("memory_used_pct") {
        parts.push(format!("memory {v}%"));
    }
    if let Some(v) = get("disk_used_pct") {
        parts.push(format!("disk {v}%"));
    }
    parts.join(" · ")
}

fn run(args: &[String]) -> Result<()> {
    let every = match flag_value(args, "--every")? {
        Some(raw) => raw
            .parse::<u64>()
            .map_err(|_| "--every takes a number of seconds.".to_owned())?,
        None => 60,
    };
    if every < 15 {
        return Err("Send at most every 15 seconds (--every 15 or more).".into());
    }
    let once = args.iter().any(|a| a == "--once");
    let config = require_config()?;
    println!(
        "Sending readings from \"{}\" to account {} every {every}s. Press Ctrl-C to stop.",
        field(&config, "device_name")?,
        field(&config, "account")?
    );
    loop {
        let metrics = system_metrics();
        if metrics.is_empty() {
            return Err(
                "No built-in readings are available on this system. Use `fizz send` instead."
                    .into(),
            );
        }
        match send_metrics(&config, &metrics) {
            Ok(()) => println!("{}  sent  {}", clock(), summary(&metrics)),
            Err(e) if e.contains("revoked") => return Err(e),
            Err(e) => eprintln!("{}  not sent: {e}", clock()),
        }
        if once {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(every));
    }
}

fn clock() -> String {
    let secs = (now_millis() / 1000) as u64;
    format!(
        "{:02}:{:02}:{:02} UTC",
        secs / 3600 % 24,
        secs / 60 % 60,
        secs % 60
    )
}

fn status() -> Result<()> {
    let config = require_config()?;
    println!("Device:   {}", field(&config, "device_name")?);
    println!("Account:  {}", field(&config, "account")?);
    println!("Sensor:   {}", field(&config, "sensor_id")?);
    println!("Fizz:     {}", config_api(&config));
    println!("Config:   {}", config_path()?.display());
    Ok(())
}

fn unpair() -> Result<()> {
    let config = require_config()?;
    let reply = post(
        &config_api(&config),
        "/api/devices",
        &json!({"action": "unpair", "sensor_id": field(&config, "sensor_id")?}),
        Some(field(&config, "api_key")?),
    )?;
    // 401 means the key was already revoked from the dashboard; forgetting it is still right.
    if reply.status != 200 && reply.status != 401 {
        return Err(reply.message());
    }
    let path = config_path()?;
    fs::remove_file(&path).map_err(|e| format!("Could not remove {}: {e}", path.display()))?;
    println!(
        "Unpaired. This device's key is revoked and {} is removed.",
        path.display()
    );
    Ok(())
}

fn service() -> Result<()> {
    let binary = env::current_exe().map_err(|e| format!("Could not find the fizz binary: {e}"))?;
    let user = env::var("USER").unwrap_or_else(|_| "pi".into());
    let config = config_path()?;
    println!(
        "# Save as /etc/systemd/system/fizz.service, then run:
#   sudo systemctl daemon-reload && sudo systemctl enable --now fizz
[Unit]
Description=Fizz device readings
After=network-online.target
Wants=network-online.target

[Service]
User={user}
Environment=FIZZ_CONFIG={config}
ExecStart={binary} run --every 60
Restart=on-failure
RestartSec=30

[Install]
WantedBy=multi-user.target",
        config = config.display(),
        binary = binary.display(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_parse_as_bool_number_or_text() {
        assert_eq!(parse_value("true").unwrap(), json!(true));
        assert_eq!(parse_value("22.4").unwrap(), json!(22.4));
        assert_eq!(parse_value("-3").unwrap(), json!(-3.0));
        assert_eq!(
            parse_value("garage door open").unwrap(),
            json!("garage door open")
        );
        assert_eq!(parse_value("NaN").unwrap(), json!("NaN"));
        assert!(parse_value(&"x".repeat(513)).is_err());
    }

    #[test]
    fn metric_names_match_the_ingest_api() {
        assert!(valid_metric("temperature_c"));
        assert!(valid_metric("load_1m"));
        assert!(!valid_metric("Temperature"));
        assert!(!valid_metric("1st"));
        assert!(!valid_metric(&"a".repeat(41)));
    }

    #[test]
    fn names_drop_control_characters() {
        assert_eq!(printable("pi\u{1b}[31m-lab\n", 80), "pi[31m-lab");
        assert_eq!(printable("abcdef", 3), "abc");
    }

    #[test]
    fn system_metrics_use_the_ingest_metric_names() {
        for name in system_metrics().keys() {
            assert!(valid_metric(name), "{name}");
        }
    }
}
