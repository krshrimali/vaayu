#!/usr/bin/env python3
"""Highlight contrast across themes, including remapped pale terminal colors."""
import codecs
import fcntl
import os
import pathlib
import pty
import select
import signal
import struct
import sys
import tempfile
import termios
import time

import pyte
import pyte.graphics

binary = str(pathlib.Path(sys.argv[1]).resolve())
themes = ("default", "mono", "warm", "cool", "gruvbox", "gruvbox-light",
          "flexoki", "flexoki-light", "tokyonight", "tokyonight-day")
stock = pyte.graphics.FG_BG_256[:16]
# Like the reported terminal: blue/cyan backgrounds and ANSI white are pale.
pale = ["2c3439", "bf616a", "a3be8c", "ebcb8b", "81b6af", "b48ead", "86be98", "d6c5aa",
        "607078", "e08787", "b4ce9f", "f0d49a", "9bc9c1", "cba2c5", "a1d2b2", "d8c9ad"]


def contrast(fg, bg):
    def luminance(color):
        assert len(color) == 6, ("highlight needs explicit colors", color)
        values = [int(color[i:i + 2], 16) / 255 for i in (0, 2, 4)]
        linear = [v / 12.92 if v <= .04045 else ((v + .055) / 1.055) ** 2.4 for v in values]
        return sum(v * w for v, w in zip(linear, (.2126, .7152, .0722)))
    a, b = luminance(fg), luminance(bg)
    return (max(a, b) + .05) / (min(a, b) + .05)


for cols, rows in ((40, 12), (100, 24), (180, 50)):
    for palette, colors in (("stock", stock), ("pale", pale)):
        pyte.graphics.FG_BG_256[:16] = colors
        with tempfile.TemporaryDirectory(prefix="vaayu-contrast-") as tmp:
            root = pathlib.Path(tmp)
            cfg = root / "config/vaayu"
            cfg.mkdir(parents=True)
            (cfg / "config.toml").write_text(
                "jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\nwatch=false\nprogress=false\n"
            )
            source = root / "main.txt"
            source.write_text("alpha target omega\nanother target line\n")
            other = root / "main-extra.txt"
            other.write_text("other buffer\n")
            pid, fd = pty.fork()
            if pid == 0:
                os.chdir(root)
                os.environ.pop("NO_COLOR", None)
                os.environ.update(TERM="xterm-256color", XDG_CONFIG_HOME=str(root / "config"),
                                  XDG_DATA_HOME=str(root / "data"), XDG_STATE_HOME=str(root / "state"))
                os.execv(binary, [binary, str(source)])
            fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
            screen = pyte.Screen(cols, rows)
            stream = pyte.Stream(screen)
            decoder = codecs.getincrementaldecoder("utf-8")("replace")

            def drain(seconds=.08):
                end = time.monotonic() + seconds
                while time.monotonic() < end:
                    ready, _, _ = select.select([fd], [], [], min(.02, max(0, end - time.monotonic())))
                    if ready:
                        try:
                            data = os.read(fd, 65536)
                        except OSError:
                            break
                        if not data:
                            break
                        stream.feed(decoder.decode(data))

            def key(value, seconds=.08):
                os.write(fd, value.encode())
                drain(seconds)

            def check_row(y, label):
                cells = [screen.buffer[y][x] for x in range(cols)]
                assert any(c.data.strip() for c in cells), (theme, label, "empty row", screen.display)
                for cell in cells:
                    assert contrast(cell.fg, cell.bg) >= 4.5, (theme, palette, label, cell, screen.display)

            try:
                drain(.3)
                key(":e " + str(other) + "\r")
                key(":e " + str(source) + "\r")
                for theme in themes:
                    key(":colorscheme " + theme + "\r")
                    check_row(rows - 2, "statusline")
                    key("/target\r")
                    matches = [(y, line.find("target")) for y, line in enumerate(screen.display[:2])
                               if "target" in line]
                    assert matches, (theme, "search text missing", screen.display)
                    y, x = matches[0]
                    match = screen.buffer[y][x]
                    assert contrast(match.fg, match.bg) >= 4.5, (theme, palette, "search", match, screen.display)
                    key(",h")
                    key(",ff", .2)
                    key("main")
                    assert screen.display[0].startswith("> main"), (theme, "picker missing", screen.display)
                    check_row(0, "picker prompt")
                    check_row(1, "picker selection")
                    check_row(rows - 1, "picker footer")
                    key("\x1b[B")
                    check_row(2, "moved picker selection")
                    key("\x1b")  # query normal
                    key("\x1b")  # close picker
                    key(":ls\r")
                    assert "results" in screen.display[0], (theme, "results missing", screen.display)
                    check_row(0, "results header")
                    check_row(2, "results selection")
                    check_row(rows - 2, "results footer")
                    key("\x1b")
                key(":qa!\r", .05)
                end = time.monotonic() + 3
                while time.monotonic() < end:
                    done, status = os.waitpid(pid, os.WNOHANG)
                    if done:
                        assert os.waitstatus_to_exitcode(status) == 0
                        pid = None
                        break
                    drain(.05)
                assert pid is None, "editor did not exit"
            finally:
                if pid:
                    os.kill(pid, signal.SIGKILL)
                    os.waitpid(pid, 0)
                os.close(fd)
        print(f"Highlight contrast PTY passed: {cols}x{rows}, {palette}, {len(themes)} themes", flush=True)
pyte.graphics.FG_BG_256[:16] = stock
