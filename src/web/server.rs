//! A deliberately small HTTP/WebSocket server. It binds to loopback only, but
//! that alone doesn't protect a session that can run shell commands: any page
//! the user visits can reach loopback. So every request must name a loopback
//! host (DNS rebinding), match its Origin (cross-site WebSockets), and carry
//! the session's random token.
use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::Context as _;
use crossterm::event::Event;
use tungstenite::handshake::derive_accept_key;
use tungstenite::protocol::{Role, WebSocket, WebSocketConfig};
use tungstenite::{Message, Utf8Bytes};

use super::protocol::{self, ClientMessage, Font};

const INDEX_HTML: &str = include_str!("assets/index.html");
const APP_JS: &str = include_str!("assets/app.js");
const APP_CSS: &str = include_str!("assets/app.css");

const FIRST_PORT: u16 = 7420;
const PORT_ATTEMPTS: u16 = 32;
const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 64 * 1024;
const MAX_MESSAGE: usize = 32 * 1024 * 1024;
/// A tab this far behind (asleep, or on a stalled tunnel) is dropped rather than
/// buffered for; when it reconnects it gets a complete repaint anyway.
const MAX_BACKLOG: usize = 64 * 1024 * 1024;

pub enum Inbound {
    Attached {
        cols: u16,
        rows: u16,
        cell: (u16, u16),
    },
    Detached,
    Input(Event),
    Clipboard(String),
    Open(String),
}

enum Outbound {
    Frame(Vec<u8>),
    Text(String),
}

struct Client {
    id: u64,
    queue: Arc<OutQueue>,
    socket: TcpStream,
}

#[derive(Default)]
struct OutState {
    items: VecDeque<Outbound>,
    closed: bool,
    delivered: bool,
}

#[derive(Default)]
struct OutQueue {
    state: Mutex<OutState>,
    ready: Condvar,
    bytes: AtomicUsize,
}

impl OutQueue {
    fn lock(&self) -> std::sync::MutexGuard<'_, OutState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn push(&self, item: Outbound) {
        self.lock().items.push_back(item);
        self.ready.notify_all();
    }

    fn close(&self) {
        self.lock().closed = true;
        self.ready.notify_all();
    }

    /// Everything queued, once something is; `None` when closed and empty.
    fn take(&self) -> Option<Vec<Outbound>> {
        let mut state = self.lock();
        while state.items.is_empty() {
            if state.closed {
                return None;
            }
            state = self.ready.wait(state).unwrap_or_else(|e| e.into_inner());
        }
        Some(state.items.drain(..).collect())
    }

    fn mark_delivered(&self) {
        self.lock().delivered = true;
        self.ready.notify_all();
    }

    fn wait_delivered(&self, timeout: Duration) {
        let state = self.lock();
        let _ = self
            .ready
            .wait_timeout_while(state, timeout, |state| !state.delivered);
    }
}

/// The attached tab. Only one tab drives a session; a newer one takes over.
pub struct Hub {
    client: Mutex<Option<Client>>,
    next_id: AtomicU64,
}

impl Hub {
    pub fn send_frame(&self, frame: Vec<u8>) {
        let mut slot = self.client.lock().unwrap_or_else(|e| e.into_inner());
        let Some(client) = slot.as_ref() else {
            return;
        };
        let backlog = client.queue.bytes.fetch_add(frame.len(), Ordering::Relaxed);
        if backlog > MAX_BACKLOG {
            if let Some(client) = slot.take() {
                client.queue.close();
                let _ = client.socket.shutdown(Shutdown::Both);
            }
            return;
        }
        client.queue.push(Outbound::Frame(frame));
    }

    pub fn send_text(&self, text: String) {
        let slot = self.client.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(client) = slot.as_ref() {
            client.queue.push(Outbound::Text(text));
        }
    }

    /// Stops accepting output and waits for the tab's queue to reach the socket.
    pub fn drain(&self, timeout: Duration) {
        let client = self.client.lock().unwrap_or_else(|e| e.into_inner()).take();
        let Some(client) = client else {
            return;
        };
        client.queue.close();
        client.queue.wait_delivered(timeout);
        let _ = client.socket.shutdown(Shutdown::Both);
    }

    fn attach(&self, queue: Arc<OutQueue>, socket: TcpStream) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let previous = self
            .client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replace(Client { id, queue, socket });
        if let Some(previous) = previous {
            previous.queue.push(Outbound::Text(protocol::superseded()));
            previous.queue.close();
        }
        id
    }

    /// False if this client was already replaced by a newer tab.
    fn detach(&self, id: u64) -> bool {
        let mut slot = self.client.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_some_and(|c| c.id == id) {
            *slot = None;
            return true;
        }
        false
    }
}

struct Shared {
    hub: Arc<Hub>,
    inbound: Sender<Inbound>,
    token: String,
    cookie: String,
    font: Font,
}

pub fn start(
    port: Option<u16>,
    font: Font,
) -> anyhow::Result<(Arc<Hub>, Receiver<Inbound>, u16, String)> {
    let listener = bind(port)?;
    let port = listener.local_addr()?.port();
    let token = new_token()?;
    let (inbound, receiver) = mpsc::channel();
    let hub = Arc::new(Hub {
        client: Mutex::new(None),
        next_id: AtomicU64::new(1),
    });
    let shared = Arc::new(Shared {
        hub: hub.clone(),
        inbound,
        cookie: format!("vaayu_{port}"),
        token: token.clone(),
        font,
    });
    std::thread::Builder::new()
        .name("web-accept".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let shared = shared.clone();
                let _ = std::thread::Builder::new()
                    .name("web-conn".into())
                    .spawn(move || {
                        let _ = handle_connection(stream, shared);
                    });
            }
        })?;
    Ok((hub, receiver, port, token))
}

fn bind(port: Option<u16>) -> anyhow::Result<TcpListener> {
    if let Some(port) = port {
        return TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .with_context(|| format!("cannot listen on 127.0.0.1:{port}"));
    }
    for port in FIRST_PORT..FIRST_PORT + PORT_ATTEMPTS {
        if let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
            return Ok(listener);
        }
    }
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).context("cannot listen on loopback")
}

fn new_token() -> anyhow::Result<String> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no system randomness: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

struct Request {
    method: String,
    path: String,
    query: String,
    headers: Vec<(String, String)>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn query_token(&self) -> Option<&str> {
        self.query
            .split('&')
            .find_map(|pair| pair.strip_prefix("token="))
    }

    fn cookie(&self, name: &str) -> Option<&str> {
        self.header("cookie")?.split(';').find_map(|pair| {
            let (key, value) = pair.trim().split_once('=')?;
            (key == name).then_some(value)
        })
    }
}

fn read_request(reader: &mut BufReader<TcpStream>) -> io::Result<Option<Request>> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "bad request");
    let mut line = String::new();
    let mut total = 0;
    let mut read_line = |line: &mut String| -> io::Result<usize> {
        line.clear();
        let n = reader
            .by_ref()
            .take((MAX_HEAD - total) as u64 + 1)
            .read_line(line)?;
        total += n;
        if total > MAX_HEAD {
            return Err(invalid());
        }
        Ok(n)
    };
    if read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().ok_or_else(invalid)?.to_string();
    let target = parts.next().ok_or_else(invalid)?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let (path, query) = (path.to_string(), query.to_string());
    let mut headers = Vec::new();
    loop {
        if read_line(&mut line)? == 0 {
            return Err(invalid());
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        let (name, value) = trimmed.split_once(':').ok_or_else(invalid)?;
        headers.push((name.trim().to_string(), value.trim().to_string()));
    }
    Ok(Some(Request {
        method,
        path,
        query,
        headers,
    }))
}

fn tokens_match(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |diff, (x, y)| diff | (x ^ y))
            == 0
}

fn host_is_loopback(host: &str) -> bool {
    let name = match host.find(']') {
        Some(end) if host.starts_with('[') => {
            let (name, port) = host.split_at(end + 1);
            if !port.is_empty() && !port.starts_with(':') {
                return false;
            }
            name
        }
        _ => host.split(':').next().unwrap_or(""),
    };
    matches!(name, "127.0.0.1" | "localhost" | "[::1]")
}

/// A browser names the requesting page in Origin; tools send none at all.
fn origin_allowed(origin: Option<&str>, host: &str) -> bool {
    origin.is_none_or(|origin| origin.strip_prefix("http://") == Some(host))
}

fn respond(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    extra_headers: &str,
    body: &[u8],
) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\
         Referrer-Policy: no-referrer\r\nConnection: close\r\n{extra_headers}\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

fn plain(stream: &mut TcpStream, status: &str, message: &str) -> io::Result<()> {
    respond(
        stream,
        status,
        "text/plain; charset=utf-8",
        "",
        message.as_bytes(),
    )
}

fn handle_connection(stream: TcpStream, shared: Arc<Shared>) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_nodelay(true)?;
    let mut stream_out = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let Some(request) = read_request(&mut reader)? else {
        return Ok(());
    };
    let host = request.header("host").unwrap_or("").to_string();
    if !host_is_loopback(&host) {
        return plain(&mut stream_out, "403 Forbidden", "host not allowed\n");
    }
    if !origin_allowed(request.header("origin"), &host) {
        return plain(&mut stream_out, "403 Forbidden", "origin not allowed\n");
    }
    let by_query = request
        .query_token()
        .is_some_and(|t| tokens_match(t, &shared.token));
    let by_cookie = request
        .cookie(&shared.cookie)
        .is_some_and(|t| tokens_match(t, &shared.token));

    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") if by_query => {
            // The cookie carries the token from here on, so it leaves the
            // address bar and browser history.
            let cookie = format!(
                "Set-Cookie: {}={}; Path=/; HttpOnly; SameSite=Strict\r\nLocation: /\r\n",
                shared.cookie, shared.token
            );
            respond(&mut stream_out, "302 Found", "text/plain", &cookie, b"")
        }
        ("GET", "/") if by_cookie => {
            let csp = format!(
                "Content-Security-Policy: default-src 'none'; script-src 'self'; \
                 style-src 'self'; img-src data:; connect-src 'self' ws://{host}; \
                 base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\n"
            );
            respond(
                &mut stream_out,
                "200 OK",
                "text/html; charset=utf-8",
                &csp,
                INDEX_HTML.as_bytes(),
            )
        }
        ("GET", "/app.js") if by_cookie => respond(
            &mut stream_out,
            "200 OK",
            "text/javascript; charset=utf-8",
            "",
            APP_JS.as_bytes(),
        ),
        ("GET", "/app.css") if by_cookie => respond(
            &mut stream_out,
            "200 OK",
            "text/css; charset=utf-8",
            "",
            APP_CSS.as_bytes(),
        ),
        ("GET", "/ws") if by_cookie => websocket(request, reader, stream_out, shared),
        ("GET", "/api/ping") if by_query => plain(&mut stream_out, "200 OK", "vaayu\n"),
        ("POST", "/api/open") if by_query => {
            let length: usize = request
                .header("content-length")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            if length > MAX_BODY {
                return plain(&mut stream_out, "413 Payload Too Large", "too large\n");
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body)?;
            let path = String::from_utf8(body)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "path is not UTF-8"))?;
            let _ = shared.inbound.send(Inbound::Open(path));
            plain(&mut stream_out, "200 OK", "ok\n")
        }
        ("GET", "/") | ("GET", "/ws") | ("GET", "/app.js") | ("GET", "/app.css") => plain(
            &mut stream_out,
            "401 Unauthorized",
            "This Vaayu session needs its private link. Run `vaayu --web` in the project \
             again (or `:web` in the terminal editor) to get it.\n",
        ),
        _ => plain(&mut stream_out, "404 Not Found", "not found\n"),
    }
}

fn websocket(
    request: Request,
    reader: BufReader<TcpStream>,
    mut stream: TcpStream,
    shared: Arc<Shared>,
) -> io::Result<()> {
    let upgrade = request
        .header("upgrade")
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
    let key = request.header("sec-websocket-key");
    let (true, Some(key)) = (upgrade, key) else {
        return plain(&mut stream, "400 Bad Request", "expected a WebSocket\n");
    };
    write!(
        stream,
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\n\r\n",
        derive_accept_key(key.as_bytes())
    )?;
    stream.flush()?;

    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE));
    let read_stream = reader.into_inner();
    read_stream.set_read_timeout(None)?;
    // One socket, two directions on two threads, so a large frame on its way
    // out never delays a keystroke on its way in.
    let mut incoming = WebSocket::from_raw_socket(read_stream, Role::Server, Some(config));
    let outgoing = WebSocket::from_raw_socket(stream.try_clone()?, Role::Server, Some(config));

    let queue = Arc::new(OutQueue::default());
    queue.push(Outbound::Text(protocol::hello(&shared.font)));
    let id = shared.hub.attach(queue.clone(), stream);

    let writer = std::thread::Builder::new()
        .name("web-write".into())
        .spawn(move || write_loop(outgoing, queue))?;

    let mut attached = false;
    while let Ok(message) = incoming.read() {
        let Message::Text(text) = message else {
            if message.is_close() {
                break;
            }
            continue;
        };
        let Ok(message) = serde_json::from_str::<ClientMessage>(text.as_str()) else {
            continue;
        };
        let inbound = match message {
            ClientMessage::Size { cols, rows, cw, ch } if !attached && cols > 0 && rows > 0 => {
                attached = true;
                Inbound::Attached {
                    cols,
                    rows,
                    cell: (cw, ch),
                }
            }
            ClientMessage::Clipboard { text } => Inbound::Clipboard(text),
            other => match protocol::input_event(other) {
                Some(event) => Inbound::Input(event),
                None => continue,
            },
        };
        if shared.inbound.send(inbound).is_err() {
            break;
        }
    }
    if shared.hub.detach(id) {
        let _ = shared.inbound.send(Inbound::Detached);
    }
    drop(incoming);
    let _ = writer.join();
    Ok(())
}

fn write_loop(mut socket: WebSocket<TcpStream>, queue: Arc<OutQueue>) {
    while let Some(items) = queue.take() {
        let mut frames: Vec<u8> = Vec::new();
        let mut failed = false;
        for item in items {
            match item {
                Outbound::Frame(frame) => {
                    queue.bytes.fetch_sub(frame.len(), Ordering::Relaxed);
                    // Frames that piled up while the socket was busy travel as
                    // one message; only the final state of each row matters.
                    frames.extend_from_slice(&frame);
                }
                Outbound::Text(text) => {
                    failed |= send_frames(&mut socket, &mut frames).is_err();
                    failed |= socket.send(Message::Text(Utf8Bytes::from(text))).is_err();
                }
            }
        }
        failed |= send_frames(&mut socket, &mut frames).is_err();
        if failed {
            break;
        }
    }
    queue.mark_delivered();
}

fn send_frames(socket: &mut WebSocket<TcpStream>, frames: &mut Vec<u8>) -> tungstenite::Result<()> {
    if frames.is_empty() {
        return Ok(());
    }
    socket.send(Message::Binary(std::mem::take(frames).into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_hosts_are_served() {
        for host in [
            "127.0.0.1:7420",
            "localhost:7420",
            "localhost",
            "[::1]:7420",
        ] {
            assert!(host_is_loopback(host), "{host}");
        }
        for host in [
            "",
            "evil.example:7420",
            "127.0.0.1.evil.example",
            "localhost.evil.example:7420",
            "[::1].evil.example",
        ] {
            assert!(!host_is_loopback(host), "{host}");
        }
    }

    #[test]
    fn cross_site_origins_are_refused() {
        let host = "127.0.0.1:7420";
        assert!(origin_allowed(None, host));
        assert!(origin_allowed(Some("http://127.0.0.1:7420"), host));
        assert!(!origin_allowed(Some("http://127.0.0.1:9999"), host));
        assert!(!origin_allowed(Some("https://evil.example"), host));
        assert!(!origin_allowed(Some("null"), host));
    }

    #[test]
    fn tokens_are_long_random_and_compared_exactly() {
        let (a, b) = (new_token().unwrap(), new_token().unwrap());
        assert_eq!(a.len(), 48);
        assert_ne!(a, b);
        assert!(tokens_match(&a, &a));
        assert!(!tokens_match(&a, &b));
        assert!(!tokens_match(&a, &a[1..]));
    }
}
