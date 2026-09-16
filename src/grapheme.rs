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
pub fn expand_tabs(text: &str, tab: usize) -> String {
    let mut out = String::new();
    let mut width = 0;
    for g in text.graphemes(true) {
        if g == "\t" {
            let n = tab.max(1) - width % tab.max(1);
            out.push_str(&" ".repeat(n));
            width += n;
        } else {
            out.push_str(g);
            width += unicode_width::UnicodeWidthStr::width(g);
        }
    }
    out
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
