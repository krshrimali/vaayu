use std::io::{self, BufRead, Read, Write};

use serde_json::Value;

/// Reads one `Content-Length`-framed LSP message from `r`. Returns `Ok(None)`
/// on a clean EOF (server exited), `Err` on a malformed stream.
pub fn read_message<R: BufRead>(r: &mut R) -> io::Result<Option<Value>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = r.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(v) = line.strip_prefix("Content-Length:") {
            content_length = v.trim().parse().ok();
        }
        // Other headers (Content-Type) are accepted and ignored.
    }
    let len = content_length.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length"))?;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    let value: Value = serde_json::from_slice(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(Some(value))
}

pub fn write_message<W: Write>(w: &mut W, value: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(value)?;
    write!(w, "Content-Length: {}\r\n\r\n", body.len())?;
    w.write_all(&body)?;
    w.flush()
}
