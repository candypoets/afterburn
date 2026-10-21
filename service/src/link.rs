//! Join-link handling.
//!
//! The FigCraft CLI publishes against the loopback server and returns a link whose `host` is
//! `127.0.0.1:47910`. The service stores the same file and token but rewrites the host to the
//! address a client can reach (the tailnet address by default), exactly like `service.py`.

use crate::error::{Error, Result};

/// Rewrite (or add) the `host` query parameter of a `figcraft://join?...` link.
///
/// Everything else in the link is preserved; the value is decoded before being re-encoded so an
/// already-escaped host is not double-escaped.
pub fn rewrite_host(link: &str, host: &str) -> Result<String> {
    if link.trim().is_empty() {
        return Err(Error::new("empty join link"));
    }
    if host.trim().is_empty() {
        return Err(Error::new("empty public server address"));
    }

    let (body, fragment) = match link.split_once('#') {
        Some((body, fragment)) => (body, Some(fragment)),
        None => (link, None),
    };
    let (base, query) = match body.split_once('?') {
        Some((base, query)) => (base, query),
        None => (body, ""),
    };
    if base.trim().is_empty() {
        return Err(Error::new("join link has no scheme"));
    }

    let mut replaced = false;
    let mut parameters: Vec<(String, Option<String>)> = Vec::new();
    for part in query.split('&').filter(|part| !part.is_empty()) {
        let (name, value) = match part.split_once('=') {
            Some((name, value)) => (decode(name), Some(decode(value))),
            None => (decode(part), None),
        };
        if name == "host" {
            // `parse_qs` in the reference collapsed duplicate hosts; keep the first.
            if !replaced {
                parameters.push((name, Some(host.to_string())));
                replaced = true;
            }
        } else {
            parameters.push((name, value));
        }
    }
    if !replaced {
        parameters.push(("host".to_string(), Some(host.to_string())));
    }

    let query = parameters
        .iter()
        .map(|(name, value)| match value {
            Some(value) => format!("{}={}", encode(name), encode(value)),
            None => encode(name),
        })
        .collect::<Vec<_>>()
        .join("&");

    let mut result = base.to_string();
    if !query.is_empty() {
        result.push('?');
        result.push_str(&query);
    }
    if let Some(fragment) = fragment {
        result.push('#');
        result.push_str(fragment);
    }
    Ok(result)
}

/// Return one decoded query parameter from a link, if present.
pub fn query_param(link: &str, name: &str) -> Option<String> {
    let body = link.split_once('#').map_or(link, |(body, _)| body);
    let query = body.split_once('?').map(|(_, query)| query)?;
    for part in query.split('&').filter(|part| !part.is_empty()) {
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        if decode(key) == name {
            return Some(decode(value));
        }
    }
    None
}

/// Percent-encode a query component using the RFC 3986 unreserved set.
pub fn encode(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for byte in input.as_bytes() {
        let unreserved = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~');
        if unreserved {
            output.push(*byte as char);
        } else {
            output.push('%');
            output.push_str(&format!("{byte:02X}"));
        }
    }
    output
}

/// Decode percent escapes and `+` as a space.
fn decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let high = hex_value(bytes[index + 1]);
                let low = hex_value(bytes[index + 2]);
                match (high, low) {
                    (Some(high), Some(low)) => {
                        output.push(high * 16 + low);
                        index += 3;
                    }
                    _ => {
                        output.push(b'%');
                        index += 1;
                    }
                }
            }
            b'+' => {
                output.push(b' ');
                index += 1;
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_loopback_host() {
        let link =
            "figcraft://join?host=127.0.0.1%3A47910&file=3e3bb408a90e35a1&token=test-token_abc-123";
        let rewritten = rewrite_host(link, "100.104.9.15:47910").unwrap_or_default();
        assert_eq!(
            rewritten,
            "figcraft://join?host=100.104.9.15%3A47910&file=3e3bb408a90e35a1&token=test-token_abc-123"
        );
    }

    #[test]
    fn keeps_an_encoded_host_from_double_encoding() {
        let link = "figcraft://join?file=abc&host=100.104.9.15%3A47910&token=t";
        let rewritten = rewrite_host(link, "100.104.9.15:47910").unwrap_or_default();
        assert_eq!(
            rewritten,
            "figcraft://join?file=abc&host=100.104.9.15%3A47910&token=t"
        );
    }

    #[test]
    fn adds_host_and_keeps_fragment() {
        let link = "figcraft://join?file=abc#page-2";
        let rewritten = rewrite_host(link, "host:1").unwrap_or_default();
        assert_eq!(rewritten, "figcraft://join?file=abc&host=host%3A1#page-2");
    }

    #[test]
    fn reads_file_parameter() {
        let link = "figcraft://join?host=h&file=3e3bb408a90e35a1&token=t";
        assert_eq!(
            query_param(link, "file").as_deref(),
            Some("3e3bb408a90e35a1")
        );
        assert_eq!(query_param(link, "missing"), None);
    }

    #[test]
    fn rejects_empty_inputs() {
        assert!(rewrite_host("", "h:1").is_err());
        assert!(rewrite_host("figcraft://join?file=x", "").is_err());
    }
}
