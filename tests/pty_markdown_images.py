#!/usr/bin/env python3
"""Markdown images through real PTYs: paths, HTTP, graphics negotiation,
fallback, edits, zoom, split previews, image refresh and terminal cleanup."""
import base64
import codecs
import fcntl
import http.server
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
import threading
import time
import zlib

import pyte

binary = str(pathlib.Path(sys.argv[1]).resolve())


def png(width, height, color):
    def chunk(kind, data):
        return (struct.pack(">I", len(data)) + kind + data +
                struct.pack(">I", zlib.crc32(kind + data)))
    pixels = (b"\0" + bytes(color) * width) * height
    return (b"\x89PNG\r\n\x1a\n" +
            chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)) +
            chunk(b"IDAT", zlib.compress(pixels)) + chunk(b"IEND", b""))


def run(mode="auto", supported=True, no_color=False):
    class Handler(http.server.BaseHTTPRequestHandler):
        requests = 0

        def do_GET(self):
            Handler.requests += 1
            data = b'<svg xmlns="http://www.w3.org/2000/svg" width="120" height="40"><rect width="120" height="40" fill="blue"/></svg>'
            self.send_response(200)
            self.send_header("Content-Type", "image/svg+xml")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def log_message(self, *args):
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix="vaayu-images-pty-") as tmp:
            root = pathlib.Path(tmp)
            config = root / "config/vaayu"
            config.mkdir(parents=True)
            (config / "config.toml").write_text(
                f'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
                f'watch=false\nprogress=false\nmermaid_preview="{mode}"\n'
            )
            (root / "assets").mkdir()
            image_path = root / "assets/local photo.png"
            image_path.write_bytes(png(160, 80, (220, 20, 30, 255)))
            (root / "docs").mkdir()
            source = root / "docs/images.md"
            source.write_text(
                "# Images\n\nBefore image\n\n![Local](../assets/local%20photo.png)\n\n"
                "After image\n\n![Repeated][photo]\n\n"
                "[photo]: ../assets/local%20photo.png\n\n"
                f"![Remote](http://127.0.0.1:{server.server_port}/image.svg)\n\n"
                "![Missing](missing.png)\n\nTail of document\n"
            )
            pid, fd = pty.fork()
            if pid == 0:
                os.chdir(root)  # Deliberately different from the Markdown directory.
                os.environ.update(TERM="xterm-256color", XDG_CONFIG_HOME=str(root / "config"),
                                  XDG_DATA_HOME=str(root / "data"), XDG_STATE_HOME=str(root / "state"))
                for name in ("TMUX", "WEZTERM_PANE", "TERM_PROGRAM", "NO_COLOR"):
                    os.environ.pop(name, None)
                if no_color:
                    os.environ["NO_COLOR"] = "1"
                os.execv(binary, [binary, str(source)])
            cols, rows = 100, 34
            fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, cols * 8, rows * 16))
            screen = pyte.Screen(cols, rows)
            stream = pyte.Stream(screen)
            decoder = codecs.getincrementaldecoder("utf-8")("replace")
            capture = bytearray()
            pending = bytearray()
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
                    query = re.search(rb"\x1b_Ga=q,i=(\d+)[^;]*;AAAA\x1b\\", capture)
                    if query and not answered:
                        os.write(fd, b"\x1b_Gi=" + query[1] + (b";OK" if supported else b";ENOTSUP") + b"\x1b\\")
                        answered = True
                    pending.extend(data)
                    # pyte renders text only; remove potentially fragmented APCs.
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

            def key(value, seconds=.15):
                os.write(fd, value.encode())
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

            def uploads():
                result = []
                image_id = None
                chunks = []
                for header, payload in re.findall(rb"\x1b_G([^;]*);(.*?)\x1b\\", capture, re.S):
                    if header.startswith(b"a=t,"):
                        image_id = int(re.search(rb"i=(\d+)", header)[1])
                        chunks = [payload]
                    elif header.startswith(b"m=") and image_id is not None:
                        chunks.append(payload)
                    else:
                        continue
                    if b"m=0" in header:
                        data = base64.b64decode(b"".join(chunks), validate=True)
                        assert data[:8] == b"\x89PNG\r\n\x1a\n"
                        result.append((image_id, struct.unpack(">II", data[16:24])))
                        image_id = None
                return result

            try:
                drain(.3)
                key(",mp")
                if (mode == "auto" and supported) or mode == "kitty":
                    wait_for(lambda: len(uploads()) == 2 and "loading" not in text())
                    initial = uploads()
                    assert {size for _, size in initial} == {(160, 80), (120, 40)}, initial
                    assert Handler.requests == 1, "HTTP image was not cached"
                    assert "Before image" in text() and "After image" in text(), text()
                    assert "No such file" in text(), "missing image needs a readable error\n" + text()
                    key("+-0jk")
                    assert uploads() == initial, "scroll or zoom re-uploaded cached pixels"
                    image_path.write_bytes(png(176, 80, (20, 220, 30, 255)))
                    wait_for(lambda: any(size == (176, 80) for _, size in uploads()))
                    assert Handler.requests == 1, "local refresh refetched HTTP image"
                    image_ids = {image_id for image_id, _ in uploads()}
                    key("q")
                    for image_id in image_ids:
                        assert f"a=d,d=I,i={image_id},".encode() in capture, "preview did not free image"
                    # Full and split previews share cached artifacts; closing and
                    # reopening the pane also exercises its saved layout cache.
                    key(",ms")
                    wait_for(lambda: capture.count(b"a=t,") >= 5)
                    key("\x17wq")
                    key(",ms")
                    wait_for(lambda: capture.count(b"a=t,") >= 7)
                    key("\x17wq")
                    assert Handler.requests == 1
                    key(":%s/local%20photo.png/absent.png/g\r")
                    key(",mp")
                    wait_for(lambda: "absent.png" in text() and "loading" not in text())
                    assert "No such file" in text(), text()
                else:
                    wait_for(lambda: "[Image: Local]" in text())
                    drain(.3)
                    assert b"a=t," not in capture and Handler.requests == 0
                    assert "../assets/local%20photo.png" in text() and "[Image: Missing]" in text()
                if mode == "auto":
                    assert answered, "an image-only document must probe graphics support"
                else:
                    assert b"a=q," not in capture
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
    finally:
        server.shutdown()
        server.server_close()
    print(f"Markdown images PTY passed: {mode}, supported={supported}, NO_COLOR={no_color}")


run()
run(no_color=True)
run(mode="kitty")
run(supported=False)
run(mode="unicode")
run(mode="off")
