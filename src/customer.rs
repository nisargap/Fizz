use http_body_util::{BodyExt, Limited};
use serde_json::{Value, json};
use vercel_runtime::{Request, Response, ResponseBody};

use crate::store::{StoreError, Supabase};

const COOKIE_NAME: &str = "fizz_session";

pub fn reply(status: u16, body: Value, cookie: Option<&str>) -> Response<ResponseBody> {
    let mut builder = Response::builder()
        .status(status)
        .header("content-type", "application/json; charset=utf-8")
        .header("cache-control", "no-store")
        .header("x-content-type-options", "nosniff");
    if let Some(cookie) = cookie {
        builder = builder.header("set-cookie", cookie);
    }
    builder
        .body(ResponseBody::from(body))
        .expect("valid response")
}

pub fn error(status: u16, code: &str, message: &str) -> Response<ResponseBody> {
    reply(
        status,
        json!({"error": {"code": code, "message": message}}),
        None,
    )
}

pub async fn credentials(request: Request) -> Result<(String, String), Response<ResponseBody>> {
    let is_json = request
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    if !is_json {
        return Err(error(415, "unsupported_media_type", "Send JSON."));
    }
    let bytes = Limited::new(request.into_body(), 1024)
        .collect()
        .await
        .map_err(|_| error(413, "invalid_request", "Request is too large."))?
        .to_bytes();
    let data: Value = serde_json::from_slice(&bytes).map_err(|_| {
        error(
            400,
            "invalid_request",
            "Enter a username and six digit code.",
        )
    })?;
    let username = data
        .get("username")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let code = data
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let valid_username = (3..=24).contains(&username.len())
        && username.as_bytes()[0].is_ascii_lowercase()
        && username
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_');
    let valid_code = code.len() == 6 && code.bytes().all(|c| c.is_ascii_digit());
    if !valid_username || !valid_code {
        return Err(error(
            400,
            "invalid_credentials",
            "Use 3–24 lowercase letters, numbers, or underscores for the username and exactly six digits for the code.",
        ));
    }
    Ok((username, code))
}

pub fn session_token(request: &Request) -> Option<String> {
    request
        .headers()
        .get("cookie")?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE_NAME)
        .map(|(_, value)| value.to_owned())
        .filter(|value| value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit()))
}

fn session_cookie(token: &str) -> String {
    format!("{COOKIE_NAME}={token}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=43200")
}

pub fn clear_cookie() -> &'static str {
    "fizz_session=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0"
}

pub async fn register(request: Request) -> Response<ResponseBody> {
    let (username, code) = match credentials(request).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let Ok(store) = Supabase::from_env() else {
        return error(
            503,
            "database_unavailable",
            "Account setup is temporarily unavailable.",
        );
    };
    match store
        .rpc(
            "fizz_register",
            json!({"p_username": username, "p_code": code}),
        )
        .await
    {
        Ok(value) => account_reply(value),
        Err(StoreError::Conflict) => {
            error(409, "username_taken", "That username is already taken.")
        }
        Err(_) => error(
            503,
            "database_unavailable",
            "Account setup is temporarily unavailable.",
        ),
    }
}

pub async fn sign_in(request: Request) -> Response<ResponseBody> {
    let (username, code) = match credentials(request).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let Ok(store) = Supabase::from_env() else {
        return error(
            503,
            "database_unavailable",
            "Sign-in is temporarily unavailable.",
        );
    };
    match store
        .rpc(
            "fizz_sign_in",
            json!({"p_username": username, "p_code": code}),
        )
        .await
    {
        Ok(value) if value.get("error").and_then(Value::as_str) == Some("rate_limited") => error(
            429,
            "rate_limited",
            "Too many attempts. Try again in 15 minutes.",
        ),
        Ok(value) if value.get("error").is_some() => {
            error(401, "invalid_credentials", "Username or code is incorrect.")
        }
        Ok(value) => account_reply(value),
        Err(_) => error(
            503,
            "database_unavailable",
            "Sign-in is temporarily unavailable.",
        ),
    }
}

fn account_reply(value: Value) -> Response<ResponseBody> {
    let (Some(username), Some(token)) = (
        value.get("username").and_then(Value::as_str),
        value.get("token").and_then(Value::as_str),
    ) else {
        return error(
            503,
            "database_unavailable",
            "Account setup is temporarily unavailable.",
        );
    };
    reply(
        200,
        json!({"ok": true, "username": username}),
        Some(&session_cookie(token)),
    )
}

pub async fn current(request: Request) -> Response<ResponseBody> {
    let Some(token) = session_token(&request) else {
        return error(401, "unauthorized", "Sign in to continue.");
    };
    let Ok(store) = Supabase::from_env() else {
        return error(503, "database_unavailable", "Session check is unavailable.");
    };
    match store
        .rpc("fizz_current_customer", json!({"p_token": token}))
        .await
    {
        Ok(value) if value.get("username").is_some() => reply(200, value, None),
        Ok(_) => error(401, "unauthorized", "Sign in to continue."),
        Err(_) => error(503, "database_unavailable", "Session check is unavailable."),
    }
}

pub async fn sign_out(request: Request) -> Response<ResponseBody> {
    if let Some(token) = session_token(&request) {
        let Ok(store) = Supabase::from_env() else {
            return error(
                503,
                "database_unavailable",
                "Sign-out is temporarily unavailable.",
            );
        };
        if store
            .rpc("fizz_sign_out", json!({"p_token": token}))
            .await
            .is_err()
        {
            return error(
                503,
                "database_unavailable",
                "Sign-out is temporarily unavailable.",
            );
        }
    }
    reply(200, json!({"ok": true}), Some(clear_cookie()))
}

pub async fn sensor_choices(request: Request) -> Response<ResponseBody> {
    let Some(token) = session_token(&request) else {
        return error(401, "unauthorized", "Sign in to continue.");
    };
    let Ok(store) = Supabase::from_env() else {
        return error(
            503,
            "database_unavailable",
            "Sensor setup is temporarily unavailable.",
        );
    };
    match store
        .rpc("fizz_get_sensor_choices", json!({"p_token": token}))
        .await
    {
        Ok(value) if value.get("error").and_then(Value::as_str) == Some("unauthorized") => {
            error(401, "unauthorized", "Sign in to continue.")
        }
        Ok(value) if value.get("choices").and_then(Value::as_array).is_some() => {
            reply(200, value, None)
        }
        _ => error(
            503,
            "database_unavailable",
            "Sensor setup is temporarily unavailable.",
        ),
    }
}

pub async fn select_sensor(request: Request) -> Response<ResponseBody> {
    let Some(token) = session_token(&request) else {
        return error(401, "unauthorized", "Sign in to continue.");
    };
    let is_json = request
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/json"));
    if !is_json {
        return error(415, "unsupported_media_type", "Send JSON.");
    }
    let bytes = match Limited::new(request.into_body(), 512).collect().await {
        Ok(body) => body.to_bytes(),
        Err(_) => return error(413, "invalid_request", "Request is too large."),
    };
    let data: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return error(400, "invalid_request", "Choose a sensor type."),
    };
    let kind = data.get("kind").and_then(Value::as_str).unwrap_or("");
    if !matches!(
        kind,
        "water"
            | "gas"
            | "radio"
            | "temperature"
            | "pressure"
            | "humidity"
            | "sound"
            | "phone"
            | "custom"
    ) {
        return error(
            400,
            "invalid_kind",
            "Choose one of the listed sensor types.",
        );
    }
    let Ok(store) = Supabase::from_env() else {
        return error(
            503,
            "database_unavailable",
            "Sensor setup is temporarily unavailable.",
        );
    };
    match store
        .rpc(
            "fizz_add_sensor_choice",
            json!({"p_token": token, "p_kind": kind}),
        )
        .await
    {
        Ok(value) if value.get("error").and_then(Value::as_str) == Some("unauthorized") => {
            error(401, "unauthorized", "Sign in to continue.")
        }
        Ok(value) if value.get("choices").and_then(Value::as_array).is_some() => {
            reply(200, value, None)
        }
        _ => error(
            503,
            "database_unavailable",
            "Sensor setup is temporarily unavailable.",
        ),
    }
}
