use std::net::IpAddr;

use http_body_util::{BodyExt, Limited};
use reqwest::Method;
use serde_json::{Value, json};
use vercel_runtime::{Request, Response, ResponseBody};

use crate::store::Supabase;

const COOKIE_NAME: &str = "fizz_session";
/// A signed-in person's Supabase access token, kept for up to an hour so they can add a passkey.
pub const SUPABASE_COOKIE: &str = "fizz_supabase";
pub const CLEAR_SUPABASE_COOKIE: &str =
    "fizz_supabase=; HttpOnly; Secure; SameSite=Lax; Path=/api; Max-Age=0";

pub fn reply(status: u16, body: Value, cookie: Option<&str>) -> Response<ResponseBody> {
    reply_with(status, body, cookie.as_slice())
}

pub fn reply_with(status: u16, body: Value, cookies: &[&str]) -> Response<ResponseBody> {
    let mut builder = Response::builder()
        .status(status)
        .header("content-type", "application/json; charset=utf-8")
        .header("cache-control", "no-store")
        .header("x-content-type-options", "nosniff");
    for cookie in cookies {
        builder = builder.header("set-cookie", *cookie);
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

pub async fn json_body(request: Request, limit: usize) -> Result<Value, Response<ResponseBody>> {
    let is_json = request
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    if !is_json {
        return Err(error(415, "unsupported_media_type", "Send JSON."));
    }
    let bytes = Limited::new(request.into_body(), limit)
        .collect()
        .await
        .map_err(|_| error(413, "invalid_request", "Request is too large."))?
        .to_bytes();
    serde_json::from_slice(&bytes).map_err(|_| error(400, "invalid_request", "Send a JSON object."))
}

/// The caller's IP for rate limiting. Vercel sets `x-real-ip` and overwrites
/// `x-forwarded-for`, so clients cannot choose these values in production.
pub fn client_ip(request: &Request) -> Option<String> {
    ip_from_headers(|name| request.headers().get(name).and_then(|v| v.to_str().ok()))
}

fn ip_from_headers<'a>(header: impl Fn(&str) -> Option<&'a str>) -> Option<String> {
    header("x-real-ip")
        .or_else(|| header("x-forwarded-for").and_then(|v| v.split(',').next()))
        .and_then(|v| v.trim().parse::<IpAddr>().ok())
        .map(|ip| ip.to_string())
}

/// Invite codes look like FIZZ-ABCD-EFGH-JKMN. The database normalizes and checks them;
/// this only rejects input that cannot possibly be a code.
pub fn invite_code(invite: &str) -> Option<String> {
    let invite = invite.trim();
    let symbols = invite.bytes().filter(u8::is_ascii_alphanumeric).count();
    ((12..=16).contains(&symbols)
        && invite.len() <= 32
        && invite
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b' '))
    .then(|| invite.to_owned())
}

pub fn cookie<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request
        .headers()
        .get("cookie")?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

pub fn session_token(request: &Request) -> Option<String> {
    cookie(request, COOKIE_NAME)
        .filter(|value| value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_owned)
}

pub fn session_cookie(token: &str) -> String {
    format!("{COOKIE_NAME}={token}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=43200")
}

pub fn clear_cookie() -> &'static str {
    "fizz_session=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0"
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
    let Ok(store) = Supabase::from_env() else {
        return error(
            503,
            "database_unavailable",
            "Sign-out is temporarily unavailable.",
        );
    };
    if let Some(token) = session_token(&request)
        && store
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
    // Also end the Supabase session kept for adding passkeys. It expires within the hour anyway,
    // so a failure here does not block signing out.
    if let Some(token) = cookie(&request, SUPABASE_COOKIE) {
        let _ = store
            .auth(
                Method::POST,
                "logout",
                &[("scope", "local")],
                Some(token),
                None,
                None,
            )
            .await;
    }
    reply_with(
        200,
        json!({"ok": true}),
        &[clear_cookie(), CLEAR_SUPABASE_COOKIE],
    )
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
    update_sensor(request, false).await
}

pub async fn remove_sensor(request: Request) -> Response<ResponseBody> {
    update_sensor(request, true).await
}

async fn update_sensor(request: Request, removing: bool) -> Response<ResponseBody> {
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
            if removing {
                "fizz_remove_sensor_choice"
            } else {
                "fizz_add_sensor_choice"
            },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_codes_accept_typed_variants_and_reject_junk() {
        assert_eq!(
            invite_code(" FIZZ-ABCD-EFGH-JKMN "),
            Some("FIZZ-ABCD-EFGH-JKMN".into())
        );
        assert!(invite_code("abcd efgh jkmn").is_some());
        assert!(invite_code("FIZZ-ABCD").is_none());
        assert!(invite_code("FIZZ-ABCD-EFGH-JKMN-PQRS").is_none());
        assert!(invite_code("FIZZ_ABCD_EFGH_JKMN").is_none());
        assert!(invite_code("").is_none());
    }

    #[test]
    fn client_ip_prefers_real_ip_and_ignores_non_addresses() {
        let ip = |headers: &[(&'static str, &'static str)]| {
            let headers = headers.to_vec();
            ip_from_headers(move |name| headers.iter().find(|(n, _)| *n == name).map(|(_, v)| *v))
        };
        assert_eq!(
            ip(&[
                ("x-real-ip", "203.0.113.9"),
                ("x-forwarded-for", "198.51.100.1")
            ]),
            Some("203.0.113.9".into())
        );
        assert_eq!(
            ip(&[("x-forwarded-for", "2001:db8::1, 10.0.0.1")]),
            Some("2001:db8::1".into())
        );
        assert_eq!(ip(&[("x-real-ip", "not-an-ip")]), None);
        assert_eq!(ip(&[]), None);
    }
}
