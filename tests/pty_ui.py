#!/usr/bin/env python3
"""Terminal UI regression: modes, selection, result footer, panes and resize."""
import codecs
import fcntl
import json
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


binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "target/release/vaayu").resolve())


def run(cols, rows):
    with tempfile.TemporaryDirectory(prefix="vaayu-ui-") as temp:
        root = pathlib.Path(temp)
        (root / "config/vaayu").mkdir(parents=True)
        (root / "config/vaayu/config.toml").write_text(
            "jk_escape=false\nclipboard_unnamedplus=false\nnumber=true\nwrap=true\n"
        )
        source = root / "review.md"
        source.write_text(
            "# UI review\n\n"
            "wide: 界 and combining: a\u0301 and emoji: 👩‍💻\n"
            "first review line\nsecond review line\n\n"
            "| left | right |\n| --- | ---: |\n| code `x` | 42 |\n"
        )
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color", XDG_CONFIG_HOME=str(root / "config"))
            os.execv(binary, [binary, str(source)])
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        screen = pyte.Screen(cols, rows)
        stream = pyte.Stream(screen)
        decoder = codecs.getincrementaldecoder("utf-8")("replace")
        capture = bytearray()

        def drain(seconds=0.2):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                ready, _, _ = select.select([fd], [], [], min(0.02, deadline - time.monotonic()))
                if ready:
                    try:
                        data = os.read(fd, 65536)
                    except OSError:
                        break
                    if not data:
                        break
                    capture.extend(data)
                    stream.feed(decoder.decode(data))

        def key(value, delay=0.2):
            os.write(fd, value.encode())
            drain(delay)

        def text():
            return "\n".join(screen.display)

        def status():
            return "\n".join(screen.display[-2:])

        try:
            drain(0.7)
            assert "NORMAL" in status(), text()
            assert "UI review" in text() and "界" in text() and "�" not in text(), text()
            assert screen.cursor.x < cols and screen.cursor.y < rows

            key("jjvll")
            assert "VISUAL" in status(), text()
            selected = [cell for row in screen.buffer.values() for cell in row.values() if cell.reverse]
            assert selected or b"\x1b[7m" in capture, "Visual selection emitted no reverse-video style"
            key("\x1b")
            key("i")
            assert "INSERT" in status(), text()
            key("\x1b")
            key(":set nowrap\r")
            assert "NORMAL" in status(), text()

            key("jVj,rc")
            assert "Private comment" in status(), text()
            key("iCheck UI alignment")
            key("\x1b")
            key("\x13", 0.3)
            key(":comments\r")
            assert "Private comments" in text() and "Check UI alignment" in text(), text()
            assert "R resolve" in status(), text()
            key("R")
            key("q")
            key(",rw")
            saved = json.loads((root / ".vaayu/comments.json").read_text())
            assert saved["notes"][0]["resolved"] is True
            key(":comments\r")
            key("\x11")
            assert "QUICKFIX / Private comments" in screen.display[0], text()
            assert "Private comments" in text() and "review.md:4" in text(), text()
            pathlib.Path(f"/tmp/vaayu-ui-quickfix-{cols}x{rows}.txt").write_text(text())

            key("q")
            key(":edit review.md\r")
            key(":vsplit\r")
            key(":split\r")
            body = text()
            assert "│" in body and "─" in body, body
            assert "NORMAL" in body and "BUFFER" in body, body
            key(":vpreview\r", 0.3)
            assert "PREVIEW" in text(), text()
            assert screen.cursor.x < cols and screen.cursor.y < rows

            new_cols, new_rows = cols + 7, rows + 3
            screen.resize(new_rows, new_cols)
            fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", new_rows, new_cols, 0, 0))
            os.kill(pid, signal.SIGWINCH)
            drain(0.3)
            assert screen.cursor.x < new_cols and screen.cursor.y < new_rows
            assert "PREVIEW" in text(), text()
            pathlib.Path(f"/tmp/vaayu-ui-splits-{cols}x{rows}.txt").write_text(text())

            key(":qa\r")
            deadline = time.monotonic() + 3
            while time.monotonic() < deadline:
                done, status = os.waitpid(pid, os.WNOHANG)
                if done:
                    assert os.waitstatus_to_exitcode(status) == 0
                    pid = None
                    break
                drain(0.05)
            assert pid is None, "Editor failed to exit"
        finally:
            if pid:
                os.kill(pid, signal.SIGKILL)
                os.waitpid(pid, 0)
            os.close(fd)
    print(f"UI PTY passed: {cols}x{rows}")


for dimensions in [(40, 12), (100, 24), (180, 50)]:
    run(*dimensions)
