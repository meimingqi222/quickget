//! 本地 HTTP 通道：浏览器扩展投递任务的 Native Messaging 备选路径。
//!
//! 动机：在部分机器上（Edge 增强安全模式、企业策略、安全软件等）,
//! Native Messaging 宿主进程能被浏览器拉起，但 stdio 协议始终无法完成
//! 握手，扩展侧永远超时，宿主进程还会堆积成僵尸。HTTP 通道只监听
//! 127.0.0.1，请求体仍是同一份 JSON，与 native_host 共用 handle 逻辑。
//!
//! 端口固定 18666；被占用则安静失败，扩展自动回退 Native Messaging。

use serde_json::Value;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

pub const HTTP_PORT: u16 = 18666;
const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 4 * 1024 * 1024;

const CORS_HEADERS: &str = concat!(
    "Access-Control-Allow-Origin: *\r\n",
    "Access-Control-Allow-Methods: POST, GET, OPTIONS\r\n",
    "Access-Control-Allow-Headers: Content-Type\r\n",
    // Chrome Private Network Access 预检要求
    "Access-Control-Allow-Private-Network: true\r\n",
);

/// 在 GUI 启动时拉起；失败不影响其它通道。
pub fn start() {
    std::thread::spawn(|| {
        let listener = match TcpListener::bind(("127.0.0.1", HTTP_PORT)) {
            Ok(l) => l,
            Err(e) => {
                crate::core::log::write(format_args!("HTTP 通道绑定 {HTTP_PORT} 失败: {e}"));
                return;
            }
        };
        crate::core::log::write(format_args!("HTTP 通道已监听 127.0.0.1:{HTTP_PORT}"));
        serve(listener);
    });
}

fn serve(listener: TcpListener) {
    for conn in listener.incoming() {
        let Ok(stream) = conn else { continue };
        std::thread::spawn(move || handle_conn(stream));
    }
}

fn handle_conn(mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let Some((method, path, body)) = read_request(&mut stream) else {
        return;
    };
    let (status, payload) = route(&method, &path, &body);
    let _ = write_response(&mut stream, status, &payload);
}

/// 解析最小合法 HTTP 请求：方法、路径、body。head 超上限或 body 超上限直接断开。
fn read_request(stream: &mut TcpStream) -> Option<(String, String, Vec<u8>)> {
    let mut raw = Vec::with_capacity(2048);
    let mut buf = [0u8; 2048];
    let head_end;
    loop {
        if let Some(pos) = find_subslice(&raw, b"\r\n\r\n") {
            head_end = pos + 4;
            break;
        }
        if raw.len() > MAX_HEAD {
            return None;
        }
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        raw.extend_from_slice(&buf[..n]);
    }
    let head = String::from_utf8_lossy(&raw[..head_end]).to_string();
    let mut lines = head.lines();
    let mut parts = lines.next()?.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut content_length = 0usize;
    for line in lines {
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    if content_length > MAX_BODY {
        return None;
    }
    let mut body = raw[head_end..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        body.extend_from_slice(&buf[..n]);
    }
    body.truncate(content_length);
    Some((method, path, body))
}

/// 路由 + 业务处理。返回 (HTTP 状态码, 响应体)。
fn route(method: &str, path: &str, body: &[u8]) -> (u16, Vec<u8>) {
    if method == "OPTIONS" {
        return (204, Vec::new());
    }
    match (method, path) {
        ("GET", "/ping") => (
            200,
            serde_json::to_vec(&serde_json::json!({"ok": true})).unwrap(),
        ),
        ("POST", "/job") | ("POST", "/ping") => {
            let msg: Value = match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(_) => {
                    return (400, b"{\"ok\":false,\"error\":\"bad json\"}".to_vec());
                }
            };
            (200, serde_json::to_vec(&crate::core::native_host::handle(msg)).unwrap())
        }
        _ => (404, b"{\"ok\":false,\"error\":\"not found\"}".to_vec()),
    }
}

fn write_response(stream: &mut TcpStream, status: u16, body: &[u8]) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Internal Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\n{CORS_HEADERS}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    #[test]
    fn parses_minimal_post() {
        let raw = b"POST /job HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 13\r\n\r\n{\"ping\":true}";
        let (mut client, mut server) = pipe_pair();
        client.write_all(raw).unwrap();
        let (method, path, body) = read_request(&mut server).unwrap();
        assert_eq!(method, "POST");
        assert_eq!(path, "/job");
        assert_eq!(body, b"{\"ping\":true}");
    }

    #[test]
    fn end_to_end_get_ping_over_tcp() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || serve(listener));
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(b"GET /ping HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .unwrap();
        let mut resp = Vec::new();
        s.read_to_end(&mut resp).unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.starts_with("HTTP/1.1 200"), "{text}");
        assert!(text.contains("\"ok\":true"), "{text}");
        assert!(text.contains("Access-Control-Allow-Private-Network"), "{text}");
    }

    /// socketpair 的丐版：用临时 listener 造一对互通的 TcpStream。
    fn pipe_pair() -> (TcpStream, TcpStream) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let c = TcpStream::connect(addr).unwrap();
        let (s, _) = l.accept().unwrap();
        (c, s)
    }
}
