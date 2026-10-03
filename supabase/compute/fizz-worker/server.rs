use std::{env, io::{Read, Write}, net::TcpListener};

fn main() -> std::io::Result<()> {
    let port = env::var("PORT").unwrap_or_else(|_| "8080".to_owned());
    let listener = TcpListener::bind(format!("0.0.0.0:{port}"))?;
    for stream in listener.incoming() {
        let mut stream = stream?;
        let mut request = [0_u8; 1024];
        let bytes = stream.read(&mut request)?;
        let healthy = request[..bytes].starts_with(b"GET /health ");
        let (status, body) = if healthy {
            ("200 OK", r#"{"service":"fizz-worker","status":"ok"}"#)
        } else {
            ("404 Not Found", r#"{"error":"not_found"}"#)
        };
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes())?;
    }
    Ok(())
}
