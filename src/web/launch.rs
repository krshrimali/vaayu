//! Starting a session and finding it again: the project's `.vaayu/web.json`
//! names the running server, so `:web` and `vaayu --web` reopen the session
//! that is already there instead of starting a second editor on the project.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::Options;
use crate::editor::Editor;

const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// This process's own address, when it is a web session.
static ACTIVE_URL: OnceLock<String> = OnceLock::new();

fn default_host() -> IpAddr {
    IpAddr::V4(Ipv4Addr::LOCALHOST)
}

#[derive(Serialize, Deserialize)]
pub struct State {
    /// What the server was bound to, which may be a wildcard.
    #[serde(default = "default_host")]
    pub host: IpAddr,
    pub port: u16,
    pub token: String,
}

impl State {
    /// How this machine reaches the server.
    fn local_address(&self) -> SocketAddr {
        let ip = match self.host {
            IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
            ip => ip,
        };
        SocketAddr::new(ip, self.port)
    }

    fn link(&self, address: SocketAddr) -> String {
        format!("http://{address}/?token={}", self.token)
    }

    fn url(&self) -> String {
        self.link(self.local_address())
    }

    /// The link for other machines, when a wildcard makes it differ from `url`.
    fn network_url(&self) -> Option<String> {
        if !self.host.is_unspecified() {
            return None;
        }
        // Connecting a UDP socket only picks a route; nothing is sent.
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
        socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
        let ip = socket.local_addr().ok()?.ip();
        (!ip.is_loopback()).then(|| self.link(SocketAddr::new(ip, self.port)))
    }
}

fn state_path(root: &Path) -> PathBuf {
    root.join(".vaayu").join("web.json")
}

pub fn write_state(root: &Path, state: &State) -> anyhow::Result<PathBuf> {
    let dir = root.join(".vaayu");
    if !dir.exists() {
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&dir)?;
    }
    anyhow::ensure!(
        !std::fs::symlink_metadata(&dir)?.file_type().is_symlink(),
        ".vaayu must not be a symlink"
    );
    // The address and token are nobody else's business, in Git or otherwise.
    crate::files::atomic_write(&dir.join(".gitignore"), b"*\n", true)?;
    let path = state_path(root);
    crate::files::atomic_write(&path, &serde_json::to_vec(state)?, true)?;
    let _ = ACTIVE_URL.set(state.network_url().unwrap_or_else(|| state.url()));
    Ok(path)
}

/// One HTTP exchange with a local session; true on a 200.
fn request(state: &State, method: &str, target: &str, body: &str) -> bool {
    let address = state.local_address();
    let Ok(mut stream) = TcpStream::connect_timeout(&address, PROBE_TIMEOUT) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(PROBE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(PROBE_TIMEOUT));
    let head = format!(
        "{method} {target} HTTP/1.1\r\nHost: {address}\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    if stream.write_all(head.as_bytes()).is_err() {
        return false;
    }
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    response.starts_with("HTTP/1.1 200")
}

fn running_session(root: &Path) -> Option<State> {
    let text = std::fs::read(state_path(root)).ok()?;
    let state: State = serde_json::from_slice(&text).ok()?;
    let target = format!("/api/ping?token={}", state.token);
    request(&state, "GET", &target, "").then_some(state)
}

fn absolute(root: &Path, file: &str) -> PathBuf {
    root.join(file)
}

/// True when `root` already has a live session; it was given `options.file` and
/// shown to the user, so this process has nothing left to do.
pub fn reuse_running(root: &Path, options: &Options) -> anyhow::Result<bool> {
    let Some(state) = running_session(root) else {
        return Ok(false);
    };
    if let Some(host) = options.host.filter(|host| *host != state.host) {
        anyhow::bail!(
            "this project's web session is already listening on {}; quit it before using --host {host}",
            state.host
        );
    }
    if let Some(file) = &options.file {
        let target = format!("/api/open?token={}", state.token);
        let path = absolute(root, file).to_string_lossy().into_owned();
        anyhow::ensure!(
            request(&state, "POST", &target, &path),
            "the running web session did not accept {file}"
        );
    }
    announce(&state, options);
    Ok(true)
}

/// The URL is the first line of stdout: `:web` reads it from there.
pub fn announce(state: &State, options: &Options) {
    let url = state.url();
    println!("{url}");
    if !state.host.is_loopback() {
        if let Some(network_url) = state.network_url() {
            println!("From another machine: {network_url}");
        }
        eprintln!(
            "Warning: listening on {} without encryption. Anyone on the network who gets this \
             link can read your files and run commands as you, and it crosses the network in \
             clear text.",
            SocketAddr::new(state.host, state.port)
        );
    }
    if !options.detached {
        println!("Vaayu is running in your browser. Quit with :q there or Ctrl-C here.");
    }
    if !options.no_open {
        open_browser(&url);
    }
}

fn open_browser(url: &str) {
    let configured = std::env::var("BROWSER")
        .ok()
        .filter(|b| !b.trim().is_empty());
    let mut candidates: Vec<(String, Vec<String>)> = Vec::new();
    // $BROWSER is a colon-separated list of commands; %s marks the URL.
    for entry in configured.iter().flat_map(|b| b.split(':')) {
        let mut words = entry.split_whitespace().map(str::to_string);
        if let Some(program) = words.next() {
            let mut args: Vec<String> = words.collect();
            if args.iter().any(|a| a.contains("%s")) {
                args.iter_mut().for_each(|a| *a = a.replace("%s", url));
            } else {
                args.push(url.to_string());
            }
            candidates.push((program, args));
        }
    }
    let system = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    candidates.push((system.to_string(), vec![url.to_string()]));
    for (program, args) in candidates {
        let spawned = Command::new(&program)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(mut child) = spawned {
            std::thread::spawn(move || child.wait());
            return;
        }
    }
    eprintln!("Could not start a browser; open the link above yourself.");
}

/// `:web [file]`, in either frontend.
pub fn ex_command(ed: &mut Editor, file: &str) {
    if let Some(url) = ACTIVE_URL.get() {
        crate::clipboard::copy(url);
        ed.set_message(format!("Web session: {url} (copied)"));
        return;
    }
    if ed.web_launch.is_some() {
        ed.set_message("The web editor is already starting");
        return;
    }
    let target = if file.is_empty() {
        ed.buf().path.clone()
    } else {
        Some(absolute(&ed.project_root, file))
    };
    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(e) => {
            ed.set_message(format!("web: {e}"));
            return;
        }
    };
    let mut command = Command::new(executable);
    command
        .args(["--web", "--web-detached"])
        .args(target.iter())
        .current_dir(&ed.project_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            ed.set_message(format!("web: {e}"));
            return;
        }
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    ed.web_launch = Some(receiver);
    ed.set_message("Starting the web editor...");
    std::thread::spawn(move || {
        let mut line = String::new();
        if let Some(stdout) = child.stdout.take() {
            let _ = BufReader::new(stdout).read_line(&mut line);
        }
        let url = line.trim().to_string();
        let _ = sender.send(if url.starts_with("http://") {
            Ok(url)
        } else {
            Err("the web editor did not start".to_string())
        });
        // Reap it if it stops while this editor is still open.
        let _ = child.wait();
    });
}

/// Reports a finished `:web` launch. True when there was something to report.
pub fn poll(ed: &mut Editor) -> bool {
    let Some(result) = ed.web_launch.as_ref().and_then(|rx| rx.try_recv().ok()) else {
        return false;
    };
    ed.web_launch = None;
    match result {
        Ok(url) => {
            let unsaved = ed.buffers.iter().filter(|b| b.is_modified()).count();
            ed.set_message(if unsaved == 0 {
                format!("Web editor: {url}")
            } else {
                format!("Web editor: {url} ({unsaved} unsaved here; :w to share them)")
            });
        }
        Err(e) => ed.set_message(format!("web: {e}")),
    }
    true
}

pub fn title(ed: &Editor) -> String {
    let project = ed
        .project_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let buffer = ed.buf();
    let marker = if buffer.is_modified() { "● " } else { "" };
    let name = buffer
        .path
        .as_deref()
        .map(|path| path.strip_prefix(&ed.project_root).unwrap_or(path))
        .map_or_else(|| buffer.name(), |path| path.display().to_string());
    format!("{marker}{name} — {project} — Vaayu")
}

impl Editor {
    /// Whether stopping now could lose anything.
    pub fn has_unsaved_or_running(&self) -> bool {
        self.notes.dirty
            || !self.terminals.is_empty()
            || self.buffers.iter().any(|b| b.is_modified())
    }
}
