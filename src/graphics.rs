//! Kitty image uploads and Unicode placeholder cells. Image cells participate
//! in the normal row compositor, so scrolling, clipping and overlays are text
//! operations; no floating image can bleed across a pane or dialog.
use base64::Engine as _;
use crossterm::{
    event::{Event, KeyCode, KeyEvent, KeyModifiers},
    queue,
    style::Print,
};
use std::{
    collections::{HashMap, HashSet},
    io::{self, Write},
    time::{Duration, Instant},
};

const PROBE_ID: u32 = 2_113_929_214;
const DIACRITICS: [u32; 256] = include!("graphics_diacritics.rs");

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct ImageRow {
    pub id: u32,
    pub row: u16,
    pub columns: u16,
    pub rows: u16,
}

impl ImageRow {
    fn placement(self) -> u32 {
        (u32::from(self.columns) << 9) | u32::from(self.rows)
    }
}

#[derive(Default)]
pub struct Capability {
    pub available: bool,
    sent: bool,
    replied: bool,
    capture: Vec<Event>,
    since: Option<Instant>,
}

impl Capability {
    pub fn probe(&mut self, out: &mut impl Write) -> io::Result<()> {
        if self.sent {
            return Ok(());
        }
        // A plain tmux/screen connection needs negotiated passthrough. WezTerm
        // acknowledges basic Kitty graphics but does not yet ship Unicode
        // placeholders (wezterm/wezterm#986). Both use Unicode in auto mode.
        if std::env::var_os("TMUX").is_some()
            || std::env::var("TERM").is_ok_and(|t| t.starts_with("screen") || t.starts_with("tmux"))
            || std::env::var_os("WEZTERM_PANE").is_some()
            || std::env::var("TERM_PROGRAM").is_ok_and(|t| t.eq_ignore_ascii_case("wezterm"))
        {
            self.sent = true;
            self.replied = true;
            return Ok(());
        }
        write!(out, "\x1b_Ga=q,i={PROBE_ID},s=1,v=1,f=24,t=d;AAAA\x1b\\")?;
        self.sent = true;
        Ok(())
    }

    /// Crossterm 0.28 reports APC replies as Alt-_ / characters / Alt-\.
    /// Recognize only our own outstanding probe. Anything else is replayed as
    /// the original events, preserving actual keyboard and mouse input.
    pub fn filter(&mut self, event: Event) -> Vec<Event> {
        let mut expired = self.expired_input();
        if !expired.is_empty() {
            expired.extend(self.filter(event));
            return expired;
        }
        if !self.capture.is_empty() && !matches!(&event, Event::Key(_)) {
            return vec![event];
        }
        if self.capture.is_empty() {
            if self.sent
                && !self.replied
                && matches!(&event, Event::Key(KeyEvent { code: KeyCode::Char('_'), modifiers, .. }) if *modifiers == KeyModifiers::ALT)
            {
                self.capture.push(event);
                self.since = Some(Instant::now());
                return Vec::new();
            }
            return vec![event];
        }
        let terminator = matches!(&event, Event::Key(KeyEvent { code: KeyCode::Char('\\'), modifiers, .. }) if *modifiers == KeyModifiers::ALT);
        self.capture.push(event);
        let prefix = format!("Gi={PROBE_ID};");
        let text: Option<String> = self
            .capture
            .iter()
            .skip(1)
            .take(
                self.capture
                    .len()
                    .saturating_sub(if terminator { 2 } else { 1 }),
            )
            .map(|e| match e {
                Event::Key(KeyEvent {
                    code: KeyCode::Char(c),
                    modifiers,
                    ..
                }) if c.is_ascii()
                    && !c.is_control()
                    && !modifiers.intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) =>
                {
                    Some(*c)
                }
                _ => None,
            })
            .collect();
        if let Some(text) = text {
            if terminator && text.starts_with(&prefix) {
                self.available = text[prefix.len()..].trim() == "OK";
                self.replied = true;
                self.capture.clear();
                self.since = None;
                return Vec::new();
            }
            if !terminator
                && self.capture.len() < 128
                && (prefix.starts_with(&text) || text.starts_with(&prefix))
            {
                return Vec::new();
            }
        }
        self.since = None;
        std::mem::take(&mut self.capture)
    }

    pub fn expired_input(&mut self) -> Vec<Event> {
        if self
            .since
            .is_some_and(|s| s.elapsed() >= Duration::from_millis(200))
        {
            self.since = None;
            return std::mem::take(&mut self.capture);
        }
        Vec::new()
    }
}

#[derive(Default)]
pub struct Uploads {
    images: HashMap<u32, HashSet<u32>>,
}

impl Uploads {
    pub fn reset(&mut self) {
        self.images.clear();
    }

    pub fn sync(
        &mut self,
        out: &mut impl Write,
        visible: &[ImageRow],
        service: &crate::mermaid::Service,
    ) -> io::Result<()> {
        let mut needed: HashMap<u32, HashSet<u32>> = HashMap::new();
        for cell in visible {
            needed.entry(cell.id).or_default().insert(cell.placement());
        }
        for (id, placements) in &mut self.images {
            if let Some(current) = needed.get(id) {
                for placement in placements.iter().filter(|p| !current.contains(p)) {
                    write!(out, "\x1b_Ga=d,d=i,i={id},p={placement},q=2;\x1b\\")?;
                }
                placements.retain(|p| current.contains(p));
            }
        }
        // Delete just Vaayu-owned IDs; never delete all images in the terminal.
        // Removal is issued before row painting, then visible images are uploaded.
        for cell in visible {
            if let std::collections::hash_map::Entry::Vacant(entry) = self.images.entry(cell.id) {
                let Some(artifact) = service.image(cell.id) else {
                    continue;
                };
                let crate::mermaid::Artifact::Image { png, .. } = artifact.as_ref() else {
                    continue;
                };
                let encoded = base64::engine::general_purpose::STANDARD.encode(png);
                let mut chunks = encoded.as_bytes().chunks(4096).peekable();
                let mut first = true;
                while let Some(chunk) = chunks.next() {
                    let more = u8::from(chunks.peek().is_some());
                    if first {
                        write!(out, "\x1b_Ga=t,f=100,t=d,i={},q=2,m={more};", cell.id)?;
                        first = false;
                    } else {
                        write!(out, "\x1b_Gm={more};")?;
                    }
                    out.write_all(chunk)?;
                    out.write_all(b"\x1b\\")?;
                }
                entry.insert(HashSet::new());
            }
            let placements = self.images.get_mut(&cell.id).unwrap();
            if placements.insert(cell.placement()) {
                write!(
                    out,
                    "\x1b_Ga=p,U=1,i={},p={},c={},r={},q=2;\x1b\\",
                    cell.id,
                    cell.placement(),
                    cell.columns,
                    cell.rows
                )?;
            }
        }
        Ok(())
    }

    pub fn delete_unused(&mut self, out: &mut impl Write, visible: &[ImageRow]) -> io::Result<()> {
        let ids: HashSet<u32> = visible.iter().map(|cell| cell.id).collect();
        for id in self.images.keys().filter(|id| !ids.contains(id)) {
            write!(out, "\x1b_Ga=d,d=I,i={id},q=2;\x1b\\")?;
        }
        self.images.retain(|id, _| ids.contains(id));
        Ok(())
    }
}

/// Emits only the visible columns. Explicit coordinates let any subset of
/// rows/columns display the corresponding image crop after scrolling.
pub fn draw_row(out: &mut Vec<u8>, cell: ImageRow, left: usize, width: usize) -> io::Result<usize> {
    // These colors encode protocol IDs, not appearance. NO_COLOR must not
    // suppress them, or the terminal cannot identify the image/placement.
    let placement = cell.placement();
    write!(
        out,
        "\x1b[38;2;{};{};{}m\x1b[58;2;{};{};{}m",
        (cell.id >> 16) as u8,
        (cell.id >> 8) as u8,
        cell.id as u8,
        (placement >> 16) as u8,
        (placement >> 8) as u8,
        placement as u8
    )?;
    let end = usize::from(cell.columns).min(left.saturating_add(width));
    for &column_mark in DIACRITICS.iter().take(end).skip(left.min(end)) {
        let row_mark = char::from_u32(DIACRITICS[usize::from(cell.row)]).unwrap();
        let col_mark = char::from_u32(column_mark).unwrap();
        queue!(out, Print(format!("\u{10eeee}{row_mark}{col_mark}")))?;
    }
    Ok(end.saturating_sub(left))
}

pub fn cell_ratio() -> f64 {
    crossterm::terminal::window_size()
        .ok()
        .filter(|s| s.width > 0 && s.height > 0 && s.columns > 0 && s.rows > 0)
        .map(|s| {
            (f64::from(s.width) / f64::from(s.columns)) / (f64::from(s.height) / f64::from(s.rows))
        })
        .filter(|ratio| *ratio > 0.1 && *ratio < 2.0)
        .unwrap_or(0.5)
}

pub fn cell_width() -> u16 {
    crossterm::terminal::window_size()
        .ok()
        .filter(|s| s.width > 0 && s.columns > 0)
        .map(|s| (s.width / s.columns).clamp(4, 64))
        .unwrap_or(8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn character(c: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }
    fn alt(c: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT))
    }

    #[test]
    fn probe_replies_are_consumed_and_unrelated_input_is_preserved() {
        let mut capability = Capability {
            sent: true,
            ..Default::default()
        };
        assert!(capability.filter(alt('_')).is_empty());
        for c in format!("Gi={PROBE_ID};OK").chars() {
            assert!(capability.filter(character(c)).is_empty());
        }
        assert_eq!(
            capability.filter(Event::Resize(80, 24)),
            vec![Event::Resize(80, 24)]
        );
        assert!(capability.filter(alt('\\')).is_empty());
        assert!(capability.available);
        assert_eq!(capability.filter(character('j')), vec![character('j')]);
        assert_eq!(capability.filter(alt('_')), vec![alt('_')]);

        let mut unsupported = Capability {
            sent: true,
            ..Default::default()
        };
        unsupported.filter(alt('_'));
        for c in format!("Gi={PROBE_ID};ENOTSUP").chars() {
            unsupported.filter(character(c));
        }
        assert!(unsupported.filter(alt('\\')).is_empty());
        assert!(!unsupported.available);

        let mut unrelated = Capability {
            sent: true,
            ..Default::default()
        };
        unrelated.filter(alt('_'));
        assert_eq!(
            unrelated.filter(character('x')),
            vec![alt('_'), character('x')]
        );
        unrelated.filter(alt('_'));
        unrelated.since = Some(Instant::now() - Duration::from_secs(1));
        assert_eq!(
            unrelated.filter(character('j')),
            vec![alt('_'), character('j')]
        );
    }

    #[test]
    fn image_coordinates_crop_exactly_and_placement_dimensions_do_not_collide() {
        let cell = ImageRow {
            id: 4,
            row: 2,
            columns: 100,
            rows: 30,
        };
        let mut row = Vec::new();
        assert_eq!(draw_row(&mut row, cell, 90, 5).unwrap(), 5);
        let row = String::from_utf8(row).unwrap();
        assert!(
            row.contains("\x1b[38;2;0;0;4m"),
            "image IDs must be encoded even with NO_COLOR"
        );
        assert_eq!(row.matches('\u{10eeee}').count(), 5);
        assert!(row.contains(char::from_u32(DIACRITICS[90]).unwrap()));
        let mut placements = HashSet::new();
        for columns in 1..=256 {
            for rows in 1..=256 {
                assert!(placements.insert(
                    ImageRow {
                        columns,
                        rows,
                        ..cell
                    }
                    .placement()
                ));
            }
        }
    }
}
