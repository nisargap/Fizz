//! The Settings page: account details and passkeys.
//!
//! `GET /api/account` returns the signed-in customer's email, sign-in methods, and passkeys.
//! `DELETE /api/account` with `{"passkey_id": ...}` removes a passkey. Both use Supabase Auth's
//! admin API, so they work for the whole session. Adding a passkey goes through `/api/auth`,
//! because Supabase only registers passkeys for a recently signed-in user.

use reqwest::Method;
use serde_json::{Value, json};
use vercel_runtime::{Request, Response, ResponseBody};

use crate::customer::{SUPABASE_COOKIE, cookie, error, json_body, reply, session_token};
use crate::store::Supabase;

pub async fn handle(request: Request) -> Response<ResponseBody> {
    let Some(token) = session_token(&request) else {
        return error(401, "unauthorized", "Sign in to continue.");
    };
    let Ok(store) = Supabase::from_env() else {
        return unavailable();
    };
    let account = match store
        .rpc("fizz_session_account", json!({"p_token": token}))
        .await
    {
        Ok(value) if value.get("auth_user_id").is_some_and(Value::is_string) => value,
        Ok(_) => return error(401, "unauthorized", "Sign in to continue."),
        Err(_) => return unavailable(),
    };
    let user_id = account["auth_user_id"].as_str().unwrap_or_default();
    match request.method().as_str() {
        "GET" => {
            // The cookie only exists for an hour after signing in, which is when Supabase will
            // register a new passkey. The page uses this to explain before trying.
            let can_add = cookie(&request, SUPABASE_COOKIE).is_some();
            details(&store, user_id, &account, can_add).await
        }
        "DELETE" => {
            let data = match json_body(request, 512).await {
                Ok(value) => value,
                Err(response) => return response,
            };
            let id = data.get("passkey_id").and_then(Value::as_str).unwrap_or("");
            if !valid_uuid(id) {
                return error(400, "invalid_request", "Choose a passkey to remove.");
            }
            match store
                .auth_admin(
                    Method::DELETE,
                    &format!("admin/users/{user_id}/passkeys/{id}"),
                )
                .await
            {
                Ok((status, _)) if status.is_success() => reply(200, json!({"ok": true}), None),
                Ok((status, _)) if status.as_u16() == 404 => {
                    error(404, "not_found", "That passkey was already removed.")
                }
                _ => unavailable(),
            }
        }
        _ => error(405, "method_not_allowed", "Use GET or DELETE."),
    }
}

async fn details(
    store: &Supabase,
    user_id: &str,
    account: &Value,
    can_add: bool,
) -> Response<ResponseBody> {
    let user_path = format!("admin/users/{user_id}");
    let passkeys_path = format!("admin/users/{user_id}/passkeys");
    let (user, passkeys) = tokio::join!(
        store.auth_admin(Method::GET, &user_path),
        store.auth_admin(Method::GET, &passkeys_path),
    );
    let (Ok((user_status, user)), Ok((passkeys_status, passkeys))) = (user, passkeys) else {
        return unavailable();
    };
    if !user_status.is_success() || !passkeys_status.is_success() {
        return unavailable();
    }
    reply(
        200,
        json!({
            "username": account["username"],
            "email": user["email"],
            "providers": user["app_metadata"]["providers"],
            "created_at": user["created_at"],
            "passkeys": passkey_list(&passkeys),
            "can_add_passkey": can_add,
        }),
        None,
    )
}

/// Supabase returns a plain list; accept `{"passkeys": [...]}` too.
fn passkey_list(body: &Value) -> Vec<Value> {
    body.as_array()
        .or_else(|| body.get("passkeys").and_then(Value::as_array))
        .map(|list| {
            list.iter()
                .filter(|p| p.get("id").is_some_and(Value::is_string))
                .map(|p| {
                    json!({
                        "id": p["id"],
                        "name": p["friendly_name"],
                        "created_at": p["created_at"],
                        "last_used_at": p["last_used_at"],
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn valid_uuid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

fn unavailable() -> Response<ResponseBody> {
    error(
        503,
        "account_unavailable",
        "Account settings are temporarily unavailable.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passkey_list_accepts_both_shapes_and_skips_entries_without_ids() {
        let item = json!({"id": "a", "friendly_name": "Chrome on Mac", "created_at": "2026-10-07"});
        assert_eq!(passkey_list(&json!([item, {"x": 1}])).len(), 1);
        assert_eq!(
            passkey_list(&json!({"passkeys": [item]}))[0]["name"],
            "Chrome on Mac"
        );
        assert!(passkey_list(&Value::Null).is_empty());
    }

    #[test]
    fn passkey_ids_must_be_uuids() {
        assert!(valid_uuid("0f8fad5b-d9cb-469f-a165-70867728950e"));
        assert!(!valid_uuid("0f8fad5b-d9cb-469f-a165-70867728950"));
        assert!(!valid_uuid("../../admin/users"));
    }
}
