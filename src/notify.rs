use std::{env, time::Duration};

use reqwest::{Client, StatusCode};
use serde_json::{Value, json};

use crate::store::Supabase;

const TWILIO_API: &str = "https://api.twilio.com/2010-04-01/Accounts";

/// Normalize a US phone number to E.164 (`+1NXXNXXXXXX`), or reject it.
pub fn us_phone(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty()
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_digit() || " +-().".contains(c))
    {
        return None;
    }
    let digits: String = trimmed.chars().filter(char::is_ascii_digit).collect();
    let national = match digits.len() {
        10 => digits.as_str(),
        11 if digits.starts_with('1') => &digits[1..],
        _ => return None,
    };
    let bytes = national.as_bytes();
    // Area codes and exchanges never start with 0 or 1 in the North American plan.
    if bytes[0] < b'2' || bytes[3] < b'2' {
        return None;
    }
    Some(format!("+1{national}"))
}

fn metric_name(metric: &str) -> String {
    match metric {
        "temperature_c" => "temperature".into(),
        "flow_l_min" => "flow rate".into(),
        "concentration_ppm" => "gas concentration".into(),
        "rssi_dbm" => "signal strength".into(),
        "pressure_kpa" => "pressure".into(),
        "humidity_pct" => "humidity".into(),
        "sound_db" => "sound level".into(),
        "acceleration_ms2" => "acceleration".into(),
        "rotation_alpha" => "compass heading".into(),
        "rotation_beta" => "front to back tilt".into(),
        "rotation_gamma" => "left to right tilt".into(),
        "location_accuracy" => "location accuracy".into(),
        other => other.replace('_', " "),
    }
}

fn number(value: f64) -> String {
    let text = format!("{:.2}", value);
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn written(value: f64, unit: &str) -> String {
    match unit {
        "" => number(value),
        "%" => format!("{}%", number(value)),
        _ => format!("{} {unit}", number(value)),
    }
}

fn spoken(value: f64, unit: &str) -> String {
    let unit = match unit {
        "°C" | "C" => "degrees Celsius",
        "°F" | "F" => "degrees Fahrenheit",
        "%" => "percent",
        "L/min" => "liters per minute",
        "ppm" => "parts per million",
        "dB" => "decibels",
        "dBm" => "d B m",
        "kPa" => "kilopascals",
        "m/s²" => "meters per second squared",
        other => other,
    };
    format!("{} {unit}", number(value)).trim_end().to_owned()
}

fn comparison(comparator: &str) -> &'static str {
    match comparator {
        "gt" => "above",
        "gte" => "at or above",
        "lt" => "below",
        _ => "at or below",
    }
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Text message and spoken call script for a triggered alert.
pub fn alert_messages(alert: &Value) -> (String, String) {
    let sensor: String = alert
        .get("sensor_name")
        .and_then(Value::as_str)
        .unwrap_or("A sensor")
        .chars()
        .take(60)
        .collect();
    let metric = metric_name(
        alert
            .get("metric")
            .and_then(Value::as_str)
            .unwrap_or("value"),
    );
    let value = alert
        .get("numeric_value")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let threshold = alert
        .get("threshold")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let unit = alert.get("unit").and_then(Value::as_str).unwrap_or("");
    let comparator = comparison(
        alert
            .get("comparator")
            .and_then(Value::as_str)
            .unwrap_or(""),
    );
    let text = format!(
        "Fizz alert: {sensor} {metric} is {}, {comparator} your {} limit.",
        written(value, unit),
        written(threshold, unit)
    );
    let speech = format!(
        "Fizz alert. {sensor} {metric} is {}, {comparator} your {} limit.",
        spoken(value, unit),
        spoken(threshold, unit)
    );
    let twiml = format!(
        "<Response><Say voice=\"Polly.Joanna\">{}</Say></Response>",
        xml_escape(&speech)
    );
    (text, twiml)
}

enum Failure {
    Retry(String),
    Permanent(String),
}

struct Twilio {
    client: Client,
    account: String,
    user: String,
    secret: String,
    from: Option<String>,
}

impl Twilio {
    fn from_env() -> Option<Self> {
        let read = |name: &str| {
            env::var(name)
                .ok()
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        let user = read("TWILIO_SID")?;
        let secret = read("TWILIO_CLIENT_SECRET")?;
        // TWILIO_SID may be the Account SID itself, or an API key SID used with TWILIO_ACCOUNT_SID.
        let account = if user.starts_with("AC") {
            user.clone()
        } else {
            read("TWILIO_ACCOUNT_SID").filter(|sid| sid.starts_with("AC"))?
        };
        let client = Client::builder()
            .timeout(Duration::from_secs(8))
            .build()
            .ok()?;
        Some(Self {
            client,
            account,
            user,
            secret,
            from: read("TWILIO_FROM_NUMBER"),
        })
    }

    async fn sender(&mut self) -> Result<String, Failure> {
        if let Some(from) = &self.from {
            return Ok(from.clone());
        }
        // Without TWILIO_FROM_NUMBER, use the first voice and SMS capable number on the account.
        let response = self
            .client
            .get(format!(
                "{TWILIO_API}/{}/IncomingPhoneNumbers.json?PageSize=20",
                self.account
            ))
            .basic_auth(&self.user, Some(&self.secret))
            .send()
            .await
            .map_err(|_| Failure::Retry("Twilio is unreachable.".into()))?;
        if !response.status().is_success() {
            return Err(Failure::Retry(format!(
                "Twilio number lookup failed ({}).",
                response.status().as_u16()
            )));
        }
        let numbers: Value = response
            .json()
            .await
            .map_err(|_| Failure::Retry("Twilio number lookup failed.".into()))?;
        let capable = |n: &&Value, key: &str| {
            n.pointer(&format!("/capabilities/{key}")) == Some(&json!(true))
        };
        let from = numbers
            .get("incoming_phone_numbers")
            .and_then(Value::as_array)
            .and_then(|list| {
                list.iter()
                    .find(|n| capable(n, "sms") && capable(n, "voice"))
                    .or_else(|| list.first())
            })
            .and_then(|n| n.get("phone_number"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                Failure::Permanent("No Twilio phone number is available to send from.".into())
            })?;
        self.from = Some(from.clone());
        Ok(from)
    }

    async fn send(&mut self, channel: &str, to: &str, alert: &Value) -> Result<(), Failure> {
        let from = self.sender().await?;
        let (text, twiml) = alert_messages(alert);
        let (resource, content) = match channel {
            "sms" => ("Messages", ("Body", text)),
            "call" => ("Calls", ("Twiml", twiml)),
            _ => return Err(Failure::Permanent("Unknown notification type.".into())),
        };
        let response = self
            .client
            .post(format!("{TWILIO_API}/{}/{resource}.json", self.account))
            .basic_auth(&self.user, Some(&self.secret))
            .form(&[
                ("To", to),
                ("From", from.as_str()),
                (content.0, content.1.as_str()),
            ])
            .send()
            .await
            .map_err(|_| Failure::Retry("Twilio is unreachable.".into()))?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let detail: Value = response.json().await.unwrap_or(Value::Null);
        let message = format!(
            "Twilio rejected the {} ({}): {}",
            if channel == "sms" { "text" } else { "call" },
            detail
                .get("code")
                .and_then(Value::as_i64)
                .unwrap_or(status.as_u16().into()),
            detail
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
        );
        if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            Err(Failure::Retry(message))
        } else {
            Err(Failure::Permanent(message))
        }
    }
}

/// Send queued alert texts and calls. Reading writers call this after storing data, so a
/// notification goes out on the same request that triggered it. Failures never affect ingestion.
pub async fn dispatch(store: &Supabase) {
    // Skip claiming when Twilio is not configured, e.g. on preview deployments sharing the database.
    let Some(mut twilio) = Twilio::from_env() else {
        return;
    };
    let Ok(batch) = store
        .rpc("fizz_claim_alert_notifications", json!({"p_limit":3}))
        .await
    else {
        return;
    };
    for alert in batch.as_array().into_iter().flatten() {
        let Some(id) = alert.get("id").and_then(Value::as_str) else {
            continue;
        };
        let channel = alert.get("channel").and_then(Value::as_str).unwrap_or("");
        let phone = alert.get("phone").and_then(Value::as_str).unwrap_or("");
        let (sent, retry, problem) = match twilio.send(channel, phone, alert).await {
            Ok(()) => (true, false, None),
            Err(Failure::Retry(message)) => (false, true, Some(message)),
            Err(Failure::Permanent(message)) => (false, false, Some(message)),
        };
        if let Some(message) = &problem {
            eprintln!("Fizz alert notification {id} failed: {message}");
        }
        let _ = store
            .rpc(
                "fizz_finish_alert_notification",
                json!({"p_event_id":id,"p_sent":sent,"p_retry":retry,"p_error":problem}),
            )
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_us_phone_numbers() {
        assert_eq!(us_phone("(415) 555-0123").as_deref(), Some("+14155550123"));
        assert_eq!(us_phone("+1 415.555.0123").as_deref(), Some("+14155550123"));
        assert_eq!(us_phone("14155550123").as_deref(), Some("+14155550123"));
        assert_eq!(us_phone("415-055-0123"), None);
        assert_eq!(us_phone("115-555-0123"), None);
        assert_eq!(us_phone("+44 20 7946 0958"), None);
        assert_eq!(us_phone("415555012"), None);
        assert_eq!(us_phone("415-555-0123 ext 4"), None);
    }

    #[test]
    fn alert_messages_are_short_and_escaped() {
        let alert = json!({"sensor_name":"Kitchen <oven> & stove","metric":"temperature_c","numeric_value":31.237,"threshold":30.0,"comparator":"gt","unit":"°C"});
        let (text, twiml) = alert_messages(&alert);
        assert_eq!(
            text,
            "Fizz alert: Kitchen <oven> & stove temperature is 31.24 °C, above your 30 °C limit."
        );
        assert_eq!(
            twiml,
            "<Response><Say voice=\"Polly.Joanna\">Fizz alert. Kitchen &lt;oven&gt; &amp; stove temperature is 31.24 degrees Celsius, above your 30 degrees Celsius limit.</Say></Response>"
        );
        let (unitless, _) = alert_messages(
            &json!({"sensor_name":"Bike rack","metric":"value","numeric_value":2.0,"threshold":3.0,"comparator":"lte","unit":""}),
        );
        assert_eq!(
            unitless,
            "Fizz alert: Bike rack value is 2, at or below your 3 limit."
        );
    }
}
