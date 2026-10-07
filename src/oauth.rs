//! Google sign-in through Supabase Auth, run entirely on the server with PKCE.
//!
//! `GET /api/auth?provider=google[&invite=...]` sends the browser to Supabase, which hands it to
//! Google and back to `GET /api/auth?code=...`. The code is exchanged here for the verified
//! Supabase user, then Fizz issues its normal session cookie. The browser never sees a Supabase
//! key or token, and every other endpoint keeps working with `fizz_session`.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::Url;
use ring::{
    digest,
    rand::{SecureRandom, SystemRandom},
};
use serde_json::{Value, json};
use vercel_runtime::{Request, Response, ResponseBody};

use crate::customer::{client_ip, cookie, error, invite_code, session_cookie};
use crate::store::Supabase;

const FLOW_COOKIE: &str = "fizz_oauth";
const CLEAR_FLOW_COOKIE: &str =
    "fizz_oauth=; HttpOnly; Secure; SameSite=Lax; Path=/api/auth; Max-Age=0";

pub async fn handle(request: Request) -> Response<ResponseBody> {
    let query = Url::parse(&format!(
        "http://fizz.invalid/?{}",
        request.uri().query().unwrap_or("")
    ))
    .map(|url| url.query_pairs().into_owned().collect::<Vec<_>>())
    .unwrap_or_default();
    let find = |key: &str| {
        query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };

    if let Some(code) = find("code") {
        finish(&request, code).await
    } else if let Some(reason) = find("error") {
        // Supabase reports a cancelled Google consent screen as access_denied.
        let code = if reason == "access_denied" {
            "cancelled"
        } else {
            "failed"
        };
        back_to_app(Some(code), &[CLEAR_FLOW_COOKIE])
    } else if find("provider") == Some("google") {
        start(&request, find("invite"))
    } else {
        error(400, "invalid_request", "Use provider=google.")
    }
}

fn start(request: &Request, invite: Option<&str>) -> Response<ResponseBody> {
    // The invite rides along in the flow cookie with only its letters and digits, which the
    // database normalizes the same way as a typed code.
    let invite = match invite.filter(|v| !v.trim().is_empty()) {
        None => String::new(),
        Some(raw) => match invite_code(raw) {
            Some(code) => code.chars().filter(char::is_ascii_alphanumeric).collect(),
            None => return back_to_app(Some("invalid_invite"), &[]),
        },
    };
    let (Ok(store), Some(callback)) = (Supabase::from_env(), callback_url(host(request))) else {
        return back_to_app(Some("failed"), &[]);
    };
    let mut bytes = [0u8; 32];
    if SystemRandom::new().fill(&mut bytes).is_err() {
        return back_to_app(Some("failed"), &[]);
    }
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let challenge = pkce_challenge(&verifier);
    let flow = format!(
        "{FLOW_COOKIE}={verifier}.{invite}; HttpOnly; Secure; SameSite=Lax; Path=/api/auth; Max-Age=600"
    );
    redirect(
        store
            .authorize_url("google", &callback, &challenge)
            .as_str(),
        &[&flow],
    )
}

async fn finish(request: &Request, code: &str) -> Response<ResponseBody> {
    // The verifier lives only in this browser's cookie, so a code from someone else's sign-in
    // (or a forged callback link) cannot be exchanged here.
    let Some((verifier, invite)) = cookie(request, FLOW_COOKIE).and_then(|v| v.split_once('.'))
    else {
        return back_to_app(Some("expired"), &[CLEAR_FLOW_COOKIE]);
    };
    let Ok(store) = Supabase::from_env() else {
        return back_to_app(Some("failed"), &[CLEAR_FLOW_COOKIE]);
    };
    let Ok(user) = store.exchange_auth_code(code, verifier).await else {
        return back_to_app(Some("expired"), &[CLEAR_FLOW_COOKIE]);
    };
    let result = store
        .rpc(
            "fizz_oauth_sign_in",
            json!({
                "p_user_id": user["id"],
                "p_email": user["email"],
                "p_invite": (!invite.is_empty()).then_some(invite),
                "p_ip": client_ip(request),
            }),
        )
        .await;
    let Ok(value) = result else {
        return back_to_app(Some("failed"), &[CLEAR_FLOW_COOKIE]);
    };
    match (
        value.get("token").and_then(Value::as_str),
        value.get("error").and_then(Value::as_str),
    ) {
        (Some(token), _) => back_to_app(None, &[&session_cookie(token), CLEAR_FLOW_COOKIE]),
        (None, Some(reason @ ("no_account" | "invalid_invite" | "rate_limited"))) => {
            back_to_app(Some(reason), &[CLEAR_FLOW_COOKIE])
        }
        _ => back_to_app(Some("failed"), &[CLEAR_FLOW_COOKIE]),
    }
}

/// The S256 code challenge from RFC 7636.
fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(digest::digest(&digest::SHA256, verifier.as_bytes()))
}

/// Supabase only redirects to URLs on its allow list, so the callback must match one of the
/// project's Redirect URLs, such as `https://fizzlayer.com/api/auth`.
fn callback_url(host: Option<&str>) -> Option<String> {
    let host = host?;
    if host.is_empty()
        || !host
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b':'))
    {
        return None;
    }
    let local = host.starts_with("localhost") || host.starts_with("127.0.0.1");
    Some(format!(
        "{}://{host}/api/auth",
        if local { "http" } else { "https" }
    ))
}

fn host(request: &Request) -> Option<&str> {
    request.headers().get("host")?.to_str().ok()
}

fn back_to_app(auth_error: Option<&str>, cookies: &[&str]) -> Response<ResponseBody> {
    match auth_error {
        Some(code) => redirect(&format!("/app.html#auth_error={code}"), cookies),
        None => redirect("/app.html", cookies),
    }
}

fn redirect(location: &str, cookies: &[&str]) -> Response<ResponseBody> {
    let mut builder = Response::builder()
        .status(303)
        .header("location", location)
        .header("cache-control", "no-store")
        .header("referrer-policy", "no-referrer");
    for cookie in cookies {
        builder = builder.header("set-cookie", *cookie);
    }
    builder
        .body(ResponseBody::from(()))
        .expect("valid response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc_7636_example() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn callback_url_uses_https_except_on_localhost_and_rejects_odd_hosts() {
        assert_eq!(
            callback_url(Some("fizzlayer.com")).as_deref(),
            Some("https://fizzlayer.com/api/auth")
        );
        assert_eq!(
            callback_url(Some("localhost:3000")).as_deref(),
            Some("http://localhost:3000/api/auth")
        );
        assert_eq!(callback_url(Some("evil.com/path?x=")), None);
        assert_eq!(callback_url(Some("")), None);
        assert_eq!(callback_url(None), None);
    }
}
