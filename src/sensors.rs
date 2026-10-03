use http_body_util::{BodyExt, Limited};
use serde_json::{Value, json};
use vercel_runtime::{Request, Response, ResponseBody};

use crate::{
    customer::{error, reply, session_token},
    store::Supabase,
};

const KINDS: [&str; 9] = [
    "water",
    "gas",
    "radio",
    "temperature",
    "pressure",
    "humidity",
    "sound",
    "phone",
    "custom",
];

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

async fn body(request: Request, max: usize) -> Result<Value, Response<ResponseBody>> {
    let json_type = request
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    if !json_type {
        return Err(error(415, "unsupported_media_type", "Send JSON."));
    }
    let bytes = Limited::new(request.into_body(), max)
        .collect()
        .await
        .map_err(|_| error(413, "request_too_large", "Request is too large."))?
        .to_bytes();
    serde_json::from_slice(&bytes).map_err(|_| error(400, "invalid_request", "Send valid JSON."))
}

fn db() -> Result<Supabase, Response<ResponseBody>> {
    Supabase::from_env().map_err(|_| {
        error(
            503,
            "database_unavailable",
            "Sensor data is temporarily unavailable.",
        )
    })
}

fn outcome(value: Value, success: u16) -> Response<ResponseBody> {
    match value.get("error").and_then(Value::as_str) {
        Some("unauthorized") => error(401, "unauthorized", "Sign in to continue."),
        Some("not_found") => error(404, "not_found", "Sensor not found."),
        Some("invalid_request" | "invalid_reading") => {
            error(400, "invalid_request", "Check the sensor details.")
        }
        Some(_) => error(
            503,
            "database_unavailable",
            "Sensor data is temporarily unavailable.",
        ),
        None => reply(success, value, None),
    }
}

pub async fn list(request: Request) -> Response<ResponseBody> {
    let Some(token) = session_token(&request) else {
        return error(401, "unauthorized", "Sign in to continue.");
    };
    let Ok(store) = db() else {
        return error(
            503,
            "database_unavailable",
            "Sensor data is temporarily unavailable.",
        );
    };
    match store
        .rpc("fizz_tick_simulated", json!({"p_token":token}))
        .await
    {
        Ok(v) if v.get("error").is_some() => return outcome(v, 200),
        Err(_) => {
            return error(
                503,
                "database_unavailable",
                "Sensor data is temporarily unavailable.",
            );
        }
        _ => {}
    }
    match store
        .rpc("fizz_list_sensors", json!({"p_token":token}))
        .await
    {
        Ok(v) => outcome(v, 200),
        Err(_) => error(
            503,
            "database_unavailable",
            "Sensor data is temporarily unavailable.",
        ),
    }
}

pub async fn create(request: Request) -> Response<ResponseBody> {
    let Some(token) = session_token(&request) else {
        return error(401, "unauthorized", "Sign in to continue.");
    };
    let data = match body(request, 2048).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let kind = data.get("kind").and_then(Value::as_str).unwrap_or("");
    let mode = data
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("simulated");
    let default_name = format!("{} sensor", kind.split_once('_').map_or(kind, |(s, _)| s));
    let name = data
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(&default_name)
        .trim();
    if !KINDS.contains(&kind)
        || !matches!(mode, "simulated" | "api" | "phone")
        || (mode == "phone" && kind != "phone")
        || name.is_empty()
        || name.chars().count() > 80
    {
        return error(
            400,
            "invalid_request",
            "Choose a valid sensor type, mode, and name.",
        );
    }
    let Ok(store) = db() else {
        return error(
            503,
            "database_unavailable",
            "Sensor data is temporarily unavailable.",
        );
    };
    match store
        .rpc(
            "fizz_create_sensor",
            json!({"p_token":token,"p_kind":kind,"p_name":name,"p_mode":mode}),
        )
        .await
    {
        Ok(v) => outcome(v, 201),
        Err(_) => error(
            503,
            "database_unavailable",
            "Sensor data is temporarily unavailable.",
        ),
    }
}

pub async fn update(request: Request) -> Response<ResponseBody> {
    let Some(token) = session_token(&request) else {
        return error(401, "unauthorized", "Sign in to continue.");
    };
    let data = match body(request, 2048).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let id = data.get("id").and_then(Value::as_str).unwrap_or("");
    let name = data.get("name").and_then(Value::as_str);
    let mode = data.get("mode").and_then(Value::as_str);
    let status = data.get("status").and_then(Value::as_str);
    let rotate = data
        .get("rotate_key")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !valid_uuid(id)
        || name.is_some_and(|n| n.trim().is_empty() || n.chars().count() > 80)
        || mode.is_some_and(|m| !matches!(m, "simulated" | "api" | "phone"))
        || status.is_some_and(|s| !matches!(s, "active" | "paused"))
    {
        return error(400, "invalid_request", "Check the sensor details.");
    }
    let Ok(store) = db() else {
        return error(
            503,
            "database_unavailable",
            "Sensor data is temporarily unavailable.",
        );
    };
    match store.rpc("fizz_update_sensor",json!({"p_token":token,"p_id":id,"p_name":name,"p_mode":mode,"p_status":status,"p_rotate_key":rotate})).await {
        Ok(v)=>outcome(v,200),Err(_)=>error(503,"database_unavailable","Sensor data is temporarily unavailable.")
    }
}

pub async fn delete(request: Request) -> Response<ResponseBody> {
    let Some(token) = session_token(&request) else {
        return error(401, "unauthorized", "Sign in to continue.");
    };
    let data = match body(request, 1024).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let id = data.get("id").and_then(Value::as_str).unwrap_or("");
    if !valid_uuid(id) {
        return error(400, "invalid_request", "Choose a sensor to remove.");
    }
    let Ok(store) = db() else {
        return error(
            503,
            "database_unavailable",
            "Sensor data is temporarily unavailable.",
        );
    };
    match store
        .rpc("fizz_delete_sensor", json!({"p_token":token,"p_id":id}))
        .await
    {
        Ok(v) => outcome(v, 200),
        Err(_) => error(
            503,
            "database_unavailable",
            "Sensor data is temporarily unavailable.",
        ),
    }
}

pub async fn readings(request: Request) -> Response<ResponseBody> {
    let Some(token) = session_token(&request) else {
        return error(401, "unauthorized", "Sign in to continue.");
    };
    let query = request.uri().query().unwrap_or("");
    let find = |key: &str| {
        query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v)
    };
    let id = find("sensor_id").unwrap_or("");
    let limit = find("limit")
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(100)
        .clamp(1, 500);
    if !valid_uuid(id) {
        return error(400, "invalid_request", "Choose a sensor.");
    }
    let Ok(store) = db() else {
        return error(
            503,
            "database_unavailable",
            "Sensor data is temporarily unavailable.",
        );
    };
    match store
        .rpc("fizz_tick_simulated", json!({"p_token":token}))
        .await
    {
        Ok(v) if v.get("error").is_some() => return outcome(v, 200),
        Err(_) => {
            return error(
                503,
                "database_unavailable",
                "Sensor data is temporarily unavailable.",
            );
        }
        _ => {}
    }
    match store
        .rpc(
            "fizz_list_readings",
            json!({"p_token":token,"p_sensor_id":id,"p_limit":limit}),
        )
        .await
    {
        Ok(v) => outcome(v, 200),
        Err(_) => error(
            503,
            "database_unavailable",
            "Sensor data is temporarily unavailable.",
        ),
    }
}

pub async fn ingest(request: Request) -> Response<ResponseBody> {
    let key = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("")
        .to_owned();
    if key.len() != 69
        || !key.starts_with("fizz_")
        || !key[5..].bytes().all(|c| c.is_ascii_hexdigit())
    {
        return error(401, "unauthorized", "Use the sensor API key.");
    }
    let data = match body(request, 16 * 1024).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let id = data.get("sensor_id").and_then(Value::as_str).unwrap_or("");
    let event = data.get("event_id").and_then(Value::as_str).unwrap_or("");
    let observed = data.get("observed_at").and_then(Value::as_str);
    let Some(metrics) = data.get("metrics").and_then(Value::as_object) else {
        return error(400, "invalid_request", "Send a metrics object.");
    };
    if !valid_uuid(id)
        || event.is_empty()
        || event.len() > 128
        || metrics.is_empty()
        || metrics.len() > 16
    {
        return error(
            400,
            "invalid_request",
            "Check the sensor ID, event ID, and metrics.",
        );
    }
    for (metric, value) in metrics {
        if metric.len() > 40
            || !metric.bytes().enumerate().all(|(i, c)| {
                if i == 0 {
                    c.is_ascii_lowercase()
                } else {
                    c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_'
                }
            })
        {
            return error(
                400,
                "invalid_request",
                "Metric names must use lowercase letters, digits, and underscores.",
            );
        }
        if !matches!(value, Value::Number(_) | Value::Bool(_))
            && !matches!(value, Value::String(s) if s.len() <= 512)
        {
            return error(
                400,
                "invalid_request",
                "Metric values must be numbers, booleans, or short text.",
            );
        }
    }
    let Ok(store) = db() else {
        return error(
            503,
            "database_unavailable",
            "Ingestion is temporarily unavailable.",
        );
    };
    match store
        .rpc(
            "fizz_ingest_event",
            json!({"p_sensor_id":id,"p_key":key,"p_event_id":event,
            "p_observed_at":observed,"p_metrics":metrics}),
        )
        .await
    {
        Ok(v) if v.get("error").and_then(Value::as_str) == Some("unauthorized") => {
            error(401, "unauthorized", "Sensor key is invalid or revoked.")
        }
        Ok(v) if v.get("error").is_some() => outcome(v, 202),
        Ok(v) => reply(202, v, None),
        Err(_) => error(
            503,
            "database_unavailable",
            "Ingestion is temporarily unavailable.",
        ),
    }
}
