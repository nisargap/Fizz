use serde_json::{Value, json};
use vercel_runtime::{Error, Request, run, service_fn};

#[tokio::main]
async fn main() -> Result<(), Error> {
    run(service_fn(handler)).await
}

async fn handler(_request: Request) -> Result<Value, Error> {
    Ok(json!({
        "service": "fizz",
        "status": "ok",
        "description": "The physical layer for AI agents"
    }))
}
