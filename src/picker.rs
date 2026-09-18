use std::path::{Path, PathBuf};

use crate::editor::Editor;
use crate::key::Key;

const SKIP_DIRS: &[&str] = &[
    ".git",
    ".vaayu",
    "target",
    "node_modules",
    ".cache",
    "__pycache__",
    ".venv",
];
const MAX_FILES: usize = 40_000;
/// How many ranked matches the picker actually displays/keeps. The old
/// implementation scored and sorted *every* candidate, then threw all but
/// this many away -- wasted work that grows with the whole inventory
/// instead of with what's shown. `refilter` now keeps only this many
/// candidates in flight at once (see `RankStats` below).
const TAKE: usize = 500;

/// Counters `refilter` reports so ranking cost is visible (in the status
/// line and to tests) instead of just assumed: how many candidates were
/// considered, how many actually matched the query before truncation to
/// `TAKE`, and how long that took. `matched` in particular is what proves
/// the bounded top-k matcher below is really bounded -- it can be far
/// larger than `matches.len()` without `refilter` having sorted all of it.
#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub struct RankStats {
    pub scanned: usize,
    pub matched: usize,
    pub elapsed: std::time::Duration,
}

pub struct FilePicker {
    pub query: String,
    pub matches: Vec<(i64, String)>,
    pub selected: usize,
    pub stats: RankStats,
}

impl FilePicker {
    pub fn new(all_files: &[String]) -> FilePicker {
        let mut p = FilePicker {
            query: String::new(),
            matches: Vec::new(),
            selected: 0,
            stats: RankStats::default(),
        };
        p.refilter(all_files);
        p
    }

    pub fn refilter(&mut self, all_files: &[String]) {
        let start = std::time::Instant::now();
        if self.query.is_empty() {
            self.matches = all_files
                .iter()
                .take(TAKE)
                .map(|f| (0, f.clone()))
                .collect();
            self.stats = RankStats {
                scanned: all_files.len(),
                matched: all_files.len(),
                elapsed: start.elapsed(),
            };
        } else {
            let (top, matched) = top_k_matches(all_files, &self.query, TAKE);
            self.matches = top;
            self.stats = RankStats {
                scanned: all_files.len(),
                matched,
                elapsed: start.elapsed(),
            };
        }
        self.selected = 0;
    }
}

/// A candidate carrying its fuzzy score, ordered so that "greater" means
/// "ranks higher": higher score wins, and on a tie the shorter path wins
/// (matching the old full-sort's tie-break). Kept as its own type (instead
/// of a raw tuple) so that meaning is enforced by the type checker rather
/// than by remembering which tuple field means what at each call site.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Scored<'a> {
    score: i64,
    path: &'a str,
    // Original position in `candidates`. A true tie on (score, len) still
    // needs a deterministic winner -- the old full stable sort resolved
    // that by leaving equal-keyed elements in their original order, so
    // this reproduces the same result instead of leaving it to whatever
    // order the heap happens to visit ties in.
    idx: usize,
}
impl Ord for Scored<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.score
            .cmp(&other.score)
            .then_with(|| other.path.len().cmp(&self.path.len()))
            .then_with(|| other.idx.cmp(&self.idx))
    }
}
impl PartialOrd for Scored<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Bounded top-k selection: keeps only the best `k` matches seen so far in
/// a `k`-sized min-heap (ordered so the *worst* kept candidate is always at
/// the top), rather than scoring, collecting and sorting every one of
/// `candidates` before throwing all but `k` away. Once the heap is full,
/// a new candidate only costs a peek-and-maybe-replace against the current
/// worst kept match, so cost scales with `k` (bounded), not with a full
/// sort of however many candidates actually matched -- the property this
/// plan item asked for so a very large inventory doesn't pay for a sort it
/// throws almost all of away. Returns the top matches in the same
/// descending (score, then shorter-first) order the old full sort
/// produced, plus the total number of candidates that matched at all
/// (before truncation to `k`) for `RankStats`.
fn top_k_matches(candidates: &[String], query: &str, k: usize) -> (Vec<(i64, String)>, usize) {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let mut heap: BinaryHeap<Reverse<Scored>> = BinaryHeap::with_capacity(k + 1);
    let mut matched = 0usize;
    for (idx, f) in candidates.iter().enumerate() {
        let Some(score) = fuzzy_score(f, query) else {
            continue;
        };
        matched += 1;
        let candidate = Scored {
            score,
            path: f,
            idx,
        };
        if heap.len() < k {
            heap.push(Reverse(candidate));
        } else if let Some(Reverse(worst)) = heap.peek() {
            if candidate > *worst {
                heap.pop();
                heap.push(Reverse(candidate));
            }
        }
    }
    let mut top: Vec<Scored> = heap.into_iter().map(|Reverse(s)| s).collect();
    top.sort_by(|a, b| b.cmp(a));
    (
        top.into_iter()
            .map(|s| (s.score, s.path.to_string()))
            .collect(),
        matched,
    )
}

/// Case-insensitive subsequence fuzzy match. Returns None if `query` isn't a
/// subsequence of `candidate`; otherwise a score that rewards contiguous runs
/// and matches near the start (and especially near the last path segment).
fn fuzzy_score(candidate: &str, query: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    if candidate.is_ascii() && query.is_ascii() {
        return fuzzy_score_ascii(candidate.as_bytes(), query.as_bytes());
    }
    let cand: Vec<char> = candidate.chars().collect();
    // Per-char case folding (not `candidate.to_lowercase()` as a whole),
    // deliberately: some characters lowercase to more than one char (Turkish
    // 'İ' -> "i̇", for example), which would make `cand_lower` longer than
    // `cand` and desync the index used below to look back into `cand` --
    // real crash, reproduced by searching a filename containing 'İ'.
    let cand_lower: Vec<char> = cand
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let query_lower: Vec<char> = query
        .chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect();
    let basename_start = candidate
        .rfind('/')
        .map(|i| candidate[..i].chars().count() + 1)
        .unwrap_or(0);

    let mut ci = 0usize;
    let mut score: i64 = 0;
    let mut last_match: Option<usize> = None;
    for &qc in &query_lower {
        let mut found = None;
        while ci < cand_lower.len() {
            if cand_lower[ci] == qc {
                found = Some(ci);
                break;
            }
            ci += 1;
        }
        let idx = found?;
        score += 10;
        if idx >= basename_start {
            score += 15;
        }
        if let Some(last) = last_match {
            if idx == last + 1 {
                score += 20;
            }
        } else {
            score += (20i64 - idx.min(20) as i64) / 2;
        }
        if cand[idx].is_uppercase() {
            score += 3;
        }
        last_match = Some(idx);
        ci = idx + 1;
    }
    score -= cand.len() as i64 / 10;
    Some(score)
}

fn fuzzy_score_ascii(candidate: &[u8], query: &[u8]) -> Option<i64> {
    let basename_start = candidate
        .iter()
        .rposition(|b| *b == b'/')
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut ci = 0usize;
    let mut score = 0i64;
    let mut last_match = None;
    for qc in query.iter().map(u8::to_ascii_lowercase) {
        while ci < candidate.len() && candidate[ci].to_ascii_lowercase() != qc {
            ci += 1;
        }
        if ci == candidate.len() {
            return None;
        }
        score += 10;
        if ci >= basename_start {
            score += 15;
        }
        if let Some(last) = last_match {
            if ci == last + 1 {
                score += 20;
            }
        } else {
            score += (20i64 - ci.min(20) as i64) / 2;
        }
        if candidate[ci].is_ascii_uppercase() {
            score += 3;
        }
        last_match = Some(ci);
        ci += 1;
    }
    Some(score - candidate.len() as i64 / 10)
}

pub fn scan_files(root: &Path) -> Vec<String> {
    // ripgrep applies project ignore rules. Read incrementally and cap the
    // inventory rather than buffering an unbounded command output.
    if let Ok(mut child) = std::process::Command::new("rg")
        .current_dir(root)
        .args([
            "--files",
            "--hidden",
            "--glob",
            "!.git/**",
            "--glob",
            "!.vaayu/**",
            "--glob",
            "!target/**",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        use std::io::BufRead;
        let mut out = Vec::new();
        for line in std::io::BufReader::new(child.stdout.take().unwrap())
            .lines()
            .map_while(Result::ok)
        {
            out.push(line);
            if out.len() >= MAX_FILES {
                let _ = child.kill();
                break;
            }
        }
        let result = child.wait();
        if result.is_ok() {
            out.sort();
            return out;
        }
    }

    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if out.len() >= MAX_FILES {
            break;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Ok(ft) = entry.file_type() {
                if ft.is_symlink() {
                    continue;
                }
                if ft.is_dir() {
                    if SKIP_DIRS.contains(&name.as_ref()) {
                        continue;
                    }
                    stack.push(path);
                } else if ft.is_file() {
                    if let Ok(rel) = path.strip_prefix(root) {
                        out.push(rel.to_string_lossy().replace('\\', "/"));
                    }
                    if out.len() >= MAX_FILES {
                        break;
                    }
                }
            }
        }
    }
    out.sort();
    out
}

pub fn handle(ed: &mut Editor, key: Key) {
    match key {
        Key::Esc => {
            ed.last_picker = ed.file_picker.take();
            ed.last_resume = Some(crate::editor::ResumeTarget::Picker);
            ed.enter_normal();
        }
        Key::Enter | Key::Ctrl('v') | Key::Ctrl('x') | Key::Ctrl('t') => {
            let chosen = ed
                .file_picker
                .as_ref()
                .and_then(|p| p.matches.get(p.selected))
                .map(|(_, f)| f.clone());
            ed.last_picker = ed.file_picker.take();
            ed.last_resume = Some(crate::editor::ResumeTarget::Picker);
            ed.enter_normal();
            if let Some(rel) = chosen {
                match key {
                    Key::Ctrl('v') | Key::Ctrl('x') => {
                        ed.split_window(key == Key::Ctrl('v'), false)
                    }
                    Key::Ctrl('t') => ed.new_tab(),
                    _ => {}
                }
                if let Err(e) = ed.open_file(PathBuf::from(rel)) {
                    ed.set_message(format!("could not open: {}", e));
                }
            }
        }
        Key::Backspace => {
            if let Some(p) = &mut ed.file_picker {
                p.query.pop();
                p.refilter(&ed.all_files);
            }
        }
        Key::Char(c) => {
            if let Some(p) = &mut ed.file_picker {
                p.query.push(c);
            }
            if let Some(p) = &mut ed.file_picker {
                p.refilter(&ed.all_files);
            }
        }
        Key::Down | Key::Ctrl('n') => {
            if let Some(p) = &mut ed.file_picker {
                if p.selected + 1 < p.matches.len() {
                    p.selected += 1;
                }
            }
        }
        Key::Up | Key::Ctrl('p') => {
            if let Some(p) = &mut ed.file_picker {
                p.selected = p.selected.saturating_sub(1);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The brute-force reference this session's other tests already trust
    /// implicitly (score everything, sort everything, truncate) -- used
    /// here only to prove the bounded top-k matcher picks the *same* top
    /// entries, not a different-but-plausible-looking set.
    fn brute_force_top_k(candidates: &[String], query: &str, k: usize) -> Vec<(i64, String)> {
        let mut scored: Vec<(i64, &String)> = candidates
            .iter()
            .filter_map(|f| fuzzy_score(f, query).map(|s| (s, f)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.len().cmp(&b.1.len())));
        scored.truncate(k);
        scored.into_iter().map(|(s, f)| (s, f.clone())).collect()
    }

    #[test]
    fn top_k_matches_agrees_with_a_brute_force_sort_of_everything() {
        let candidates: Vec<String> = (0..2000)
            .map(|i| format!("src/module_{i}/file_{}.rs", i % 37))
            .collect();
        let (top, matched) = top_k_matches(&candidates, "modfile", 25);
        let expected = brute_force_top_k(&candidates, "modfile", 25);
        assert_eq!(top, expected);
        let brute_matched = candidates
            .iter()
            .filter(|f| fuzzy_score(f, "modfile").is_some())
            .count();
        assert_eq!(matched, brute_matched);
    }

    #[test]
    fn top_k_matches_returns_everything_when_fewer_candidates_match_than_k() {
        let candidates = vec!["src/foo.rs".to_string(), "src/bar.rs".to_string()];
        let (top, matched) = top_k_matches(&candidates, "foo", 25);
        assert_eq!(top.len(), 1);
        assert_eq!(matched, 1);
        assert_eq!(top[0].1, "src/foo.rs");
    }

    #[test]
    fn top_k_matches_reports_the_full_matched_count_even_though_kept_is_bounded() {
        // Every candidate matches an empty-ish query fragment shared by all,
        // far more than fit in a small k -- `matched` must still count all
        // of them, proving the heap didn't quietly drop the ones it evicted
        // from the running count, only from the kept set.
        let candidates: Vec<String> = (0..500).map(|i| format!("file_{i}.rs")).collect();
        let (top, matched) = top_k_matches(&candidates, "file", 10);
        assert_eq!(top.len(), 10);
        assert_eq!(matched, 500);
    }

    #[test]
    fn refilter_reports_scanned_and_matched_even_when_truncated_to_take() {
        let candidates: Vec<String> = (0..(TAKE * 3)).map(|i| format!("file_{i}.rs")).collect();
        let mut p = FilePicker::new(&candidates);
        p.query = "file".to_string();
        p.refilter(&candidates);
        assert_eq!(p.matches.len(), TAKE);
        assert_eq!(p.stats.matched, candidates.len());
        assert_eq!(p.stats.scanned, candidates.len());
    }

    #[test]
    fn refilter_empty_query_reports_scanned_equal_to_matched() {
        let candidates: Vec<String> = (0..50).map(|i| format!("file_{i}.rs")).collect();
        let p = FilePicker::new(&candidates);
        assert_eq!(p.stats.scanned, 50);
        assert_eq!(p.stats.matched, 50);
    }

    /// Not run by default (timing assertions are flaky on a shared,
    /// noisy machine -- see this session's own established rule for
    /// bench/latency.py) -- run explicitly with
    /// `cargo test --release picker::tests::benchmark -- --ignored --nocapture`
    /// to see actual old-vs-new numbers at scale.
    #[test]
    #[ignore]
    fn benchmark_top_k_vs_brute_force_at_a_million_candidates() {
        let candidates: Vec<String> = (0..1_000_000)
            .map(|i| format!("crate_{}/src/module_{}/lib.rs", i % 4000, i))
            .collect();
        let start = std::time::Instant::now();
        let brute = brute_force_top_k(&candidates, "modlib", TAKE);
        let brute_elapsed = start.elapsed();
        let start = std::time::Instant::now();
        let (top, _) = top_k_matches(&candidates, "modlib", TAKE);
        let top_k_elapsed = start.elapsed();
        assert_eq!(top, brute, "must still pick the same top matches");
        eprintln!(
            "brute-force full sort: {brute_elapsed:?}; bounded top-k: {top_k_elapsed:?} \
             ({:.1}x)",
            brute_elapsed.as_secs_f64() / top_k_elapsed.as_secs_f64().max(1e-9)
        );
    }

    #[test]
    fn refilter_scales_to_a_large_inventory_without_sorting_all_of_it() {
        // Not a timing assertion (this machine's timing is noisy under
        // load -- see bench/latency.py for the real perf comparison);
        // this just proves a million-entry inventory doesn't panic,
        // overflow or produce a wrong-sized result, which the old
        // "collect everything, sort everything" approach could still do
        // correctly but only by paying for a full sort of a million
        // scored entries every keystroke.
        let candidates: Vec<String> = (0..1_000_000)
            .map(|i| format!("crate_{}/src/module_{}/lib.rs", i % 4000, i))
            .collect();
        let mut p = FilePicker::new(&candidates);
        p.query = "modlib".to_string();
        p.refilter(&candidates);
        assert_eq!(p.matches.len(), TAKE);
        assert_eq!(p.stats.scanned, 1_000_000);
        assert!(p.stats.matched >= TAKE);
    }
}
