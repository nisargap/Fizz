use serde_json::{Value, json};
use vercel_runtime::{Request, Response, ResponseBody};

use crate::{
    customer::{client_ip, error, json_body, reply},
    store::Supabase,
};

/// A plausible address: one @, a dotted domain, no spaces or control characters.
/// Delivery is the real test; this keeps obvious junk out of the list.
fn normalize_email(value: &str) -> Option<String> {
    let email = value.trim().to_ascii_lowercase();
    let (local, domain) = email.split_once('@')?;
    let valid = email.len() <= 254
        && !local.is_empty()
        && local.len() <= 64
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains('@')
        && email.chars().all(|c| !c.is_whitespace() && !c.is_control());
    valid.then_some(email)
}

pub async fn join(request: Request) -> Response<ResponseBody> {
    let ip = client_ip(&request);
    let data = match json_body(request, 512).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let Some(email) = data
        .get("email")
        .and_then(Value::as_str)
        .and_then(normalize_email)
    else {
        return error(400, "invalid_email", "Enter a valid email address.");
    };
    let Ok(store) = Supabase::from_env() else {
        return error(
            503,
            "database_unavailable",
            "The waitlist is temporarily unavailable.",
        );
    };
    match store
        .rpc("fizz_join_waitlist", json!({"p_email": email, "p_ip": ip}))
        .await
    {
        Ok(value) if value.get("ok").and_then(Value::as_bool) == Some(true) => {
            reply(200, json!({"ok": true}), None)
        }
        Ok(value) if value.get("error").and_then(Value::as_str) == Some("rate_limited") => error(
            429,
            "rate_limited",
            "Too many sign-ups from this network. Try again in an hour.",
        ),
        Ok(value) if value.get("error").and_then(Value::as_str) == Some("invalid_email") => {
            error(400, "invalid_email", "Enter a valid email address.")
        }
        _ => error(
            503,
            "database_unavailable",
            "The waitlist is temporarily unavailable.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emails_are_trimmed_lowercased_and_sanity_checked() {
        assert_eq!(
            normalize_email("  Ann@Example.COM "),
            Some("ann@example.com".into())
        );
        assert_eq!(
            normalize_email("a.b+fizz@mail.co.uk"),
            Some("a.b+fizz@mail.co.uk".into())
        );
        for bad in [
            "",
            "ann",
            "ann@",
            "@example.com",
            "ann@example",
            "ann@.com",
            "ann@example.",
            "a@b@c.com",
            "an n@example.com",
            "ann@exa\u{7}mple.com",
        ] {
            assert_eq!(normalize_email(bad), None, "{bad:?}");
        }
        assert_eq!(
            normalize_email(&format!("{}@example.com", "a".repeat(65))),
            None
        );
    }
}
