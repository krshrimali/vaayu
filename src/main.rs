mod actions;
mod align;
mod autopairs;
mod buffer;
mod clipboard;
mod command;
mod completion;
mod config;
mod editor;
mod files;
mod git_tools;
mod gitdiff;
mod grapheme;
mod indent;
mod insert;
mod jobs;
mod key;
mod language;
mod lsp;
mod markdown;
mod mode;
mod motion;
mod mouse;
mod navigation;
mod normal;
mod notes;
mod operator;
mod picker;
mod preview;
mod profile;
mod recovery;
mod registers;
mod render;
mod resources;
mod results;
mod review;
mod search;
mod session;
mod snippet;
mod surround;
mod syntax;
mod textobject;
mod undofile;
mod vimregex;
mod visual;
mod windows;

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};

use crate::config::Config;
use crate::editor::Editor;
use crate::key::Key;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("vaayu {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let config = Config::load();
    let mut ed = Editor::new(config);
    if let Some(path) = args.first() {
        ed.open_file(PathBuf::from(path))?;
    }

    profile::init();
    install_panic_hook();
    render::setup_terminal()?;
    let result = run(&mut ed);
    render::teardown_terminal()?;
    if result.is_ok() {
        ed.recovery.cleanup();
    }
    result
}

fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = render::teardown_terminal();
        original(info);
    }));
}

fn run(ed: &mut Editor) -> anyhow::Result<()> {
    let mut stdout = io::stdout();
    let mut frame_cache = render::FrameCache::new();
    let mut terminal_size = crossterm::terminal::size()?;

    loop {
        let (cols, rows) = terminal_size;
        render::prepare_view(ed, cols as usize, rows as usize);
        profile::mark("adjust_viewport");
        ed.ensure_syntax();
        profile::mark("ensure_syntax");
        ed.ensure_git();
        profile::mark("ensure_git");
        ed.sync_lsp();
        profile::mark("sync_lsp");
        ed.poll_lsp_events();
        profile::mark("poll_lsp_events");
        ed.ensure_markdown_preview(cols as usize);
        profile::mark("ensure_markdown_preview");
        render::draw(&mut stdout, ed, cols, rows, &mut frame_cache)?;
        profile::mark("draw");
        ed.start_file_scan();

        if ed.should_quit {
            break;
        }

        if let Some(since) = ed.pending_jk {
            let full = Duration::from_millis(ed.config.timeoutlen_ms.max(1));
            let elapsed = since.elapsed();
            if elapsed >= full {
                ed.flush_pending_jk();
                continue;
            }
            if event::poll(full - elapsed)? {
                let ev = event::read()?;
                profile::frame_start();
                if let Some(size) = dispatch_event(ed, ev) {
                    terminal_size = size;
                }
                profile::mark("feed_key");
            } else {
                ed.flush_pending_jk();
            }
            continue;
        }

        if let Some(normal::Awaiting::Leader { since, .. }) = &ed.pending.awaiting {
            let delay = Duration::from_millis(ed.config.whichkey_delay_ms.max(1));
            let elapsed = since.elapsed();
            if elapsed < delay {
                if event::poll(delay - elapsed)? {
                    let ev = event::read()?;
                    profile::frame_start();
                    if let Some(size) = dispatch_event(ed, ev) {
                        terminal_size = size;
                    }
                    profile::mark("feed_key");
                }
                // Loop back to redraw either way: a handled key may have left
                // Normal mode entirely, and an elapsed wait needs one more
                // frame to actually paint the which-key popup before the
                // plain blocking read below takes over.
                continue;
            }
        }

        // No key pending: block on real input, but wake periodically (without
        // redrawing) to check whether a language server sent something on its
        // own. Only actually loop back to the top -- and pay for a redraw --
        // once a key arrives or poll_lsp_events() reports real work; an idle
        // server produces no output here, matching the pre-LSP behavior of
        // blocking quietly for input.
        loop {
            if event::poll(IDLE_POLL_INTERVAL)? {
                let ev = event::read()?;
                profile::frame_start();
                if let Some(size) = dispatch_event(ed, ev) {
                    terminal_size = size;
                }
                profile::mark("feed_key");
                break;
            }
            if ed.poll_jobs() || ed.poll_lsp_events() {
                break;
            }
            if ed.syntax_catch_up_due() {
                ed.ensure_syntax();
                profile::mark("idle_syntax_catch_up");
                break;
            }
        }
    }

    Ok(())
}

/// How often the main loop wakes up with no key pressed, purely to check
/// whether a language server sent something (diagnostics, a hover/definition
/// reply) with no keystroke to trigger a redraw on its own. A wake-up that
/// finds nothing costs an mpsc try_recv per active client and produces zero
/// output -- only a wake-up that finds real work leads to a redraw.
// Language-server replies otherwise wait for this polling tick before they
// can reach the screen. Ten milliseconds keeps the simple crossterm loop
// responsive without redrawing on empty polls.
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(10);

fn dispatch_event(ed: &mut Editor, ev: Event) -> Option<(u16, u16)> {
    ed.note_input_activity();
    if let Event::Resize(cols, rows) = ev {
        return Some((cols, rows));
    }
    if let Event::Paste(text) = &ev {
        ed.insert_paste(text);
        return None;
    }
    if let Event::Mouse(m) = ev {
        mouse::handle(ed, m);
        return None;
    }
    if let Event::Key(k) = ev {
        if k.kind == KeyEventKind::Press || k.kind == KeyEventKind::Repeat {
            if let Some(key) = Key::from_event(k) {
                ed.feed_key(key);
            }
        }
    }
    None
}

#[cfg(test)]
mod regression;
