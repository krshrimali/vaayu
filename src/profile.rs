//! Opt-in per-frame stage timing, so a latency regression can be diagnosed
//! by looking at where the microseconds actually went instead of guessing
//! and re-benchmarking after each guess. Zero cost when disabled (a single
//! thread-local `Option` check per call).
//!
//! Enable with `VAAYU_PROFILE=/path/to/log vy file`. Each line is
//! `frame,stage,cumulative_micros_since_frame_start` -- the delta between
//! consecutive stages within a frame is that stage's own cost.

use std::cell::RefCell;
use std::fs::File;
use std::io::Write as _;
use std::time::Instant;

struct Profiler {
    file: File,
    frame: u64,
    t0: Instant,
}

thread_local! {
    static PROFILER: RefCell<Option<Profiler>> = const { RefCell::new(None) };
}

pub fn init() {
    if let Ok(path) = std::env::var("VAAYU_PROFILE") {
        if let Ok(file) = File::create(&path) {
            PROFILER.with(|p| *p.borrow_mut() = Some(Profiler { file, frame: 0, t0: Instant::now() }));
        }
    }
}

pub fn frame_start() {
    PROFILER.with(|p| {
        if let Some(prof) = p.borrow_mut().as_mut() {
            prof.frame += 1;
            prof.t0 = Instant::now();
        }
    });
}

pub fn mark(stage: &str) {
    PROFILER.with(|p| {
        if let Some(prof) = p.borrow_mut().as_mut() {
            let elapsed = prof.t0.elapsed().as_micros();
            let frame = prof.frame;
            let _ = writeln!(prof.file, "{},{},{}", frame, stage, elapsed);
        }
    });
}

/// Records an explicit duration against the current frame, for a sub-cost
/// accumulated across a loop (e.g. total syntax-lookup time across all
/// visible rows) rather than a single point-in-time checkpoint. Prefixed
/// `~` in the stage name so analysis can tell these apart from `mark()`'s
/// cumulative checkpoints -- they're independent totals, not deltas of each
/// other, and summing them alongside `mark()` deltas would double-count.
pub fn note(stage: &str, duration: std::time::Duration) {
    PROFILER.with(|p| {
        if let Some(prof) = p.borrow_mut().as_mut() {
            let frame = prof.frame;
            let _ = writeln!(prof.file, "{},~{},{}", frame, stage, duration.as_micros());
        }
    });
}
