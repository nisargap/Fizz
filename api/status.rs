use fizz::store::{ConnectionError, Supabase};
use serde_json::{Value, json};
use vercel_runtime::{Error, Request, run, service_fn};

#[tokio::main]
async fn main() -> Result<(), Error> {
    run(service_fn(handler)).await
}

async fn handler(_request: Request) -> Result<Value, Error> {
    let database = match Supabase::from_env() {
        Ok(supabase) => match supabase.check_connection().await {
            Ok(()) => "connected",
            Err(_) => "unavailable",
        },
        Err(ConnectionError::MissingConfiguration) => "unconfigured",
        Err(_) => "unavailable",
    };
    Ok(json!({
        "service": "fizz",
        "status": if database == "connected" { "ok" } else { "degraded" },
        "database": database,
        "description": "The physical layer for AI agents"
    }))
}
