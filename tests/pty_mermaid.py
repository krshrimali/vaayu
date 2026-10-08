#!/usr/bin/env python3
"""Mermaid previews through real PTYs: asynchronous text, edits, bounds,
resize, graphics negotiation, PNG uploads and image lifetime."""
import base64
import codecs
import fcntl
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
source_text = ("# Diagrams\n\n```mermaid\nflowchart LR\nA[Start] --> B[Process] --> C[Destination]\n```\n\n"
               "Tail of document\n\n```mermaid\nflowchart LR\nA[Start] --> B[Process] --> C[Destination]\n```\n")


def run(cols, rows, mode, terminal="supported", no_color=False):
    with tempfile.TemporaryDirectory(prefix="vaayu-mermaid-pty-") as tmp:
        root = pathlib.Path(tmp)
        config = root / "config/vaayu"
        config.mkdir(parents=True)
        (config / "config.toml").write_text(
            f'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            f'watch=false\nprogress=false\nmermaid_preview="{mode}"\n'
        )
        source = root / "diagram.md"
        source.write_text(source_text)
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color", XDG_CONFIG_HOME=str(root / "config"),
                              XDG_DATA_HOME=str(root / "data"), XDG_STATE_HOME=str(root / "state"))
            os.environ.pop("TMUX", None)
            os.environ.pop("WEZTERM_PANE", None)
            os.environ.pop("TERM_PROGRAM", None)
            if terminal == "wezterm":
                os.environ["TERM_PROGRAM"] = "WezTerm"
            if no_color:
                os.environ["NO_COLOR"] = "1"
            else:
                os.environ.pop("NO_COLOR", None)
            os.execv(binary, [binary, str(source)])
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, cols * 8, rows * 16))
        screen = pyte.Screen(cols, rows)
        stream = pyte.Stream(screen)
        decoder = codecs.getincrementaldecoder("utf-8")("replace")
        capture = bytearray()
        text_pending = bytearray()
        answered = False

        def drain(seconds=.15):
            nonlocal answered
            end = time.monotonic() + seconds
            while time.monotonic() < end:
                ready, _, _ = select.select([fd], [], [], min(.02, max(0, end - time.monotonic())))
                if not ready:
                    continue
                try:
                    data = os.read(fd, 65536)
                except OSError:
                    break
                if not data:
                    break
                capture.extend(data)
                if mode == "auto" and not answered:
                    query = re.search(rb"\x1b_Ga=q,i=(\d+)[^;]*;AAAA\x1b\\", capture)
                    if query:
                        os.write(fd, b"\x1b_Gi=" + query[1] + b";OK\x1b\\")
                        answered = True
                # pyte is a text emulator: remove complete APC messages before
                # feeding it, retaining fragmented image transfers between reads.
                text_pending.extend(data)
                while text_pending:
                    start = text_pending.find(b"\x1b_G")
                    if start < 0:
                        keep = 1 if text_pending.endswith(b"\x1b") else 2 if text_pending.endswith(b"\x1b_") else 0
                        emit = len(text_pending) - keep
                        stream.feed(decoder.decode(bytes(text_pending[:emit])))
                        del text_pending[:emit]
                        break
                    if start:
                        stream.feed(decoder.decode(bytes(text_pending[:start])))
                        del text_pending[:start]
                    finish = text_pending.find(b"\x1b\\", 3)
                    if finish < 0:
                        break
                    del text_pending[:finish + 2]

        def key(text, seconds=.15):
            os.write(fd, text.encode())
            drain(seconds)

        def text():
            return "\n".join(screen.display)

        def wait_for(predicate, timeout=8):
            end = time.monotonic() + timeout
            while time.monotonic() < end:
                if predicate():
                    return
                drain(.05)
            raise AssertionError(text() + "\nlast bytes: " + repr(capture[-1500:]))

        try:
            drain(.3)
            if mode == "unicode":
                # Reopen before the debounce expires: the saved pane layout
                # must request its cancelled pending artifact again.
                key(",ms", .02)
                key("\x17wq", .02)
                key(",ms", .02)
                wait_for(lambda: "PREVIEW" in text() and "Start" in text() and "rendering" not in text())
                key("\x17wq")
            key(",mp")
            if mode == "unicode" or terminal == "wezterm":
                wait_for(lambda: "Start" in text() and "rendering" not in text() and "flowchart LR" not in text())
                assert "─" in text(), text()
                if terminal == "wezterm":
                    assert b"a=q," not in capture and b"a=t," not in capture
                key("l" * 40)
                assert "Destination" in text(), text()
                key("0g")
                key("G" + "j" * 50)
                assert "Tail of document" in text(), text()
                key("gq")
                key(":vpreview\r")
                wait_for(lambda: "PREVIEW" in text() and "rendering" not in text())
                key(":%s/Start/Updated/g\r", .02)
                key(":%s/Updated/Latest/g\r", .02)
                wait_for(lambda: "Latest" in text() and "rendering" not in text())
                assert "Updated" not in text(), text()
                key("\x17w")
                key("G" + "j" * 60)
                assert "Tail of document" in text(), text()
                key("g")
                cols2 = cols + 17
                screen.resize(rows, cols2)
                fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols2, cols2 * 8, rows * 16))
                os.kill(pid, signal.SIGWINCH)
                drain(.3)
                assert "PREVIEW" in text(), text()
                key("q")
                key(":%s/flowchart LR/not-a-diagram/g\r")
                key(",mp")
                wait_for(lambda: "Mermaid" in text() and "rendering" not in text())
                assert "not-a-diagram" in text(), text()
                key("q")
            elif mode == "auto":
                wait_for(lambda: b"a=p,U=1" in capture and "rendering" not in text())
                assert answered, "editor did not negotiate graphics support"
                messages = re.findall(rb"\x1b_G([^;]*);(.*?)\x1b\\", capture, re.S)
                pieces = []
                image_id = None
                for header, payload in messages:
                    if header.startswith(b"a=t,"):
                        image_id = re.search(rb"i=(\d+)", header)[1]
                        pieces = [payload]
                    elif header.startswith(b"m=") and pieces:
                        pieces.append(payload)
                    else:
                        continue
                    if b"m=0" in header:
                        png = base64.b64decode(b"".join(pieces), validate=True)
                        assert png[:8] == b"\x89PNG\r\n\x1a\n"
                        assert struct.unpack(">II", png[16:24])[0] > 100
                        break
                assert image_id is not None, "missing PNG image upload"
                encoded_id = int(image_id)
                rgb = ((encoded_id >> 16) & 255, (encoded_id >> 8) & 255, encoded_id & 255)
                assert f"\x1b[38;2;{rgb[0]};{rgb[1]};{rgb[2]}m".encode() in capture, "image cell IDs missing"
                uploads = capture.count(b"a=t,")
                key("+-0jk")
                assert capture.count(b"a=t,") == uploads, "scroll/zoom re-uploaded cached PNG"
                key("q")
                assert b"a=d,d=I,i=" + image_id in capture, "closing preview did not free image"
                assert "Gi=" not in text(), "terminal capability reply leaked into editor input"
            else:
                assert "flowchart LR" in text(), text()
                assert b"a=q," not in capture and b"a=t," not in capture
                key("q")
            key(":qa!\r")
            deadline = time.monotonic() + 4
            while time.monotonic() < deadline:
                done, status = os.waitpid(pid, os.WNOHANG)
                if done:
                    assert os.waitstatus_to_exitcode(status) == 0
                    pid = None
                    break
                drain(.05)
            assert pid is None, "editor failed to exit"
        finally:
            if pid is not None:
                os.kill(pid, signal.SIGKILL)
                os.waitpid(pid, 0)
            os.close(fd)
    print(f"Mermaid PTY passed: {cols}×{rows}, {mode}, {terminal}, NO_COLOR={no_color}")


for dimensions in [(40, 16), (100, 28)]:
    run(*dimensions, "unicode")
run(100, 28, "auto")
run(100, 28, "auto", no_color=True)
run(80, 24, "auto", terminal="wezterm")
run(60, 16, "off")
