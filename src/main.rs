mod buffer;
mod clipboard;
mod command;
mod completion;
mod config;
mod editor;
mod files;
mod git_tools;
mod gitdiff;
mod insert;
mod jobs;
mod key;
mod language;
mod lsp;
mod markdown;
mod mode;
mod motion;
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
mod results;
mod search;
mod syntax;
mod textobject;
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

    loop {
        let (cols, rows) = crossterm::terminal::size()?;
        profile::mark("terminal_size");
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
                dispatch_event(ed, ev);
                profile::mark("feed_key");
            } else {
                ed.flush_pending_jk();
            }
            continue;
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
                dispatch_event(ed, ev);
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
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(30);

fn dispatch_event(ed: &mut Editor, ev: Event) {
    if let Event::Paste(text) = &ev {
        ed.insert_paste(text);
        return;
    }
    if let Event::Key(k) = ev {
        if k.kind == KeyEventKind::Press || k.kind == KeyEventKind::Repeat {
            if let Some(key) = Key::from_event(k) {
                ed.feed_key(key);
            }
        }
    }
}

#[cfg(test)]
mod regression;
