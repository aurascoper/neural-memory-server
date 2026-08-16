use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use neural_memory_personal::runtime::load_bearer_token;
use neural_memory_personal::transport::{
    handle, require_loopback, Response, TransportConfig, MAX_BODY_BYTES,
};
use serde_json::json;

const MAX_HEADER_BYTES: usize = 16 * 1024;

struct Settings {
    listen: SocketAddr,
    token_file: PathBuf,
    config: TransportConfig,
}

fn parse_args() -> Result<Settings, String> {
    let mut listen: SocketAddr = "127.0.0.1:9443".parse().expect("literal address");
    let mut database = None;
    let mut signing_key = None;
    let mut token_file = None;
    let mut peer_key = None;
    let mut source_device = None;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut index = 0;
    while index < args.len() {
        let value = args.get(index + 1).ok_or("every flag requires one value")?;
        match args[index].as_str() {
            "--listen" => listen = value.parse().map_err(|_| "invalid listen address")?,
            "--db" => database = Some(PathBuf::from(value)),
            "--key" => signing_key = Some(PathBuf::from(value)),
            "--token-file" => token_file = Some(PathBuf::from(value)),
            "--peer-key" => peer_key = Some(PathBuf::from(value)),
            "--device" => source_device = Some(value.clone()),
            _ => return Err("unknown argument".into()),
        }
        index += 2;
    }
    let listen = require_loopback(listen)?;
    let token_file = token_file.ok_or("--token-file is required")?;
    if !token_file.starts_with("/srv/neural-memory-data/keys/") {
        return Err("token file must be inside /srv/neural-memory-data/keys".into());
    }
    Ok(Settings {
        listen,
        token_file,
        config: TransportConfig {
            database: database.ok_or("--db is required")?,
            signing_key: signing_key.ok_or("--key is required")?,
            peer_key: peer_key.ok_or("--peer-key is required")?,
            source_device: source_device.ok_or("--device is required")?,
        },
    })
}

fn read_request(
    stream: &mut TcpStream,
) -> Result<(String, String, Option<String>, Vec<u8>), Response> {
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|_| error(400, "invalidRequest", "cannot configure request timeout"))?;
    let mut received = Vec::with_capacity(4096);
    let header_end = loop {
        if let Some(position) = received.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        if received.len() >= MAX_HEADER_BYTES {
            return Err(error(
                431,
                "headersTooLarge",
                "request headers exceed 16 KiB",
            ));
        }
        let mut chunk = [0_u8; 4096];
        let count = stream
            .read(&mut chunk)
            .map_err(|_| error(400, "invalidRequest", "cannot read request"))?;
        if count == 0 {
            return Err(error(400, "invalidRequest", "incomplete request headers"));
        }
        received.extend_from_slice(&chunk[..count]);
    };
    let headers = std::str::from_utf8(&received[..header_end])
        .map_err(|_| error(400, "invalidRequest", "headers must be UTF-8"))?;
    let mut lines = headers[..headers.len() - 4].split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split(' ');
    let method = request_line.next().unwrap_or_default().to_string();
    let path = request_line.next().unwrap_or_default().to_string();
    let version = request_line.next().unwrap_or_default();
    if request_line.next().is_some() || version != "HTTP/1.1" {
        return Err(error(
            400,
            "invalidRequest",
            "HTTP/1.1 request line required",
        ));
    }
    let mut content_length = None;
    let mut authorization = None;
    let mut content_type = None;
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| error(400, "invalidRequest", "malformed header"))?;
        let value = value.trim();
        match name.to_ascii_lowercase().as_str() {
            "content-length" if content_length.is_none() => {
                content_length = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| error(400, "invalidRequest", "invalid content length"))?,
                );
            }
            "authorization" if authorization.is_none() => authorization = Some(value.to_string()),
            "content-type" if content_type.is_none() => content_type = Some(value.to_string()),
            "transfer-encoding" => {
                return Err(error(
                    400,
                    "invalidRequest",
                    "transfer encoding is unsupported",
                ));
            }
            "content-length" | "authorization" | "content-type" => {
                return Err(error(
                    400,
                    "invalidRequest",
                    "duplicate security-sensitive header",
                ));
            }
            _ => {}
        }
    }
    if content_type.as_deref() != Some("application/json") {
        return Err(error(
            415,
            "unsupportedMediaType",
            "application/json is required",
        ));
    }
    let content_length =
        content_length.ok_or_else(|| error(411, "lengthRequired", "content length is required"))?;
    if content_length > MAX_BODY_BYTES {
        return Err(error(413, "bodyTooLarge", "request exceeds 8 MiB"));
    }
    let already_read = received.len() - header_end;
    if already_read > content_length {
        return Err(error(
            400,
            "invalidRequest",
            "request body exceeds content length",
        ));
    }
    let mut body = received.split_off(header_end);
    body.resize(content_length, 0);
    stream
        .read_exact(&mut body[already_read..])
        .map_err(|_| error(400, "invalidRequest", "incomplete request body"))?;
    Ok((method, path, authorization, body))
}

fn error(status: u16, code: &str, message: &str) -> Response {
    Response {
        status,
        body: json!({"error":{"code":code,"message":message}}),
    }
}

fn write_response(stream: &mut TcpStream, response: &Response) -> std::io::Result<()> {
    let body = serde_json::to_vec(&response.body).expect("JSON response serializes");
    let reason = match response.status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        431 => "Request Header Fields Too Large",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        response.status,
        reason,
        body.len()
    )?;
    stream.write_all(&body)
}

fn serve(settings: &Settings, token: &[u8], mut stream: TcpStream) {
    let response = match read_request(&mut stream) {
        Ok((method, path, authorization, body)) => handle(
            &settings.config,
            &method,
            &path,
            authorization.as_deref(),
            token,
            &body,
        ),
        Err(response) => response,
    };
    if let Err(error) = write_response(&mut stream, &response) {
        eprintln!("response write failed: {error}");
    }
}

fn run() -> Result<(), String> {
    let settings = parse_args()?;
    let token = load_bearer_token(Path::new(&settings.token_file))?;
    let listener = TcpListener::bind(settings.listen).map_err(|error| error.to_string())?;
    eprintln!("personal sync transport listening on {}", settings.listen);
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => serve(&settings, &token, stream),
            Err(error) => eprintln!("connection failed: {error}"),
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("personal sync transport refused to start: {error}");
            ExitCode::from(2)
        }
    }
}
