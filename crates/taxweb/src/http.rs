//! Just enough HTTP/1.1 to serve one page and a few JSON routes on loopback.
//! Hand-rolled so the dashboard adds no dependencies to the workspace.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;

/// Uploads are invoices, receipts and bank statements — kilobytes to a few
/// megabytes. The cap exists so a runaway client cannot make the dashboard
/// allocate without bound, not because anything legitimate approaches it.
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

pub struct Request {
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
    /// The client declared more than [`MAX_BODY_BYTES`]. The body was not read;
    /// the route reports this rather than acting on a truncated file.
    pub body_too_large: bool,
}

pub fn read_request(stream: &TcpStream) -> io::Result<Option<Request>> {
    let mut reader = BufReader::new(stream.try_clone()?);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(None);
    }
    let mut parts = request_line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Ok(None);
    };

    let mut content_length = 0usize;
    let mut content_type = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            } else if name.eq_ignore_ascii_case("content-type") {
                content_type = Some(value.trim().to_string());
            }
        }
    }

    let body_too_large = content_length > MAX_BODY_BYTES;
    let mut body = Vec::new();
    if body_too_large {
        // Drain without buffering so the connection stays in sync and the
        // route can still answer with a status instead of a dropped socket.
        io::copy(&mut reader.take(content_length as u64), &mut io::sink())?;
    } else if content_length > 0 {
        body.reserve_exact(content_length);
        reader.take(content_length as u64).read_to_end(&mut body)?;
    }

    let (path, query_string) = match target.split_once('?') {
        Some((p, q)) => (p, q),
        None => (target, ""),
    };
    let query = query_string
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (percent_decode(k), percent_decode(v)))
        .collect();

    Ok(Some(Request {
        method: method.to_string(),
        path: path.to_string(),
        query,
        content_type,
        body,
        body_too_large,
    }))
}

/// Percent-decoding, enough for query values. `+` is a space here because
/// that is what form encoding does and what every client will send for a
/// filename with a space in it.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(&input[i + 1..i + 3], 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    // Not a valid escape — a literal '%', which is legal in a
                    // filename. Keep it rather than dropping a character.
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::percent_decode;

    #[test]
    fn decodes_what_a_filename_actually_contains() {
        assert_eq!(percent_decode("INV-8842.pdf"), "INV-8842.pdf");
        assert_eq!(percent_decode("March%20receipt.pdf"), "March receipt.pdf");
        assert_eq!(percent_decode("March+receipt.pdf"), "March receipt.pdf");
        assert_eq!(percent_decode("%E5%8F%91%E7%A5%A8.pdf"), "发票.pdf");
        // A stray percent is kept, not swallowed.
        assert_eq!(percent_decode("100%.pdf"), "100%.pdf");
        assert_eq!(percent_decode("%zz"), "%zz");
    }
}

pub fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}
