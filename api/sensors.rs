use fizz::{customer::error, sensors};
use vercel_runtime::{Error, Request, Response, ResponseBody, run, service_fn};

#[tokio::main]
async fn main() -> Result<(), Error> {
    run(service_fn(handler)).await
}

async fn handler(request: Request) -> Result<Response<ResponseBody>, Error> {
    Ok(match request.method().as_str() {
        "GET" => sensors::list(request).await,
        "POST" => sensors::create(request).await,
        "PATCH" => sensors::update(request).await,
        "DELETE" => sensors::delete(request).await,
        _ => error(
            405,
            "method_not_allowed",
            "Use GET, POST, PATCH, or DELETE.",
        ),
    })
}
