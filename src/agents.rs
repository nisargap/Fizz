//! Dashboard management of agent connections: list, create (token shown once), and revoke.

use serde_json::{Value, json};
use vercel_runtime::{Request, Response, ResponseBody};

use crate::{
    customer::{error, json_body, reply, session_token},
    sensors::valid_uuid,
    store::Supabase,
};

fn outcome(
    result: Result<Value, crate::store::StoreError>,
    success: u16,
) -> Response<ResponseBody> {
    match result {
        Ok(value) => match value.get("error").and_then(Value::as_str) {
            None => reply(success, value, None),
            Some("unauthorized") => error(401, "unauthorized", "Sign in to continue."),
            Some("invalid_name") => error(
                400,
                "invalid_name",
                "Name the agent in 60 characters or fewer.",
            ),
            Some("too_many") => error(
                409,
                "too_many",
                "You can connect up to 20 agents. Revoke one first.",
            ),
            Some("not_found") => error(404, "not_found", "That agent connection was not found."),
            Some(_) => error(400, "invalid_request", "Check the request and try again."),
        },
        Err(_) => error(
            503,
            "database_unavailable",
            "Agent connections are temporarily unavailable.",
        ),
    }
}

pub async fn handle(request: Request) -> Response<ResponseBody> {
    let Some(token) = session_token(&request) else {
        return error(401, "unauthorized", "Sign in to continue.");
    };
    let method = request.method().as_str().to_owned();
    let Ok(store) = Supabase::from_env() else {
        return error(
            503,
            "database_unavailable",
            "Agent connections are temporarily unavailable.",
        );
    };
    match method.as_str() {
        "GET" => outcome(
            store
                .rpc("fizz_agent_list", json!({"p_token": token}))
                .await,
            200,
        ),
        "POST" => {
            let data = match json_body(request, 1024).await {
                Ok(value) => value,
                Err(response) => return response,
            };
            let name = data
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            if name.is_empty() || name.chars().count() > 60 || name.chars().any(char::is_control) {
                return error(
                    400,
                    "invalid_name",
                    "Name the agent in 60 characters or fewer.",
                );
            }
            let can_create_alerts = data
                .get("can_create_alerts")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            outcome(
                store
                    .rpc(
                        "fizz_agent_create",
                        json!({"p_token": token, "p_name": name, "p_can_create_alerts": can_create_alerts}),
                    )
                    .await,
                201,
            )
        }
        "DELETE" => {
            let data = match json_body(request, 512).await {
                Ok(value) => value,
                Err(response) => return response,
            };
            let id = data.get("id").and_then(Value::as_str).unwrap_or("");
            if !valid_uuid(id) {
                return error(
                    400,
                    "invalid_request",
                    "Choose an agent connection to revoke.",
                );
            }
            outcome(
                store
                    .rpc("fizz_agent_revoke", json!({"p_token": token, "p_id": id}))
                    .await,
                200,
            )
        }
        _ => error(405, "method_not_allowed", "Use GET, POST, or DELETE."),
    }
}
