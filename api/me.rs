use fizz::customer::{current, error};
use vercel_runtime::{Error, Request, Response, ResponseBody, run, service_fn};

#[tokio::main]
async fn main() -> Result<(), Error> {
    run(service_fn(handler)).await
}

async fn handler(request: Request) -> Result<Response<ResponseBody>, Error> {
    Ok(if request.method() == "GET" {
        current(request).await
    } else {
        error(405, "method_not_allowed", "Use GET.")
    })
}
