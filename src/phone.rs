use http_body_util::{BodyExt, Limited};
use serde_json::{Value, json};
use vercel_runtime::{Request, Response, ResponseBody};

use crate::{
    customer::{error, reply, session_token},
    notify,
    store::Supabase,
};

fn uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

fn device_token(request: &Request) -> Option<String> {
    let token = request
        .headers()
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")?;
    (token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())).then(|| token.to_owned())
}

async fn body(request: Request, max_bytes: usize) -> Result<Value, Response<ResponseBody>> {
    if !request
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"))
    {
        return Err(error(415, "unsupported_media_type", "Send JSON."));
    }
    let bytes = Limited::new(request.into_body(), max_bytes)
        .collect()
        .await
        .map_err(|_| error(413, "too_large", "Request is too large."))?
        .to_bytes();
    serde_json::from_slice(&bytes).map_err(|_| error(400, "invalid_json", "Send valid JSON."))
}

fn respond(value: Value) -> Response<ResponseBody> {
    match value.get("error").and_then(Value::as_str) {
        Some("unauthorized") => error(
            401,
            "unauthorized",
            "The phone connection is no longer active.",
        ),
        Some("sensor_not_found") => error(404, "sensor_not_found", "Phone sensor not found."),
        Some("clip_not_found") => error(404, "clip_not_found", "Voice clip not found."),
        Some("invalid_link") => error(
            410,
            "invalid_link",
            "This pairing link expired or was already used.",
        ),
        Some("invalid_reading" | "invalid_clip" | "invalid_transcript") => {
            error(400, "invalid_data", "Invalid phone data.")
        }
        Some(_) => error(
            400,
            "invalid_request",
            "This request could not be completed.",
        ),
        None => reply(200, value, None),
    }
}

pub async fn handle(request: Request) -> Response<ResponseBody> {
    let method = request.method().as_str().to_owned();
    if method == "GET" || method == "DELETE" {
        let Some(owner) = session_token(&request) else {
            return error(401, "unauthorized", "Sign in to continue.");
        };
        let query = request.uri().query().unwrap_or("");
        if method == "GET" && query.split('&').any(|part| part == "clips=1") {
            let Ok(store) = Supabase::from_env() else {
                return error(503, "database_unavailable", "Voice clips are unavailable.");
            };
            return match store
                .rpc("fizz_phone_list_clips", json!({"p_owner_token": owner}))
                .await
            {
                Ok(value) => respond(value),
                Err(_) => error(503, "database_unavailable", "Voice clips are unavailable."),
            };
        }
        let clip = query
            .split('&')
            .find_map(|part| part.strip_prefix("clip_id="))
            .unwrap_or("");
        if method == "GET" && !clip.is_empty() {
            if !uuid(clip) {
                return error(400, "invalid_clip", "Choose a voice clip.");
            }
            let Ok(store) = Supabase::from_env() else {
                return error(503, "database_unavailable", "Voice clip is unavailable.");
            };
            return match store
                .rpc(
                    "fizz_phone_get_clip",
                    json!({"p_owner_token": owner, "p_clip_id": clip}),
                )
                .await
            {
                Ok(value) => respond(value),
                Err(_) => error(503, "database_unavailable", "Voice clip is unavailable."),
            };
        }
        let sensor = query
            .split('&')
            .find_map(|part| part.strip_prefix("sensor_id="))
            .unwrap_or("");
        if !uuid(sensor) {
            return error(400, "invalid_sensor", "Choose a phone sensor.");
        }
        let Ok(store) = Supabase::from_env() else {
            return error(503, "database_unavailable", "Phone pairing is unavailable.");
        };
        let function = if method == "GET" {
            "fizz_phone_status"
        } else {
            "fizz_phone_revoke"
        };
        return match store
            .rpc(
                function,
                json!({"p_owner_token": owner, "p_sensor_id": sensor}),
            )
            .await
        {
            Ok(value) => respond(value),
            Err(_) => error(503, "database_unavailable", "Phone pairing is unavailable."),
        };
    }
    if method != "POST" {
        return error(405, "method_not_allowed", "Use GET, POST, or DELETE.");
    }
    let owner = session_token(&request);
    let device = device_token(&request);
    let data = match body(request, 360_000).await {
        Ok(data) => data,
        Err(response) => return response,
    };
    let action = data.get("action").and_then(Value::as_str).unwrap_or("");
    let (function, payload) = match action {
        "create" => {
            let Some(owner) = owner else {
                return error(401, "unauthorized", "Sign in to continue.");
            };
            let sensor = data.get("sensor_id").and_then(Value::as_str).unwrap_or("");
            if !uuid(sensor) {
                return error(400, "invalid_sensor", "Choose a phone sensor.");
            }
            (
                "fizz_phone_create_pairing",
                json!({"p_owner_token": owner, "p_sensor_id": sensor}),
            )
        }
        "claim" => {
            let token = data.get("token").and_then(Value::as_str).unwrap_or("");
            if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
                return error(
                    410,
                    "invalid_link",
                    "This pairing link expired or was already used.",
                );
            }
            ("fizz_phone_claim", json!({"p_pairing_token": token}))
        }
        "reading" => {
            let Some(device) = device else {
                return error(401, "unauthorized", "Connect this phone first.");
            };
            let metric = data.get("metric").and_then(Value::as_str).unwrap_or("");
            let event = data.get("event_id").and_then(Value::as_str).unwrap_or("");
            let value = data.get("value").and_then(Value::as_f64);
            if !matches!(
                metric,
                "acceleration_x"
                    | "acceleration_y"
                    | "acceleration_z"
                    | "rotation_alpha"
                    | "rotation_beta"
                    | "rotation_gamma"
                    | "latitude"
                    | "longitude"
                    | "location_accuracy"
                    | "voice_level"
            ) || !(1..=100).contains(&event.len())
                || value.is_none_or(|v| !v.is_finite() || v.abs() > 100000.0)
            {
                return error(400, "invalid_reading", "Invalid phone reading.");
            }
            let unit = if metric.starts_with("acceleration") {
                "m/s²"
            } else if metric.starts_with("rotation") || matches!(metric, "latitude" | "longitude") {
                "degrees"
            } else if metric == "location_accuracy" {
                "meters"
            } else {
                "level"
            };
            (
                "fizz_phone_reading",
                json!({"p_device_token": device, "p_event_id": event, "p_metric": metric,
                "p_numeric_value": value, "p_unit": unit}),
            )
        }
        "clip" => {
            let Some(device) = device else {
                return error(401, "unauthorized", "Connect this phone first.");
            };
            let mime = data.get("mime_type").and_then(Value::as_str).unwrap_or("");
            let audio = data
                .get("audio_base64")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !matches!(
                mime,
                "audio/webm" | "audio/webm;codecs=opus" | "audio/mp4" | "audio/ogg;codecs=opus"
            ) || audio.len() > 349528
            {
                return error(
                    400,
                    "invalid_clip",
                    "The audio clip is too large or unsupported.",
                );
            }
            (
                "fizz_phone_clip",
                json!({"p_device_token": device, "p_mime_type": mime, "p_audio_base64": audio}),
            )
        }
        "disconnect" => {
            let Some(device) = device else {
                return error(401, "unauthorized", "Connect this phone first.");
            };
            ("fizz_phone_disconnect", json!({"p_device_token": device}))
        }
        "transcript" => {
            let Some(device) = device else {
                return error(401, "unauthorized", "Connect this phone first.");
            };
            let event = data.get("event_id").and_then(Value::as_str).unwrap_or("");
            let transcript = data
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            if !(1..=100).contains(&event.len()) || !(1..=500).contains(&transcript.len()) {
                return error(400, "invalid_transcript", "Enter up to 500 characters.");
            }
            (
                "fizz_phone_transcript",
                json!({"p_device_token": device, "p_event_id": event, "p_text": transcript}),
            )
        }
        _ => return error(400, "invalid_action", "Choose a phone action."),
    };
    let Ok(store) = Supabase::from_env() else {
        return error(503, "database_unavailable", "Phone pairing is unavailable.");
    };
    match store.rpc(function, payload).await {
        Ok(mut value) => {
            if action == "reading" && value.get("error").is_none() {
                notify::dispatch(&store).await;
            }
            if action == "create" {
                if let Some(token) = value.get("token").and_then(Value::as_str) {
                    value["url"] = json!(format!("/phone.html#pair={token}"));
                    value.as_object_mut().unwrap().remove("token");
                }
            }
            respond(value)
        }
        Err(_) => error(503, "database_unavailable", "Phone pairing is unavailable."),
    }
}
