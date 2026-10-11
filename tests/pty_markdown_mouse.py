#!/usr/bin/env python3
"""Exercise Markdown wheel scrolling with real SGR mouse input and rendered
terminal cells: full/split previews, focus, bounds, resize and image cropping."""
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


def run(cols, rows, graphical=False):
    with tempfile.TemporaryDirectory(prefix="vaayu-markdown-mouse-") as tmp:
        root = pathlib.Path(tmp)
        (root / "config/vaayu").mkdir(parents=True)
        (root / "config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            'watch=false\nprogress=false\nnotifications=false\n'
            f'mermaid_preview="{"kitty" if graphical else "off"}"\n'
        )
        source = root / "preview.md"
        source.write_text("# TOP\n\n![Figure](figure.svg)\n\n" +
                          "\n".join(f"ROW{i:03}  " for i in range(1, 121)) + "\n\nBOTTOM\n")
        (root / "figure.svg").write_text(
            '<svg xmlns="http://www.w3.org/2000/svg" width="640" height="320">'
            '<rect width="640" height="320" fill="blue"/></svg>'
        )
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color", XDG_CONFIG_HOME=str(root / "config"),
                              XDG_DATA_HOME=str(root / "data"), XDG_STATE_HOME=str(root / "state"))
            os.execv(binary, [binary, str(source)])
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, cols * 8, rows * 16))
        screen = pyte.Screen(cols, rows)
        stream = pyte.Stream(screen)
        decoder = codecs.getincrementaldecoder("utf-8")("replace")
        capture = bytearray()
        pending = bytearray()

        def drain(seconds=.08):
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
                pending.extend(data)
                # Strip graphics APCs, including ones split across reads.
                while pending:
                    start = pending.find(b"\x1b_G")
                    if start < 0:
                        keep = 1 if pending.endswith(b"\x1b") else 2 if pending.endswith(b"\x1b_") else 0
                        emit = len(pending) - keep
                        stream.feed(decoder.decode(bytes(pending[:emit])))
                        del pending[:emit]
                        break
                    if start:
                        stream.feed(decoder.decode(bytes(pending[:start])))
                        del pending[:start]
                    finish = pending.find(b"\x1b\\", 3)
                    if finish < 0:
                        break
                    del pending[:finish + 2]

        def key(value, seconds=.08):
            os.write(fd, value.encode())
            drain(seconds)

        def wheel(down, x, y, count=1):
            key(f"\x1b[<{65 if down else 64};{x + 1};{y + 1}M" * count, .15)

        def text():
            return "\n".join(screen.display)

        def wait_for(predicate):
            end = time.monotonic() + 8
            while time.monotonic() < end:
                if predicate():
                    return
                drain(.05)
            raise AssertionError(text() + "\nlast bytes: " + repr(capture[-1200:]))

        try:
            drain(.3)
            source_cursor = (screen.cursor.x, screen.cursor.y)
            source_top = screen.display[0]
            key(",mp")
            wait_for(lambda: "PREVIEW" in text() and "loading" not in text())
            if graphical:
                wait_for(lambda: b"a=p,U=1" in capture)
            assert "TOP" in text(), text()
            before = text()
            uploads = capture.count(b"a=t,")
            wheel(True, 5, 3)
            assert "TOP" not in text(), "full preview did not scroll\n" + text()
            if graphical:
                assert capture.count(b"a=t,") == uploads, "cropping an image re-uploaded it"
            wheel(False, 5, 3)
            assert text() == before, "wheel up did not restore full preview\n" + text()
            wheel(True, 5, rows - 1)
            assert text() == before, "wheel over command row changed the preview"
            wheel(True, 5, 3, 100)
            wait_for(lambda: "BOTTOM" in text())
            bottom = text()
            wheel(True, 5, 3, 10)
            assert text() == bottom, "wheel scrolled past the final preview row"
            wheel(False, 5, 3, 100)
            wait_for(lambda: text() == before)
            key("q")
            assert screen.display[0] == source_top, "full preview moved source viewport"
            assert (screen.cursor.x, screen.cursor.y) == source_cursor, "full preview moved source cursor"

            key(",ms")
            wait_for(lambda: "PREVIEW" in text() and "loading" not in text())
            border = screen.display[0].index("│")

            def left():
                return "\n".join(line[:border] for line in screen.display[:-2])

            def right():
                return "\n".join(line[border + 1:] for line in screen.display[:-2])

            preview_x = border + 3
            editor_before = left()
            preview_before = right()
            assert screen.cursor.x < border
            wheel(True, preview_x, 3)
            assert right() != preview_before and left() == editor_before, "inactive preview routing\n" + text()
            assert screen.cursor.x < border, "wheel changed focus to the preview"
            wheel(False, preview_x, 3)
            assert right() == preview_before

            key("\x17l")
            key("j")
            assert right() != preview_before and left() == editor_before, "preview focus failed\n" + text()
            key("k")
            wheel(True, preview_x, 3)
            assert right() != preview_before and left() == editor_before, "active preview routing\n" + text()
            wheel(False, preview_x, 3)
            assert right() == preview_before

            # Hovering the source keeps preview focus and scrolls only the source.
            wheel(True, 5, 3)
            assert left() != editor_before and right() == preview_before
            scrolled_source = left()
            key("j")
            assert right() != preview_before and left() == scrolled_source, "hovered source stole preview focus"
            key("k")
            wheel(False, 5, 3, 100)
            wait_for(lambda: left() == editor_before)

            wheel(True, preview_x, 3, 100)
            wait_for(lambda: "BOTTOM" in right())
            bottom = right()
            wheel(True, preview_x, 3, 10)
            assert right() == bottom, "split preview scrolled past the document"
            cols, rows = cols + 20, rows + 5
            screen.resize(rows, cols)
            fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, cols * 8, rows * 16))
            os.kill(pid, signal.SIGWINCH)
            drain(.2)
            border = screen.display[0].index("│")
            preview_x = border + 3
            wheel(True, preview_x, 3, 100)
            wait_for(lambda: "BOTTOM" in right())
            wheel(False, preview_x, 3, 100)
            wait_for(lambda: "TOP" in right())
            key("q")
            assert screen.display[0].startswith(source_top.rstrip()), "split preview moved source viewport"
            assert (screen.cursor.x, screen.cursor.y) == source_cursor, "split preview moved source cursor"
            key(":qa\r")
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
    print(f"Markdown mouse PTY passed: {cols - 20}×{rows - 5}, graphics={graphical}")


for size in [(40, 12), (100, 24), (180, 50)]:
    run(*size)
run(40, 12, graphical=True)
run(100, 24, graphical=True)
