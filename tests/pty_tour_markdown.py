#!/usr/bin/env python3
"""Tour Markdown: real terminal styling, scrolling, raw edits and navigation.

Usage: python tests/pty_tour_markdown.py BINARY [artifact-directory]
"""
import json
from pathlib import Path
import sys
import tempfile

import pty_tour_visuals as ui


DESCRIPTION = """# Markdown_heading

Read **bold_word**, *italic_word* and `inline_token`.

- First_item
- Second_item

[Guide_link](https://example.com/docs)

| Phase | Purpose |
| --- | --- |
| Cache | Reuse |

```rust
fn render_step() {
    let answer = 42;
}
```

""" + "\n".join(f"- ROW_{i:03}" for i in range(100))


def body(session):
    divider = next(i for i, row in enumerate(session.screen.display) if "─" * 10 in row)
    return "\n".join(session.screen.display[divider + 1:-2])


def scroll_to(session, needle):
    session.key("g", .015)
    for _ in range(180):
        if needle in body(session):
            return session.cell_at(needle)
        session.key("j", .012)
    raise AssertionError((needle, session.text()))


def run(binary, cols, rows, colors, artifacts=None):
    ui.pyte.graphics.FG_BG_256[:16] = colors
    with tempfile.TemporaryDirectory(prefix="vaayu-tour-markdown-") as tmp:
        root = Path(tmp)
        original_source = ui.write_fixture(root)
        path = root / ".tours/visual.tour"
        tour = json.loads(path.read_text())
        tour["steps"][0]["description"] = DESCRIPTION
        original_tour = json.dumps(tour)
        path.write_text(original_tour)
        session = ui.Session(binary, root, cols, rows)
        try:
            session.key(":tourlayout horizontal\r")
            session.key(":tour visual\r", .12)
            session.key(":tourmarkdown on\r")
            session.key("K")
            for theme in ui.THEMES:
                session.key(":colorscheme " + theme + "\r")
                heading = scroll_to(session, "Markdown_heading")
                assert heading.bold, (theme, session.text())
                assert "# Markdown_heading" not in body(session)
                assert scroll_to(session, "bold_word").bold
                assert scroll_to(session, "italic_word").italics
                inline = scroll_to(session, "inline_token")
                assert "`inline_token`" not in body(session)
                scroll_to(session, "First_item")
                scroll_to(session, "Guide_link")
                scroll_to(session, "Cache")
                keyword = scroll_to(session, "fn render_step")
                if theme == "default":
                    assert keyword.fg != session.cell_at("render_step").fg
                    session.screenshot(artifacts, "code")
                    scroll_to(session, "bold_word")
                    session.screenshot(artifacts, "rendered")
                # Inline code must use the theme's explicit color.
                assert inline.fg != "default", (theme, inline)
            session.key("G")
            end = body(session)
            session.key("G")
            assert body(session) == end
            assert ",v? Help" in session.screen.display[-2]
            scroll_to(session, "ROW_099")
            session.resize(100, 30)
            scroll_to(session, "Markdown_heading")
            session.resize(cols, rows)
            session.key(":colorscheme default\r")
            session.key("\x1b")
            session.wait("Tour Markdown off")
            session.key("/bold_word\r")
            session.wait("**bold_word**")
            session.screenshot(artifacts, "raw")
            session.key(":nohlsearch\r")
            session.key("ggi**EDITED_BOLD**\r\x1b")
            session.key(",mp")
            assert scroll_to(session, "EDITED_BOLD").bold
            session.key("\x1b")
            session.wait("**EDITED_BOLD**")
            session.key(":tourmarkdown on\r")
            # Tour keys and source previews remain usable from this pane.
            session.key("]h")
            session.wait("Highlight 1/3")
            session.wait("read_input();")
            session.key("K")
            for view in (",vf", ",vd"):
                session.key(view, .12)
                session.key("\x0f")
                session.wait("Tour Source view")
                assert "[Tour Flow]" not in session.text()
                assert "[Tour Diff]" not in session.text()
                scroll_to(session, "Markdown_heading")
            session.key("]t")
            scroll_to(session, "Plain step")
            session.key("[t")
            assert scroll_to(session, "bold_word").bold
            session.key("]q")
            session.wait("Tour ended")
            session.wait("read_input();")
            assert "[Tour Markdown]" not in session.text()
            assert path.read_text() == original_tour
            assert (root / "demo.txt").read_text() == original_source
        finally:
            session.close()
    print(f"Tour Markdown PTY passed: {cols}x{rows}, {len(ui.THEMES)} themes, "
          f"palette={'pale' if colors == ui.PALE else 'stock'}", flush=True)


if __name__ == "__main__":
    binary = str(Path(sys.argv[1]).resolve())
    artifacts = Path(sys.argv[2]).resolve() if len(sys.argv) > 2 else None
    stock = list(ui.pyte.graphics.FG_BG_256[:16])
    for cols, rows in ((40, 12), (80, 24), (140, 40)):
        for colors in (stock, ui.PALE):
            output = artifacts if (cols, rows) == (140, 40) and colors == stock else None
            run(binary, cols, rows, colors, output)
