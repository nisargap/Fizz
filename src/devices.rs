//! Device pairing for the Fizz CLI. Each action names its own credential:
//! `start` is anonymous and rate-limited by IP, `status`/`approve`/`reject` use the device's
//! poll secret, `claim` uses the dashboard session, and `unpair` uses the device's sensor key.

use serde_json::{Value, json};
use vercel_runtime::{Request, Response, ResponseBody};

use crate::{
    customer::{client_ip, error, json_body, reply, session_token},
    sensors::valid_uuid,
    store::Supabase,
};

fn valid_secret(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// Short display text from the device: trimmed, bounded, no control characters.
fn label(value: Option<&Value>, max_chars: usize) -> Option<String> {
    let text = value.and_then(Value::as_str)?.trim();
    (text.chars().count() <= max_chars && !text.chars().any(char::is_control))
        .then(|| text.to_owned())
}

fn sensor_key(request: &Request) -> Option<String> {
    let key = request
        .headers()
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")?;
    (key.len() == 69 && key.starts_with("fizz_") && key[5..].bytes().all(|c| c.is_ascii_hexdigit()))
        .then(|| key.to_owned())
}

fn outcome(
    result: Result<Value, crate::store::StoreError>,
    success: u16,
) -> Response<ResponseBody> {
    match result {
        Ok(value) => match value.get("error").and_then(Value::as_str) {
            None => reply(success, value, None),
            Some("unauthorized") => {
                error(401, "unauthorized", "Sign in, or use this device's key.")
            }
            Some("rate_limited") => error(
                429,
                "rate_limited",
                "Too many pairing attempts. Try again in an hour.",
            ),
            Some("invalid_code") => error(
                404,
                "invalid_code",
                "That code is not valid or has expired. Run fizz pair for a new one.",
            ),
            Some("expired") => error(
                410,
                "expired",
                "This pairing code expired. Run fizz pair again.",
            ),
            Some("not_claimed") => error(409, "not_claimed", "Nobody has claimed this code yet."),
            Some("not_found") => error(404, "not_found", "This pairing was not found."),
            Some(_) => error(503, "unavailable", "Pairing is temporarily unavailable."),
        },
        Err(_) => error(
            503,
            "database_unavailable",
            "Pairing is temporarily unavailable.",
        ),
    }
}

pub async fn handle(request: Request) -> Response<ResponseBody> {
    if request.method() != "POST" {
        return error(405, "method_not_allowed", "Use POST.");
    }
    let ip = client_ip(&request);
    let session = session_token(&request);
    let key = sensor_key(&request);
    let data = match json_body(request, 2048).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let Ok(store) = Supabase::from_env() else {
        return error(
            503,
            "database_unavailable",
            "Pairing is temporarily unavailable.",
        );
    };
    let secret = data
        .get("poll_secret")
        .and_then(Value::as_str)
        .filter(|s| valid_secret(s));
    match data.get("action").and_then(Value::as_str).unwrap_or("") {
        "start" => {
            let (Some(name), Some(platform)) = (
                label(data.get("name").or(Some(&json!(""))), 80),
                label(data.get("platform").or(Some(&json!(""))), 80),
            ) else {
                return error(
                    400,
                    "invalid_request",
                    "Use a device name and platform under 80 characters.",
                );
            };
            outcome(
                store
                    .rpc(
                        "fizz_pair_start",
                        json!({"p_name": name, "p_platform": platform, "p_ip": ip}),
                    )
                    .await,
                201,
            )
        }
        "status" => {
            let Some(secret) = secret else {
                return error(
                    400,
                    "invalid_request",
                    "Send the poll secret from fizz pair.",
                );
            };
            outcome(
                store
                    .rpc("fizz_pair_status", json!({"p_poll_secret": secret}))
                    .await,
                200,
            )
        }
        action @ ("approve" | "reject") => {
            let Some(secret) = secret else {
                return error(
                    400,
                    "invalid_request",
                    "Send the poll secret from fizz pair.",
                );
            };
            outcome(
                store
                    .rpc(
                        "fizz_pair_decide",
                        json!({"p_poll_secret": secret, "p_approve": action == "approve"}),
                    )
                    .await,
                200,
            )
        }
        "claim" => {
            let Some(session) = session else {
                return error(401, "unauthorized", "Sign in to pair a device.");
            };
            let code = data
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            if code.is_empty() || code.len() > 24 {
                return error(400, "invalid_code", "Enter the code shown by fizz pair.");
            }
            let Some(name) = label(data.get("name").or(Some(&json!(""))), 80) else {
                return error(
                    400,
                    "invalid_request",
                    "Use a device name under 80 characters.",
                );
            };
            outcome(
                store
                    .rpc(
                        "fizz_pair_claim",
                        json!({"p_token": session, "p_code": code, "p_device_name": name}),
                    )
                    .await,
                200,
            )
        }
        "unpair" => {
            let id = data.get("sensor_id").and_then(Value::as_str).unwrap_or("");
            let (Some(key), true) = (key, valid_uuid(id)) else {
                return error(401, "unauthorized", "Use this device's sensor ID and key.");
            };
            outcome(
                store
                    .rpc(
                        "fizz_device_unpair",
                        json!({"p_sensor_id": id, "p_key": key}),
                    )
                    .await,
                200,
            )
        }
        _ => error(400, "invalid_request", "Unknown pairing action."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_labels_are_bounded_and_printable() {
        assert_eq!(
            label(Some(&json!("  raspberrypi ")), 80).as_deref(),
            Some("raspberrypi")
        );
        assert_eq!(label(Some(&json!("Pi\u{1b}[31m")), 80), None);
        assert_eq!(label(Some(&json!("x".repeat(81))), 80), None);
        assert_eq!(label(None, 80), None);
    }

    #[test]
    fn poll_secrets_are_64_lowercase_hex() {
        assert!(valid_secret(&"0f".repeat(32)));
        assert!(!valid_secret(&"0F".repeat(32)));
        assert!(!valid_secret("abc"));
    }
}
