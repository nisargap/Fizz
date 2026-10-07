//! Sign-in through Supabase Auth: Google, email and password, and passkeys.
//!
//! Supabase verifies who the person is. Fizzlayer then links that Supabase user to a customer and
//! issues its usual `fizz_session` cookie, so every other endpoint is unchanged. All Supabase calls
//! happen here on the server, and the browser never receives a Supabase key.
//!
//! - `GET /api/auth?provider=google[&invite=...]` starts Google sign-in with PKCE, and
//!   `GET /api/auth?code=...` finishes it.
//! - `POST /api/auth` with `{"action": ...}` handles email and password, links from Supabase
//!   emails (which arrive in `/app.html#access_token=...`), and passkeys.
//!
//! After any sign-in, the Supabase access token is kept for up to an hour in an HttpOnly cookie.
//! Supabase only lets a signed-in user register a passkey, and this is how "Add passkey" proves it.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{Method, StatusCode, Url};
use ring::{
    digest,
    rand::{SecureRandom, SystemRandom},
};
use serde_json::{Value, json};
use vercel_runtime::{Request, Response, ResponseBody};

use crate::customer::{
    SUPABASE_COOKIE, client_ip, cookie, error, invite_code, json_body, reply_with, session_cookie,
};
use crate::store::Supabase;
use crate::waitlist::normalize_email;

const FLOW_COOKIE: &str = "fizz_oauth";
const CLEAR_FLOW_COOKIE: &str =
    "fizz_oauth=; HttpOnly; Secure; SameSite=Lax; Path=/api/auth; Max-Age=0";

// ---- Google ------------------------------------------------------------------------------------

pub async fn google(request: Request) -> Response<ResponseBody> {
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
        finish_google(&request, code).await
    } else if let Some(reason) = find("error") {
        // Supabase reports a cancelled Google consent screen as access_denied.
        let code = if reason == "access_denied" {
            "cancelled"
        } else {
            "failed"
        };
        back_to_app(Some(code), &[CLEAR_FLOW_COOKIE])
    } else if find("provider") == Some("google") {
        start_google(&request, find("invite"))
    } else {
        error(400, "invalid_request", "Use provider=google.")
    }
}

fn start_google(request: &Request, invite: Option<&str>) -> Response<ResponseBody> {
    // The invite rides along in the flow cookie with only its letters and digits, which the
    // database normalizes the same way as a typed code.
    let invite = match invite.filter(|v| !v.trim().is_empty()) {
        None => String::new(),
        Some(raw) => match invite_code(raw) {
            Some(code) => code.chars().filter(char::is_ascii_alphanumeric).collect(),
            None => return back_to_app(Some("invalid_invite"), &[]),
        },
    };
    let (Ok(store), Some(origin)) = (Supabase::from_env(), site_origin(host(request))) else {
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
            .authorize_url("google", &format!("{origin}/api/auth"), &challenge)
            .as_str(),
        &[&flow],
    )
}

async fn finish_google(request: &Request, code: &str) -> Response<ResponseBody> {
    // The verifier lives only in this browser's cookie, so a code from someone else's sign-in
    // (or a forged callback link) cannot be exchanged here.
    let Some((verifier, invite)) = cookie(request, FLOW_COOKIE).and_then(|v| v.split_once('.'))
    else {
        return back_to_app(Some("expired"), &[CLEAR_FLOW_COOKIE]);
    };
    let Ok(store) = Supabase::from_env() else {
        return back_to_app(Some("failed"), &[CLEAR_FLOW_COOKIE]);
    };
    let ip = client_ip(request);
    let session = match store
        .auth(
            Method::POST,
            "token",
            &[("grant_type", "pkce")],
            None,
            Some(json!({"auth_code": code, "code_verifier": verifier})),
            ip.as_deref(),
        )
        .await
    {
        Ok((status, session)) if status.is_success() => session,
        _ => return back_to_app(Some("expired"), &[CLEAR_FLOW_COOKIE]),
    };
    match signed_in(&store, ip.as_deref(), &session, Some(invite)).await {
        Ok(account) => {
            let mut cookies: Vec<&str> = account.cookies.iter().map(String::as_str).collect();
            cookies.push(CLEAR_FLOW_COOKIE);
            back_to_app(None, &cookies)
        }
        Err(reason) => back_to_app(Some(reason), &[CLEAR_FLOW_COOKIE]),
    }
}

// ---- Email, password, email links, and passkeys -----------------------------------------------

pub async fn action(request: Request) -> Response<ResponseBody> {
    let ip = client_ip(&request);
    let origin = site_origin(host(&request));
    let supabase_token = cookie(&request, SUPABASE_COOKIE).map(str::to_owned);
    // Passkey responses carry public keys and signatures, so allow a few kilobytes.
    let data = match json_body(request, 16 * 1024).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (Ok(store), Some(origin)) = (Supabase::from_env(), origin) else {
        return unavailable();
    };
    let ctx = Ctx {
        store,
        ip,
        app_url: format!("{origin}/app.html"),
    };
    let text = |key: &str| data.get(key).and_then(Value::as_str).unwrap_or("");
    match text("action") {
        "sign_up" => sign_up(&ctx, text("email"), text("password"), text("invite")).await,
        "sign_in" => sign_in_with_password(&ctx, text("email"), text("password")).await,
        "resend" => send_email(&ctx, "resend", text("email")).await,
        "recover" => send_email(&ctx, "recover", text("email")).await,
        "link" => {
            let password = data.get("password").and_then(Value::as_str);
            open_email_link(&ctx, text("access_token"), password).await
        }
        "passkey_options" => passkey_options(&ctx, None).await,
        "passkey_sign_in" => passkey_sign_in(&ctx, &data).await,
        "passkey_add_options" => match supabase_token.as_deref() {
            Some(token) => passkey_options(&ctx, Some(token)).await,
            None => sign_in_again(),
        },
        "passkey_add" => match supabase_token.as_deref() {
            Some(token) => passkey_add(&ctx, token, &data).await,
            None => sign_in_again(),
        },
        _ => error(400, "invalid_request", "Unknown action."),
    }
}

struct Ctx {
    store: Supabase,
    ip: Option<String>,
    app_url: String,
}

impl Ctx {
    async fn auth(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, &str)],
        bearer: Option<&str>,
        body: Option<Value>,
    ) -> Result<(StatusCode, Value), Response<ResponseBody>> {
        self.store
            .auth(method, path, query, bearer, body, self.ip.as_deref())
            .await
            .map_err(|_| unavailable())
    }
}

async fn sign_up(ctx: &Ctx, email: &str, password: &str, invite: &str) -> Response<ResponseBody> {
    let Some(invite) = invite_code(invite) else {
        return error(
            400,
            "invalid_invite",
            "Enter the invite code from your Fizzlayer invitation.",
        );
    };
    let (email, password) = match credentials(email, password) {
        Ok(value) => value,
        Err(response) => return response,
    };
    // Check the invite before Supabase creates the user. It is consumed later, when the
    // confirmed account is first used, so an unconfirmed sign-up does not waste it.
    match ctx
        .store
        .rpc(
            "fizz_check_invite",
            json!({"p_invite": invite, "p_ip": ctx.ip}),
        )
        .await
    {
        Ok(value) if value.get("ok") == Some(&json!(true)) => {}
        Ok(value) if value.get("error").and_then(Value::as_str) == Some("rate_limited") => {
            return error(
                429,
                "rate_limited",
                "Too many attempts. Try again in an hour.",
            );
        }
        Ok(_) => {
            return error(
                403,
                "invalid_invite",
                "That invite code is not valid or has already been used.",
            );
        }
        Err(_) => return unavailable(),
    }
    let (status, body) = match ctx
        .auth(
            Method::POST,
            "signup",
            &[("redirect_to", &ctx.app_url)],
            None,
            // The invite is stored on the Supabase user and redeemed when the account is first used.
            Some(json!({"email": email, "password": password, "data": {"invite": invite}})),
        )
        .await
    {
        Ok(value) => value,
        Err(response) => return response,
    };
    if !status.is_success() {
        return auth_failure(status, &body);
    }
    // With email confirmation on (the normal case), there is no session until the link is opened.
    // Supabase answers the same way for addresses that already have an account.
    if body.get("access_token").is_none() {
        return reply_with(200, json!({"ok": true, "confirm_email": true}), &[]);
    }
    account_reply(signed_in(&ctx.store, ctx.ip.as_deref(), &body, None).await)
}

async fn sign_in_with_password(ctx: &Ctx, email: &str, password: &str) -> Response<ResponseBody> {
    let (email, password) = match credentials(email, password) {
        Ok(value) => value,
        // Do not say which part was malformed on sign-in.
        Err(_) => {
            return error(
                401,
                "invalid_credentials",
                "Email or password is incorrect.",
            );
        }
    };
    let (status, session) = match ctx
        .auth(
            Method::POST,
            "token",
            &[("grant_type", "password")],
            None,
            Some(json!({"email": email, "password": password})),
        )
        .await
    {
        Ok(value) => value,
        Err(response) => return response,
    };
    if !status.is_success() {
        return auth_failure(status, &session);
    }
    account_reply(signed_in(&ctx.store, ctx.ip.as_deref(), &session, None).await)
}

/// Resend a sign-up confirmation or send a password reset link. The reply is the same whether
/// or not the address has an account.
async fn send_email(ctx: &Ctx, kind: &str, email: &str) -> Response<ResponseBody> {
    let Some(email) = normalize_email(email) else {
        return error(400, "invalid_email", "Enter a valid email address.");
    };
    let body = if kind == "resend" {
        json!({"type": "signup", "email": email})
    } else {
        json!({"email": email})
    };
    match ctx
        .auth(
            Method::POST,
            kind,
            &[("redirect_to", &ctx.app_url)],
            None,
            Some(body),
        )
        .await
    {
        Ok((status, _)) if status.is_success() => reply_with(200, json!({"ok": true}), &[]),
        Ok((status, body)) => auth_failure(status, &body),
        Err(response) => response,
    }
}

/// Finish a link from a Supabase email. Confirmation links sign the person in; password reset
/// links also set the new `password`.
async fn open_email_link(
    ctx: &Ctx,
    access_token: &str,
    password: Option<&str>,
) -> Response<ResponseBody> {
    if !looks_like_jwt(access_token) {
        return link_expired();
    }
    let result = match password {
        Some(password) => {
            if let Err(response) = check_password(password) {
                return response;
            }
            ctx.auth(
                Method::PUT,
                "user",
                &[],
                Some(access_token),
                Some(json!({"password": password})),
            )
            .await
        }
        None => {
            ctx.auth(Method::GET, "user", &[], Some(access_token), None)
                .await
        }
    };
    let (status, user) = match result {
        Ok(value) => value,
        Err(response) => return response,
    };
    if !status.is_success() {
        return match error_code(&user) {
            "bad_jwt" | "session_not_found" | "session_expired" | "user_not_found" => {
                link_expired()
            }
            _ => auth_failure(status, &user),
        };
    }
    let session = json!({"user": user, "access_token": access_token, "expires_in": 3600});
    account_reply(signed_in(&ctx.store, ctx.ip.as_deref(), &session, None).await)
}

/// Start a passkey ceremony. Without a token this is sign-in, where the browser lets the person
/// pick any of their Fizzlayer passkeys. With the signed-in person's token it registers a new one.
async fn passkey_options(ctx: &Ctx, token: Option<&str>) -> Response<ResponseBody> {
    let path = if token.is_some() {
        "passkeys/registration/options"
    } else {
        "passkeys/authentication/options"
    };
    match ctx
        .auth(Method::POST, path, &[], token, Some(json!({})))
        .await
    {
        Ok((status, body)) if status.is_success() => reply_with(
            200,
            json!({"challenge_id": body["challenge_id"], "options": body["options"]}),
            &[],
        ),
        Ok((status, body)) => auth_failure(status, &body),
        Err(response) => response,
    }
}

async fn passkey_sign_in(ctx: &Ctx, data: &Value) -> Response<ResponseBody> {
    let Some(body) = passkey_body(data) else {
        return error(
            400,
            "passkey_failed",
            "That passkey didn't work. Try again.",
        );
    };
    match ctx
        .auth(
            Method::POST,
            "passkeys/authentication/verify",
            &[],
            None,
            Some(body),
        )
        .await
    {
        Ok((status, session)) if status.is_success() => {
            account_reply(signed_in(&ctx.store, ctx.ip.as_deref(), &session, None).await)
        }
        Ok((status, body)) => auth_failure(status, &body),
        Err(response) => response,
    }
}

async fn passkey_add(ctx: &Ctx, token: &str, data: &Value) -> Response<ResponseBody> {
    let Some(body) = passkey_body(data) else {
        return error(
            400,
            "passkey_failed",
            "That passkey didn't work. Try again.",
        );
    };
    match ctx
        .auth(
            Method::POST,
            "passkeys/registration/verify",
            &[],
            Some(token),
            Some(body),
        )
        .await
    {
        Ok((status, passkey)) if status.is_success() => {
            // Label the passkey with the browser and device the page reported, unless Supabase
            // already named it. A failed rename still leaves a working passkey.
            if let (Some(id), None, Some(name)) = (
                passkey["id"].as_str(),
                passkey["friendly_name"].as_str(),
                passkey_name(data),
            ) {
                let _ = ctx
                    .auth(
                        Method::PATCH,
                        &format!("passkeys/{id}"),
                        &[],
                        Some(token),
                        Some(json!({"friendly_name": name})),
                    )
                    .await;
            }
            reply_with(200, json!({"ok": true}), &[])
        }
        Ok((status, body)) => auth_failure(status, &body),
        Err(response) => response,
    }
}

/// A short label such as "Chrome on Mac": printable, at most 60 characters.
fn passkey_name(data: &Value) -> Option<String> {
    let name = data.get("name").and_then(Value::as_str)?.trim();
    (!name.is_empty() && name.chars().count() <= 60 && !name.chars().any(char::is_control))
        .then(|| name.to_owned())
}

/// The browser's WebAuthn response is passed to Supabase unchanged.
fn passkey_body(data: &Value) -> Option<Value> {
    let challenge = data.get("challenge_id").and_then(Value::as_str)?;
    let credential = data.get("credential").filter(|c| c.is_object())?;
    (!challenge.is_empty() && challenge.len() <= 100)
        .then(|| json!({"challenge_id": challenge, "credential": credential}))
}

// ---- Signing in --------------------------------------------------------------------------------

struct Account {
    username: String,
    cookies: Vec<String>,
}

/// Link a verified Supabase session to its customer and return the cookies that sign them in.
/// New customers need an invite, from Google's flow cookie or stored on the Supabase user at
/// email sign-up.
async fn signed_in(
    store: &Supabase,
    ip: Option<&str>,
    session: &Value,
    invite: Option<&str>,
) -> Result<Account, &'static str> {
    let user = &session["user"];
    let Some(user_id) = user["id"].as_str() else {
        return Err("failed");
    };
    let invite = invite
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            user["user_metadata"]["invite"]
                .as_str()
                .and_then(invite_code)
        });
    let value = store
        .rpc(
            "fizz_oauth_sign_in",
            json!({"p_user_id": user_id, "p_email": user["email"], "p_invite": invite, "p_ip": ip}),
        )
        .await
        .map_err(|_| "failed")?;
    match (
        value["username"].as_str(),
        value["token"].as_str(),
        value["error"].as_str(),
    ) {
        (Some(username), Some(token), _) => {
            let mut cookies = vec![session_cookie(token)];
            if let Some(access) = session["access_token"]
                .as_str()
                .filter(|t| looks_like_jwt(t))
            {
                let max_age = session["expires_in"].as_u64().unwrap_or(3600).min(3600);
                cookies.push(format!(
                    "{SUPABASE_COOKIE}={access}; HttpOnly; Secure; SameSite=Lax; Path=/api; Max-Age={max_age}"
                ));
            }
            Ok(Account {
                username: username.to_owned(),
                cookies,
            })
        }
        (_, _, Some("no_account")) => Err("no_account"),
        (_, _, Some("invalid_invite")) => Err("invalid_invite"),
        (_, _, Some("rate_limited")) => Err("rate_limited"),
        _ => Err("failed"),
    }
}

fn account_reply(result: Result<Account, &'static str>) -> Response<ResponseBody> {
    match result {
        Ok(account) => {
            let cookies: Vec<&str> = account.cookies.iter().map(String::as_str).collect();
            reply_with(
                200,
                json!({"ok": true, "username": account.username}),
                &cookies,
            )
        }
        Err("no_account") => error(
            404,
            "no_account",
            "No Fizzlayer account uses this sign-in yet. Create one with your invite code.",
        ),
        Err("invalid_invite") => error(
            403,
            "invalid_invite",
            "That invite code is not valid or has already been used.",
        ),
        Err("rate_limited") => error(
            429,
            "rate_limited",
            "Too many attempts. Try again in an hour.",
        ),
        Err(_) => unavailable(),
    }
}

// ---- Errors and checks -------------------------------------------------------------------------

fn error_code(body: &Value) -> &str {
    body.get("code")
        .and_then(Value::as_str)
        .or_else(|| body.get("error_code").and_then(Value::as_str))
        .unwrap_or("")
}

/// Turn a Supabase Auth error into a message the sign-in screen can show.
fn auth_failure(status: StatusCode, body: &Value) -> Response<ResponseBody> {
    match error_code(body) {
        "invalid_credentials" => error(
            401,
            "invalid_credentials",
            "Email or password is incorrect.",
        ),
        "email_not_confirmed" => error(
            403,
            "email_not_confirmed",
            "Confirm your email first. Check your inbox for the link.",
        ),
        "weak_password" => error(
            422,
            "weak_password",
            "Choose a stronger password. Use at least 8 characters, and avoid ones that have appeared in data breaches.",
        ),
        "same_password" => error(
            422,
            "same_password",
            "Choose a different password from your current one.",
        ),
        "email_address_invalid" | "validation_failed" => {
            error(400, "invalid_email", "Enter a valid email address.")
        }
        "signup_disabled" | "email_provider_disabled" => error(
            503,
            "signup_disabled",
            "Email sign-up is turned off right now.",
        ),
        "passkey_disabled" => error(
            503,
            "passkey_disabled",
            "Passkeys are turned off right now.",
        ),
        "too_many_passkeys" => error(
            409,
            "too_many_passkeys",
            "This account already has the most passkeys allowed.",
        ),
        "webauthn_credential_exists" => error(
            409,
            "passkey_exists",
            "This passkey is already on your account.",
        ),
        "bad_jwt"
        | "session_not_found"
        | "session_expired"
        | "no_authorization"
        | "user_not_found"
        | "insufficient_aal"
        | "reauthentication_needed" => sign_in_again(),
        "user_banned" => error(403, "user_banned", "This account is suspended."),
        code if code.starts_with("webauthn_") => error(
            400,
            "passkey_failed",
            "That passkey didn't work. Try again.",
        ),
        code if code.starts_with("over_") || status == StatusCode::TOO_MANY_REQUESTS => error(
            429,
            "rate_limited",
            "Too many attempts. Wait a few minutes and try again.",
        ),
        _ => unavailable(),
    }
}

fn unavailable() -> Response<ResponseBody> {
    error(
        503,
        "auth_unavailable",
        "Sign-in is temporarily unavailable. Try again.",
    )
}

fn sign_in_again() -> Response<ResponseBody> {
    error(
        401,
        "sign_in_again",
        "For security, sign out and sign in again to continue.",
    )
}

fn link_expired() -> Response<ResponseBody> {
    error(
        401,
        "link_expired",
        "That link has expired or was already used. Request a new one.",
    )
}

fn credentials(email: &str, password: &str) -> Result<(String, String), Response<ResponseBody>> {
    let Some(email) = normalize_email(email) else {
        return Err(error(400, "invalid_email", "Enter a valid email address."));
    };
    check_password(password)?;
    Ok((email, password.to_owned()))
}

/// Supabase stores passwords with bcrypt, which only reads the first 72 bytes.
fn check_password(password: &str) -> Result<(), Response<ResponseBody>> {
    if (8..=72).contains(&password.len()) {
        Ok(())
    } else {
        Err(error(
            400,
            "weak_password",
            "Use a password of 8 to 72 characters.",
        ))
    }
}

/// A Supabase access token: three base64url parts. This only keeps junk out of cookies and
/// requests; Supabase does the real check.
fn looks_like_jwt(token: &str) -> bool {
    token.len() <= 4096
        && token.split('.').count() == 3
        && token
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
}

/// The S256 code challenge from RFC 7636.
fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(digest::digest(&digest::SHA256, verifier.as_bytes()))
}

/// Supabase only redirects to URLs on its allow list, so links built from this origin must match
/// the project's Redirect URLs, such as `https://fizzlayer.com/api/auth`.
fn site_origin(host: Option<&str>) -> Option<String> {
    let host = host?;
    if host.is_empty()
        || !host
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b':'))
    {
        return None;
    }
    let local = host.starts_with("localhost") || host.starts_with("127.0.0.1");
    Some(format!("{}://{host}", if local { "http" } else { "https" }))
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
    fn site_origin_uses_https_except_on_localhost_and_rejects_odd_hosts() {
        assert_eq!(
            site_origin(Some("fizzlayer.com")).as_deref(),
            Some("https://fizzlayer.com")
        );
        assert_eq!(
            site_origin(Some("localhost:3000")).as_deref(),
            Some("http://localhost:3000")
        );
        assert_eq!(site_origin(Some("evil.com/path?x=")), None);
        assert_eq!(site_origin(Some("")), None);
        assert_eq!(site_origin(None), None);
    }

    #[test]
    fn access_tokens_must_look_like_jwts() {
        assert!(looks_like_jwt("eyJhbGciOi.eyJzdWIiOi.c2lnbmF0dXJl-_"));
        assert!(!looks_like_jwt("only.two"));
        assert!(!looks_like_jwt("a.b.c; Path=/"));
        assert!(!looks_like_jwt(&format!("a.b.{}", "c".repeat(5000))));
    }

    #[test]
    fn passwords_are_8_to_72_bytes() {
        assert!(check_password("12345678").is_ok());
        assert!(check_password("1234567").is_err());
        assert!(check_password(&"x".repeat(73)).is_err());
    }

    #[test]
    fn passkey_body_requires_a_challenge_and_credential_object() {
        assert!(passkey_body(&json!({"challenge_id": "abc", "credential": {"id": "x"}})).is_some());
        assert!(passkey_body(&json!({"challenge_id": "abc", "credential": "x"})).is_none());
        assert!(passkey_body(&json!({"credential": {"id": "x"}})).is_none());
    }

    #[test]
    fn passkey_names_are_short_printable_labels() {
        assert_eq!(
            passkey_name(&json!({"name": " Chrome on Mac "})).as_deref(),
            Some("Chrome on Mac")
        );
        assert!(passkey_name(&json!({"name": ""})).is_none());
        assert!(passkey_name(&json!({"name": "a\nb"})).is_none());
        assert!(passkey_name(&json!({"name": "x".repeat(61)})).is_none());
        assert!(passkey_name(&json!({})).is_none());
    }

    #[test]
    fn supabase_error_codes_map_to_sign_in_messages() {
        let code = |body: Value, status: u16| {
            let response = auth_failure(StatusCode::from_u16(status).unwrap(), &body);
            response.status().as_u16()
        };
        assert_eq!(code(json!({"code": "invalid_credentials"}), 400), 401);
        assert_eq!(code(json!({"code": "email_not_confirmed"}), 400), 403);
        assert_eq!(code(json!({"error_code": "weak_password"}), 422), 422);
        assert_eq!(
            code(json!({"code": "webauthn_verification_failed"}), 400),
            400
        );
        assert_eq!(
            code(json!({"code": "over_email_send_rate_limit"}), 429),
            429
        );
        assert_eq!(code(json!({"code": "session_not_found"}), 403), 401);
        assert_eq!(code(json!({}), 500), 503);
    }
}
