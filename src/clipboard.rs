//! System clipboard integration, matching the two-path approach this
//! machine's ~/.config/nvim already uses: local sessions shell out to the
//! desktop clipboard tool (`wl-copy`/`wl-paste` on Wayland, `xclip` on X11);
//! SSH sessions tunnel a *copy* through the terminal via OSC 52 instead,
//! since the remote host has no access to the local machine's clipboard.
//!
//! Paste is never attempted over OSC 52: querying the terminal for its
//! clipboard contents blocks waiting for a reply that most terminals and
//! multiplexers never send (the same reason the nvim config's OSC52 setup
//! only wires up copy and reads paste from its own unnamed register).

use std::io::{self, Write};
use std::process::{Command, Stdio};

fn is_ssh() -> bool {
    std::env::var_os("SSH_TTY").is_some() || std::env::var_os("SSH_CONNECTION").is_some()
}

fn local_copy_cmd() -> Option<(&'static str, &'static [&'static str])> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        Some(("wl-copy", &[]))
    } else if std::env::var_os("DISPLAY").is_some() {
        Some(("xclip", &["-selection", "clipboard"]))
    } else {
        None
    }
}

fn local_paste_cmd() -> Option<(&'static str, &'static [&'static str])> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        Some(("wl-paste", &["-n"]))
    } else if std::env::var_os("DISPLAY").is_some() {
        Some(("xclip", &["-selection", "clipboard", "-o"]))
    } else {
        None
    }
}

/// Copies `text` to the system clipboard. Never blocks the caller waiting on
/// an external process: over SSH this writes an escape sequence directly and
/// returns; locally it spawns the clipboard tool and detaches (`wl-copy` in
/// particular needs to keep running in the background to *serve* the
/// selection to other apps, so we deliberately don't wait for it to exit).
pub fn copy(text: &str) {
    if cfg!(test) {
        return;
    }
    if is_ssh() {
        copy_osc52(text);
        return;
    }
    let Some((cmd, args)) = local_copy_cmd() else {
        return;
    };
    let text = text.to_string();
    std::thread::spawn(move || {
        if let Ok(mut child) = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
        }
    });
}

/// Reads the system clipboard, local sessions only (see module docs for why
/// SSH always returns `None` here). Runs a subprocess synchronously -- fine
/// for an explicit, infrequent user action like `p`, unlike anything on the
/// per-keystroke path.
pub fn paste() -> Option<String> {
    if cfg!(test) {
        return None;
    }
    if is_ssh() {
        return None;
    }
    let (cmd, args) = local_paste_cmd()?;
    let output = Command::new(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn copy_osc52(text: &str) {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    // Cap payload size: most terminals silently ignore an OSC52 sequence
    // past some limit (commonly ~100KB) rather than erroring, so a huge
    // yank would otherwise fail invisibly. Better to skip it with a visible
    // reason than send a truncated/corrupt sequence.
    if encoded.len() > 100_000 {
        return;
    }
    let osc = format!("\x1b]52;c;{}\x07", encoded);
    // Inside tmux a bare OSC 52 is swallowed unless it's wrapped in tmux's
    // passthrough form: `ESC Ptmux; <payload, each ESC doubled> ESC \`.
    // Detect tmux via $TMUX and wrap so the copy actually reaches the
    // outer terminal rather than being dropped by the multiplexer.
    let seq = if std::env::var_os("TMUX").is_some() {
        format!("\x1bPtmux;{}\x1b\\", osc.replace('\x1b', "\x1b\x1b"))
    } else {
        osc
    };
    let _ = write_direct(&seq);
}

fn write_direct(s: &str) -> io::Result<()> {
    let mut out = io::stdout();
    out.write_all(s.as_bytes())?;
    out.flush()
}
