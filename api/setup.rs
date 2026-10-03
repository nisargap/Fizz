use fizz::customer::{error, remove_sensor, select_sensor, sensor_choices};
use vercel_runtime::{Error, Request, Response, ResponseBody, run, service_fn};

#[tokio::main]
async fn main() -> Result<(), Error> {
    run(service_fn(handler)).await
}

async fn handler(request: Request) -> Result<Response<ResponseBody>, Error> {
    Ok(match request.method().as_str() {
        "GET" => sensor_choices(request).await,
        "POST" => select_sensor(request).await,
        "DELETE" => remove_sensor(request).await,
        _ => error(405, "method_not_allowed", "Use GET, POST, or DELETE."),
    })
}
