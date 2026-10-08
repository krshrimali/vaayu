#!/usr/bin/env python3
"""Ctrl-W h/j/k/l work without a delay, including when Ctrl stays held."""
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

binary = str(pathlib.Path(sys.argv[1]).resolve())
for cols, rows in [(80, 18), (120, 28)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-window-nav-") as tmp:
        root = pathlib.Path(tmp)
        (root / "config/vaayu").mkdir(parents=True)
        (root / "config/vaayu/config.toml").write_text(
            "jk_escape=false\nnumber=false\nwatch=false\nprogress=false\n"
            "notifications=false\nclipboard_unnamedplus=false\n"
        )
        (root / "source.txt").write_text("source row\n" * 60)
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(root)
            os.environ.update(
                TERM="xterm-256color",
                XDG_CONFIG_HOME=str(root / "config"),
                XDG_DATA_HOME=str(root / "data"),
            )
            os.execv(binary, [binary, "source.txt"])
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        screen = pyte.Screen(cols, rows)
        stream = pyte.Stream(screen)
        decoder = codecs.getincrementaldecoder("utf-8")("replace")

        def drain(seconds=.12):
            end = time.monotonic() + seconds
            while time.monotonic() < end:
                ready, _, _ = select.select(
                    [fd], [], [], min(.02, max(0, end - time.monotonic()))
                )
                if ready:
                    try:
                        data = os.read(fd, 65536)
                    except OSError:
                        return
                    if not data:
                        return
                    stream.feed(decoder.decode(data))

        def key(value, seconds=.12):
            os.write(fd, value.encode())
            drain(seconds)

        def quadrant():
            return (
                "right" if screen.cursor.x > divider_x else "left",
                "bottom" if screen.cursor.y >= bottom_y else "top",
            )

        try:
            drain(.4)
            # Four ordinary panes let every direction have a real neighbor.
            key("\x17v\x17s\x17h\x17s", .4)
            divider_x = screen.display[0].index("│")
            bottom_y = next(
                y + 1 for y, row in enumerate(screen.display) if row.startswith("─")
            )
            directions = {
                "h": ((divider_x + 5, 3), ("left", "top")),
                "l": ((5, 3), ("right", "top")),
                "j": ((5, 3), ("left", "bottom")),
                "k": ((5, bottom_y + 3), ("left", "top")),
            }
            for held_ctrl in [False, True]:
                for delay in [0, .005, .05, 1.0]:
                    for direction, ((x, y), expected) in directions.items():
                        key(f"\x1b[<0;{x+1};{y+1}M\x1b[<0;{x+1};{y+1}m")
                        suffix = chr(ord(direction) & 31) if held_ctrl else direction
                        if delay:
                            os.write(fd, b"\x17")
                            drain(delay)
                            key(suffix)
                        else:
                            # One write exercises keys already queued together.
                            key("\x17" + suffix)
                        assert quadrant() == expected, (
                            f"Ctrl-W {direction}, Ctrl held={held_ctrl}, "
                            f"gap={delay}s: expected {expected}, got {quadrant()}\n"
                            + "\n".join(screen.display)
                        )
            key(":qa!\r")
            end = time.monotonic() + 3
            while time.monotonic() < end:
                done, status = os.waitpid(pid, os.WNOHANG)
                if done:
                    assert os.waitstatus_to_exitcode(status) == 0
                    pid = None
                    break
                drain(.05)
            assert pid is None, "Editor failed to exit"
        finally:
            if pid:
                os.kill(pid, signal.SIGKILL)
                os.waitpid(pid, 0)
            os.close(fd)
    print(f"Window navigation PTY passed: {cols}x{rows} (32 sequences)")
