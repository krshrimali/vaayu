#!/usr/bin/env python3
"""Tour startup and Vim scrolling must keep the source cursor above the panel."""
import codecs
import fcntl
import json
import os
import pathlib
import pty
import re
import select
import signal
import struct
import sys
import tempfile
import termios
import time

import pyte

binary = str(pathlib.Path(sys.argv[1]).resolve())


def run(cols, rows, description, winbar=False, wrap_source=False, split=False):
    with tempfile.TemporaryDirectory(prefix="vaayu-tour-scroll-") as tmp:
        root = pathlib.Path(tmp)
        config = root / "config" / "vaayu"
        config.mkdir(parents=True)
        (config / "config.toml").write_text(
            "jk_escape=false\nnumber=false\nprogress=false\nwatch=false\n"
            f"winbar={str(winbar).lower()}\n"
        )
        padding = " wrapped source" * (cols // 5) if wrap_source else ""
        (root / "source.txt").write_text(
            "".join(f"SOURCE_LINE_{i:03}{padding}\n" for i in range(1, 201))
        )
        (root / ".tours").mkdir()
        (root / ".tours" / "scroll.tour").write_text(json.dumps({
            "title": "Scroll reproduction",
            "steps": [
                {"file": "source.txt", "line": 40, "description": description},
                {"file": "source.txt", "line": 150, "description": "Next step"},
            ],
        }))
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(root)
            os.environ.update(
                TERM="xterm-256color",
                XDG_CONFIG_HOME=str(root / "config"),
                XDG_DATA_HOME=str(root / "data"),
                XDG_STATE_HOME=str(root / "state"),
                XDG_CACHE_HOME=str(root / "cache"),
            )
            os.execv(binary, [binary, "source.txt"])
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        screen = pyte.Screen(cols, rows)
        stream = pyte.Stream(screen)
        decoder = codecs.getincrementaldecoder("utf-8")("replace")

        def drain(seconds=0.04):
            end = time.monotonic() + seconds
            while time.monotonic() < end:
                ready, _, _ = select.select([fd], [], [], min(0.01, max(0, end - time.monotonic())))
                if ready:
                    try:
                        data = os.read(fd, 65536)
                    except OSError:
                        return
                    if not data:
                        return
                    stream.feed(decoder.decode(data))

        def key(value, seconds=0.04):
            os.write(fd, value.encode())
            drain(seconds)

        def check_cursor(action, expected_line=None, panel_visible=True):
            text = "\n".join(screen.display)
            ruler = re.findall(r"(\d+):(\d+)", screen.display[-2])
            assert ruler, f"{action}: missing ruler\n{text}"
            line = int(ruler[-1][0])
            if expected_line is not None:
                assert line == expected_line, f"{action}: wrong line {line}\n{text}"
            marker = f"SOURCE_LINE_{line:03}"
            assert marker in screen.display[screen.cursor.y], (
                f"{action}: cursor at line {line} is hidden or painted on the wrong row\n{text}"
            )
            panel_top = next((y for y, row in enumerate(screen.display) if "step " in row and "—" in row), None)
            assert (panel_top is not None) == panel_visible, f"{action}: panel visibility\n{text}"
            if panel_top is not None:
                assert screen.cursor.y < panel_top, f"{action}: cursor overlaps tour panel\n{text}"

        try:
            drain(0.35)
            if split:
                key(":split\r")
            key(":tour scroll\r", 0.2)
            check_cursor("tour start", 40)
            for i in range(1, rows + 4):
                key("j")
                check_cursor(f"j{i}", 40 + i)
            for i in range(1, rows + 4):
                key("k")
                check_cursor(f"k{i}", 40 + rows + 3 - i)

            for motion in ("\x04", "\x15", "\x06", "\x02", "zz", "zt", "zb", "G", "k"):
                key(motion)
                check_cursor(repr(motion))

            key("]t")
            check_cursor("next step", 150)
            key("[t")
            check_cursor("previous step", 40)
            key("K")
            assert "Tour explanation focused" in "\n".join(screen.display)
            assert not any("—  step " in row for row in screen.display)
            key("K")
            check_cursor("source focus", 40)

            for new_rows in (max(7, rows // 2), rows):
                fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", new_rows, cols, 0, 0))
                screen.resize(lines=new_rows, columns=cols)
                os.kill(pid, signal.SIGWINCH)
                drain(0.2)
                check_cursor(f"resize to {new_rows} rows", 40, panel_visible=not (split and new_rows < 10))
            key("]q")
            check_cursor("tour end", 40, panel_visible=False)
            for i in range(1, rows + 4):
                key("j")
                check_cursor(f"no tour j{i}", 40 + i, panel_visible=False)

            key(":qa!\r", 0.1)
            end = time.monotonic() + 3
            while time.monotonic() < end:
                done, status = os.waitpid(pid, os.WNOHANG)
                if done:
                    pid = None
                    assert os.waitstatus_to_exitcode(status) == 0
                    break
                drain(0.05)
            assert pid is None, "Editor did not exit"
        finally:
            if pid is not None:
                os.kill(pid, signal.SIGKILL)
                os.waitpid(pid, 0)
            os.close(fd)
    print(f"tour scroll PTY passed: {cols}x{rows}, winbar={winbar}, wrap_source={wrap_source}, split={split}", flush=True)


for cols, rows in [(80, 14), (120, 24), (180, 50)]:
    run(cols, rows, "Short description")
    run(cols, rows, "\n".join(f"Description row {i}" for i in range(6)), winbar=True)
    run(cols, rows, "A long wrapped tour description. " * 80, wrap_source=True)
run(80, 14, "A long wrapped tour description. " * 80, winbar=True, split=True)
