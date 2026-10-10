use unicode_segmentation::UnicodeSegmentation;
pub fn floor(text: &str, col: usize) -> usize {
    let mut at = 0;
    for g in text.graphemes(true) {
        let end = at + g.chars().count();
        if col < end {
            return at;
        }
        at = end;
    }
    at
}
pub fn step(text: &str, col: usize, count: usize, forward: bool) -> usize {
    let mut points = vec![0];
    let mut at = 0;
    for g in text.graphemes(true) {
        at += g.chars().count();
        points.push(at);
    }
    let i = points.partition_point(|p| *p <= col).saturating_sub(1);
    points[if forward {
        i.saturating_add(count).min(points.len() - 1)
    } else {
        i.saturating_sub(count)
    }]
}
pub fn cell(text: &str, col: usize, tab: usize) -> usize {
    let mut at = 0;
    let mut width = 0;
    for g in text.graphemes(true) {
        if at >= col {
            break;
        }
        width += if g == "\t" {
            tab.max(1) - width % tab.max(1)
        } else {
            unicode_width::UnicodeWidthStr::width(g)
        };
        at += g.chars().count();
    }
    width
}

/// The column kept by vertical motions. A cursor on a tab tracks the
/// tab's final cell, as Vim does; other graphemes track their first cell.
pub fn cursor_cell(text: &str, col: usize, tab: usize) -> usize {
    let start = cell(text, col, tab);
    if text.chars().nth(col) == Some('\t') {
        start + tab.max(1) - start % tab.max(1) - 1
    } else {
        start
    }
}

/// Split a display-cell range without expanding tabs elsewhere in the line.
/// A partially selected tab is split into spaces; a whole selected tab is
/// retained. A partially selected wide grapheme is split into spaces too,
/// preserving the unselected display cells without retaining half a glyph.
pub fn split_cells(text: &str, left: usize, right: usize, tab: usize) -> (String, String, String) {
    let (mut before, mut selected, mut after) = (String::new(), String::new(), String::new());
    let mut at = 0;
    for g in text.graphemes(true) {
        let width = if g == "\t" {
            tab.max(1) - at % tab.max(1)
        } else {
            unicode_width::UnicodeWidthStr::width(g)
        };
        let end = at + width;
        if end <= left {
            before.push_str(g);
        } else if at >= right {
            after.push_str(g);
        } else if left == right && g != "\t" {
            // Inserting within a wide character keeps that character whole,
            // with padding before the insertion to reach its display cell.
            before.push_str(&" ".repeat(left.saturating_sub(at)));
            after.push_str(g);
        } else if at < left || end > right {
            before.push_str(&" ".repeat(left.saturating_sub(at)));
            selected.push_str(&" ".repeat(end.min(right).saturating_sub(at.max(left))));
            after.push_str(&" ".repeat(end.saturating_sub(right)));
        } else {
            selected.push_str(g);
        }
        at = end;
    }
    (before, selected, after)
}
pub fn column(text: &str, target: usize, end: bool) -> usize {
    let mut col = 0;
    let mut cell = 0;
    for g in text.graphemes(true) {
        let n = unicode_width::UnicodeWidthStr::width(g);
        if cell + n > target {
            return col
                + if end && cell < target {
                    g.chars().count()
                } else {
                    0
                };
        }
        col += g.chars().count();
        cell += n;
    }
    col
}
pub fn raw_column(text: &str, target: usize, tab: usize) -> usize {
    let mut col = 0;
    let mut cells = 0;
    for g in text.graphemes(true) {
        let width = if g == "\t" {
            tab.max(1) - cells % tab.max(1)
        } else {
            unicode_width::UnicodeWidthStr::width(g)
        };
        if cells + width > target {
            return col;
        }
        col += g.chars().count();
        cells += width;
    }
    col
}
