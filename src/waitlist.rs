use serde_json::{Value, json};
use vercel_runtime::{Request, Response, ResponseBody};

use crate::{
    customer::{client_ip, error, json_body, reply},
    store::Supabase,
};

/// A plausible address: one @, a dotted domain, no spaces or control characters.
/// Delivery is the real test; this keeps obvious junk out of the list.
pub fn normalize_email(value: &str) -> Option<String> {
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

/// Free text from the business form: trimmed, length-bounded, and free of control
/// characters other than line breaks in the note.
fn clean_text(value: Option<&Value>, max_chars: usize, allow_newlines: bool) -> Option<String> {
    let text = value.and_then(Value::as_str).unwrap_or("").trim();
    let valid = text.chars().count() <= max_chars
        && text
            .chars()
            .all(|c| !c.is_control() || (allow_newlines && (c == '\n' || c == '\r')));
    valid.then(|| text.to_owned())
}

pub async fn join(request: Request) -> Response<ResponseBody> {
    let ip = client_ip(&request);
    let data = match json_body(request, 4096).await {
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
    let business = data.get("kind").and_then(Value::as_str) == Some("business");
    let (rpc, payload) = if business {
        let Some(company) = clean_text(data.get("company"), 120, false).filter(|c| !c.is_empty())
        else {
            return error(400, "invalid_details", "Enter your company name.");
        };
        let Some(interest) = clean_text(data.get("interest"), 500, true) else {
            return error(
                400,
                "invalid_details",
                "Keep the note under 500 characters.",
            );
        };
        (
            "fizz_join_business_interest",
            json!({"p_email": email, "p_company": company, "p_interest": interest, "p_ip": ip}),
        )
    } else {
        ("fizz_join_waitlist", json!({"p_email": email, "p_ip": ip}))
    };
    let Ok(store) = Supabase::from_env() else {
        return error(
            503,
            "database_unavailable",
            "The waitlist is temporarily unavailable.",
        );
    };
    match store.rpc(rpc, payload).await {
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
        Ok(value) if value.get("error").and_then(Value::as_str) == Some("invalid_details") => {
            error(
                400,
                "invalid_details",
                "Enter your company name and a note under 500 characters.",
            )
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

    #[test]
    fn business_text_is_trimmed_bounded_and_free_of_control_characters() {
        let text = |v: Value, max, newlines| clean_text(Some(&v), max, newlines);
        assert_eq!(
            text(json!("  Acme Logistics "), 120, false),
            Some("Acme Logistics".into())
        );
        assert_eq!(clean_text(None, 500, true), Some(String::new()));
        assert_eq!(
            text(json!("Cold rooms\nand docks"), 500, true),
            Some("Cold rooms\nand docks".into())
        );
        assert_eq!(text(json!("Acme\nCorp"), 120, false), None);
        assert_eq!(text(json!("bell\u{7}"), 500, true), None);
        assert_eq!(
            text(json!("é".repeat(120)), 120, false),
            Some("é".repeat(120))
        );
        assert_eq!(text(json!("x".repeat(501)), 500, true), None);
    }
}
