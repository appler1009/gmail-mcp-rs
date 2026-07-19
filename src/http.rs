// Minimal synchronous HTTPS/1.1 client: TcpStream + native-tls, hand-rolled request/response
// framing. No reqwest/hyper/tokio - kept intentionally small since Gmail calls here are
// low-volume request/response, not a server needing keep-alive or pipelining.

use native_tls::TlsConnector;
use std::io::{Read, Write};
use std::net::TcpStream;

pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Response {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn json(&self) -> Result<serde_json::Value, String> {
        serde_json::from_slice(&self.body).map_err(|e| format!("invalid JSON response: {e}"))
    }
}

pub fn request(
    method: &str,
    host: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
) -> Result<Response, String> {
    let addr = format!("{host}:443");
    let tcp = TcpStream::connect(&addr).map_err(|e| format!("connect to {addr} failed: {e}"))?;
    tcp.set_nodelay(true).ok();

    let connector = TlsConnector::new().map_err(|e| format!("tls connector init failed: {e}"))?;
    let mut stream = connector
        .connect(host, tcp)
        .map_err(|e| format!("tls handshake with {host} failed: {e}"))?;

    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nUser-Agent: gmail-mcp/0.1\r\n");
    for (k, v) in headers {
        req.push_str(k);
        req.push_str(": ");
        req.push_str(v);
        req.push_str("\r\n");
    }
    if let Some(b) = body {
        req.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");

    stream
        .write_all(req.as_bytes())
        .map_err(|e| format!("write request failed: {e}"))?;
    if let Some(b) = body {
        stream
            .write_all(b)
            .map_err(|e| format!("write body failed: {e}"))?;
    }

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| format!("read response failed: {e}"))?;

    parse_response(&raw)
}

fn parse_response(raw: &[u8]) -> Result<Response, String> {
    let header_end = find_subslice(raw, b"\r\n\r\n").ok_or("malformed HTTP response: no header terminator")?;
    let header_text = String::from_utf8_lossy(&raw[..header_end]);
    let mut lines = header_text.split("\r\n");

    let status_line = lines.next().ok_or("malformed HTTP response: missing status line")?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or("malformed HTTP response: bad status line")?;

    let mut chunked = false;
    let mut content_length: Option<usize> = None;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim();
            if k == "transfer-encoding" && v.eq_ignore_ascii_case("chunked") {
                chunked = true;
            } else if k == "content-length" {
                content_length = v.parse().ok();
            }
        }
    }

    let body_bytes = &raw[header_end + 4..];
    let body = if chunked {
        decode_chunked(body_bytes)?
    } else if let Some(len) = content_length {
        body_bytes.get(..len).unwrap_or(body_bytes).to_vec()
    } else {
        body_bytes.to_vec()
    };

    Ok(Response { status, body })
}

fn decode_chunked(mut data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let line_end = find_subslice(data, b"\r\n").ok_or("malformed chunked body: no size line")?;
        let size_str = String::from_utf8_lossy(&data[..line_end]);
        let size = usize::from_str_radix(size_str.trim(), 16)
            .map_err(|_| "malformed chunked body: bad chunk size")?;
        data = &data[line_end + 2..];
        if size == 0 {
            break;
        }
        out.extend_from_slice(&data[..size.min(data.len())]);
        data = &data[(size + 2).min(data.len())..];
    }
    Ok(out)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Decodes RFC 4648 base64url (the alphabet the Gmail API uses for message bodies and
/// attachments), tolerating missing padding since Google omits it.
pub fn base64url_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut val: u32 = 0;
    let mut bits = 0;
    let mut out = Vec::with_capacity(s.len() * 3 / 4);

    for c in s.chars() {
        let digit = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '-' => 62,
            '_' => 63,
            '=' => continue,
            '\n' | '\r' => continue,
            other => return Err(format!("invalid base64url character: {other}")),
        };
        val = (val << 6) | digit;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((val >> bits) as u8);
        }
    }

    Ok(out)
}

/// Percent-encode per RFC 3986 unreserved set, for query params and form bodies.
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
