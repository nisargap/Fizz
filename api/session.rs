use fizz::customer::{error, sign_in, sign_out};
use vercel_runtime::{Error, Request, Response, ResponseBody, run, service_fn};

#[tokio::main]
async fn main() -> Result<(), Error> {
    run(service_fn(handler)).await
}

async fn handler(request: Request) -> Result<Response<ResponseBody>, Error> {
    Ok(match request.method().as_str() {
        "POST" => sign_in(request).await,
        "DELETE" => sign_out(request).await,
        _ => error(405, "method_not_allowed", "Use POST or DELETE."),
    })
}
