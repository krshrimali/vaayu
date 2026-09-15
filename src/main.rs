mod buffer;
mod command;
mod completion;
mod config;
mod editor;
mod gitdiff;
mod insert;
mod key;
mod lsp;
mod mode;
mod motion;
mod normal;
mod operator;
mod picker;
mod registers;
mod render;
mod search;
mod syntax;
mod textobject;
mod vimregex;
mod visual;

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
        println!("anvil {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let config = Config::load();
    let mut ed = Editor::new(config);
    if let Some(path) = args.first() {
        ed.open_file(PathBuf::from(path))?;
    }

    install_panic_hook();
    render::setup_terminal()?;
    let result = run(&mut ed);
    render::teardown_terminal()?;
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

    loop {
        let (cols, rows) = crossterm::terminal::size()?;
        render::adjust_viewport(ed, rows.saturating_sub(2) as usize);
        ed.ensure_syntax();
        ed.ensure_git();
        ed.sync_lsp();
        ed.poll_lsp_events();
        render::draw(&mut stdout, ed, cols, rows)?;

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
                dispatch_event(ed, event::read()?);
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
                dispatch_event(ed, event::read()?);
                break;
            }
            if ed.poll_lsp_events() {
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
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(150);

fn dispatch_event(ed: &mut Editor, ev: Event) {
    if let Event::Key(k) = ev {
        if k.kind == KeyEventKind::Press || k.kind == KeyEventKind::Repeat {
            if let Some(key) = Key::from_event(k) {
                ed.feed_key(key);
            }
        }
    }
}
