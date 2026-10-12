#!/usr/bin/env python3
"""Tour visuals in a real PTY: colors, cache invalidation, focus, resize, jumps.

Usage: python tests/pty_tour_visuals.py target/release/vaayu [artifact-directory]
The optional screenshots reproduce the PTY's actual character/color cells.
"""
import codecs
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time

import pyte
import pyte.graphics

THEMES = ("default", "mono", "warm", "cool", "gruvbox", "gruvbox-light",
          "flexoki", "flexoki-light", "tokyonight", "tokyonight-day")
PALE = ["2c3439", "bf616a", "a3be8c", "ebcb8b", "81b6af", "b48ead", "86be98", "d6c5aa",
        "607078", "e08787", "b4ce9f", "f0d49a", "9bc9c1", "cba2c5", "a1d2b2", "d8c9ad"]


def contrast(fg, bg):
    def luminance(color):
        assert len(color) == 6, ("explicit highlight colors required", color)
        values = [int(color[i:i + 2], 16) / 255 for i in (0, 2, 4)]
        linear = [v / 12.92 if v <= .04045 else ((v + .055) / 1.055) ** 2.4 for v in values]
        return sum(v * w for v, w in zip(linear, (.2126, .7152, .0722)))
    a, b = luminance(fg), luminance(bg)
    return (max(a, b) + .05) / (min(a, b) + .05)


def write_fixture(root, large=False):
    config = root / "config/vaayu"
    config.mkdir(parents=True)
    (config / "config.toml").write_text(
        'jk_escape=false\nclipboard_unnamedplus=false\nnumber=true\nwatch=false\nprogress=false\n'
    )
    source = "intro\nread_input();\nupdate_buffer();\nrequest_redraw();\ntail\n"
    if large:
        source += "".join(f"plain source row {i}\n" for i in range(50000))
    (root / "demo.txt").write_text(source)
    (root / "other.txt").write_text("other intro\npaint_screen();\n")
    tour = {"title": "Visual tour", "steps": [
        {"file": "demo.txt", "line": 2, "endLine": 4, "pattern": "read_input();",
         "description": "Follow the edit.", "vaayu": {
             "title": "Decode then draw", "takeaway": "Only the affected rows change.",
             "annotations": [
                 {"id": "read", "line": 2, "pattern": "read_input();", "label": "Read input"},
                 {"id": "update", "line": 3, "pattern": "update_buffer();", "label": "Change buffer"},
                 {"id": "paint", "file": "other.txt", "line": 2, "pattern": "paint_screen();", "label": "Paint screen"}],
             "connections": [{"from": "update", "to": "paint", "label": "Schedules painting"}],
             "diff": {"kind": "snippets", "before": "old_value\nshared\n", "after": "new_value\nshared\n",
                      "label": "A focused example"}}},
        {"file": "demo.txt", "line": 5, "description": "Plain step"},
        {"file": "demo.txt", "line": 2, "pattern": "read_input();", "description": "Actual Git comparison",
         "vaayu": {"diff": {"kind": "git", "base": "HEAD", "target": "buffer", "label": "Current edit"}}}
    ]}
    (root / ".tours").mkdir()
    (root / ".tours/visual.tour").write_text(json.dumps(tour))
    for args in (["init", "-q"], ["add", "demo.txt"],
                 ["-c", "user.name=Test", "-c", "user.email=test@example.com", "commit", "-qm", "base"]):
        subprocess.run(["git", "-C", str(root), *args], check=True, stdout=subprocess.DEVNULL)
    return source


class Session:
    def __init__(self, binary, root, cols=100, rows=30):
        self.cols, self.rows = cols, rows
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.chdir(root)
            os.environ.pop("NO_COLOR", None)
            os.environ.update(TERM="xterm-256color", XDG_CONFIG_HOME=str(root / "config"),
                              XDG_DATA_HOME=str(root / "data"), XDG_STATE_HOME=str(root / "state"))
            os.execv(binary, [binary, "demo.txt"])
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        self.screen = pyte.Screen(cols, rows)
        self.stream = pyte.Stream(self.screen)
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self.drain(.3)

    def drain(self, seconds=.07):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            ready, _, _ = select.select([self.fd], [], [], min(.01, max(0, end - time.monotonic())))
            if ready:
                try:
                    data = os.read(self.fd, 65536)
                except OSError:
                    break
                if not data:
                    break
                self.stream.feed(self.decoder.decode(data))

    def key(self, value, seconds=.07):
        os.write(self.fd, value.encode())
        self.drain(seconds)

    def text(self):
        return "\n".join(self.screen.display)

    def wait(self, needle, seconds=3):
        end = time.monotonic() + seconds
        while needle not in self.text() and time.monotonic() < end:
            self.drain(.03)
        assert needle in self.text(), (needle, self.text())

    def cell_at(self, needle):
        for y, row in enumerate(self.screen.display):
            if needle in row:
                return self.screen.buffer[y][row.index(needle)]
        raise AssertionError((needle, self.text()))

    def resize(self, cols, rows):
        self.cols, self.rows = cols, rows
        self.screen.resize(rows, cols)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        os.kill(self.pid, signal.SIGWINCH)
        self.drain(.12)

    def screenshot(self, directory, name):
        if directory is None:
            return
        from PIL import Image, ImageDraw, ImageFont
        directory.mkdir(parents=True, exist_ok=True)
        font_path = subprocess.check_output(["fc-match", "-f", "%{file}", "monospace"], text=True)
        font = ImageFont.truetype(font_path, 16)
        variants = {(False, False): font}
        for bold, italic, style in ((True, False, "Bold"), (False, True, "Oblique"),
                                    (True, True, "Bold Oblique")):
            family = "DejaVu Sans Mono" if italic else "monospace"
            variant = subprocess.check_output(
                ["fc-match", "-f", "%{file}", family + ":style=" + style], text=True)
            variants[bold, italic] = ImageFont.truetype(variant, 16)
        symbol_path = subprocess.check_output(["fc-match", "-f", "%{file}", ":charset=2460"], text=True)
        symbol_font = ImageFont.truetype(symbol_path, 14)
        cw, ch = 10, 22
        image = Image.new("RGB", (self.cols * cw, self.rows * ch), "#181818")
        draw = ImageDraw.Draw(image)
        for y in range(self.rows):
            for x in range(self.cols):
                cell = self.screen.buffer[y][x]
                fg = "d8d8d8" if cell.fg == "default" else cell.fg
                bg = "181818" if cell.bg == "default" else cell.bg
                if cell.reverse:
                    fg, bg = bg, fg
                draw.rectangle((x*cw, y*ch, (x+1)*cw, (y+1)*ch), fill="#" + bg)
                draw.text((x*cw, y*ch), cell.data,
                          font=symbol_font if cell.data and 0x2460 <= ord(cell.data[0]) <= 0x2473
                          else variants[cell.bold, cell.italics],
                          fill="#" + fg)
        image.save(directory / (name + ".png"))
        (directory / (name + ".txt")).write_text(
            "\n".join(line.rstrip() for line in self.text().split("\n")) + "\n"
        )

    def close(self):
        try:
            os.kill(self.pid, signal.SIGKILL)
            os.waitpid(self.pid, 0)
        except ProcessLookupError:
            pass
        os.close(self.fd)


def run(binary, cols, rows, colors, artifacts=None):
    pyte.graphics.FG_BG_256[:16] = colors
    with tempfile.TemporaryDirectory(prefix="vaayu-tour-ui-") as temp:
        root = Path(temp)
        original = write_fixture(root)
        session = Session(binary, root, cols, rows)
        try:
            session.key(":tour visual\r", .12)
            session.wait("Indicator 0/3")
            session.key("]h")
            session.wait("Indicator 1/3")
            assert "read_input();" in session.screen.display[session.screen.cursor.y]
            session.key("]h")
            session.wait("Indicator 2/3")
            assert "update_buffer();" in session.screen.display[session.screen.cursor.y]
            session.screenshot(artifacts, "highlight")
            # Returning from either preview must not traverse the source
            # jumplist, lose the tour's pane, or lose its selected indicator.
            for view in (",vf", ",vd"):
                session.key(view, .12)
                session.wait("Indicator 2/3")
                session.key("\x0f", .12)
                session.wait("② → Change")
                assert "[Tour Flow]" not in session.text()
                assert "[Tour Diff]" not in session.text()
                assert "update_buffer();" in session.screen.display[session.screen.cursor.y]
                if view == ",vf":
                    session.screenshot(artifacts, "source-return")
                session.key("K")
                session.key(view, .12)
                session.key("\x0f", .12)
                session.wait("Indicator 2/3")
                assert "[Tour Flow]" not in session.text()
                assert "[Tour Diff]" not in session.text()
                session.key("K")
                assert "update_buffer();" in session.screen.display[session.screen.cursor.y]
            session.key(",v]")
            session.wait("Indicator 3/3")
            session.wait("③ → Paint screen")
            session.key("]h")
            session.wait("Last highlight")
            session.key("[h,v[")
            session.wait("Indicator 1/3")
            session.key("[h")
            session.wait("First highlight")
            session.key(":tourhnext\r")
            session.wait("Indicator 2/3")
            session.key(",vi")
            session.key("[h")
            session.wait("Indicator 1/3")
            assert "① → Read input" not in session.text()
            session.key(",vi")
            session.key(":set globalstatusline\r")
            session.wait("Indicator 1/3")
            session.key("]h")
            session.wait("Indicator 2/3")
            session.key(":set noglobalstatusline\r")
            session.key("]t")
            session.wait("Plain step")
            assert "Indicator" not in session.text()
            session.key("[t")
            session.wait("Indicator 0/3")
            for theme in THEMES:
                session.key(":colorscheme " + theme + "\r")
                session.wait("① → Read input")
                primary = session.cell_at("request_redraw();")
                callout = session.cell_at("read_input();")
                assert callout.bg != primary.bg, (theme, "focused range must stand out", session.text())
                assert contrast(callout.fg, callout.bg) >= 4.5, (theme, callout, session.text())
                assert contrast(primary.fg, primary.bg) >= 4.5, (theme, primary, session.text())
                if theme == "default":
                    session.screenshot(artifacts, "source")
                session.key(",vi")
                assert "① → Read input" not in session.text(), session.text()
                assert session.cell_at("read_input();").bg == session.cell_at("request_redraw();").bg
                session.key(",vi")
                session.key(",vd", .12)
                # Short viewports scroll the preview to its changed lines.
                session.key("7G")
                session.wait("-old_value")
                session.wait("+new_value")
                deleted = session.cell_at("-old_value")
                added = session.cell_at("+new_value")
                assert deleted.bg != added.bg, (theme, deleted, added, session.text())
                assert contrast(deleted.fg, deleted.bg) >= 4.5, (theme, deleted, session.text())
                assert contrast(added.fg, added.bg) >= 4.5, (theme, added, session.text())
                if theme == "default":
                    session.screenshot(artifacts, "diff")
                session.key(",vS")
                session.wait("① → Read input")

            session.key(":colorscheme default\r")
            session.key("ggiPREFIX\r\x1b", .35)
            session.wait("① → Read input")
            assert session.cell_at("read_input();").bg != session.cell_at("intro").bg
            session.key("u", .35)
            session.wait("① → Read input")
            session.key("/input\r")
            searched = session.cell_at("input();")
            assert contrast(searched.fg, searched.bg) >= 4.5
            session.key(":nohlsearch\r")
            session.key("K")
            session.key(",vi")
            assert "① → Read input" not in session.text()
            session.key(",vi")
            session.wait("① → Read input")
            session.key("K")
            session.key(":tourcheck\r", .2)
            session.wait("Tour check passed")
            session.key(",vf")
            session.wait("Schedules painting")
            session.screenshot(artifacts, "flow")
            session.key(":tourfollow 1\r")
            session.wait("③ → Paint screen")
            session.wait("Indicator 3/3")
            session.key("]t")
            session.wait("Plain step")
            assert "→" not in session.text(), session.text()
            session.key("[t")
            session.key("K")
            session.key(",vd", .15)
            # View changes keep explanation focus. K returns to the preview.
            session.key("K")
            session.key("7G")
            session.wait("+new_value")
            session.resize(100, 30)
            session.wait("+new_value")
            session.resize(cols, rows)
            session.key(",vS")
            session.wait("read_input();")
            # End while the preview is active; source and layout must return.
            session.key(",vd", .12)
            session.key("]q")
            session.wait("Tour ended")
            session.wait("read_input();")
            assert "[Tour Diff]" not in session.text()
            # Actual Git changes include unsaved edits and never alter disk.
            session.key(":tour visual\r")
            session.key("]t]t")
            session.key("ggiUNSAVED_ADDITION\x1b", .12)
            session.key(",vd", .15)
            session.key("7G")
            session.wait("+UNSAVED_ADDITIONintro")
            assert (root / "demo.txt").read_text() == original
            session.key(",vS")
            session.key("u")
            session.key("]q")
        finally:
            session.close()
    print(f"Tour visuals PTY passed: {cols}x{rows}, ten themes, palette={'pale' if colors == PALE else 'stock'}", flush=True)


if __name__ == "__main__":
    binary = str(Path(sys.argv[1]).resolve())
    artifacts = Path(sys.argv[2]).resolve() if len(sys.argv) > 2 else None
    stock = list(pyte.graphics.FG_BG_256[:16])
    for cols, rows in ((40, 12), (80, 24), (140, 40)):
        for colors in (stock, PALE):
            output = artifacts if (cols, rows) == (140, 40) and colors == stock else None
            run(binary, cols, rows, colors, output)
