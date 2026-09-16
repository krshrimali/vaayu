#!/usr/bin/env python3
"""Per-keystroke terminal regression for typing and completion overlays."""
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


binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "target/release/vaayu").resolve())


with tempfile.TemporaryDirectory(prefix="vaayu-typing-ui-") as temp:
    root = pathlib.Path(temp)
    (root / "config/vaayu").mkdir(parents=True)
    (root / "config/vaayu/config.toml").write_text(
        "jk_escape=false\nclipboard_unnamedplus=false\nnumber=true\nwrap=false\n"
        "[lsp.rust]\nenabled=false\n"
    )
    source = root / "typing.rs"
    source.write_text(
        "const completion_candidate: usize = 1;\n"
        "fn another_candidate() {}\n"
        "\n"
        "underlay_one\n"
        "underlay_two\n"
        "underlay_three\n"
        + "".join(f"scroll_line_{i:03}\n" for i in range(120))
    )
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(root)
        os.environ.update(TERM="xterm-256color", XDG_CONFIG_HOME=str(root / "config"))
        os.execv(binary, [binary, str(source)])
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 16, 80, 0, 0))
    screen = pyte.Screen(80, 16)
    stream = pyte.Stream(screen)
    decoder = codecs.getincrementaldecoder("utf-8")("replace")

    def drain(seconds=0.12):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            ready, _, _ = select.select([fd], [], [], min(0.01, deadline - time.monotonic()))
            if ready:
                try:
                    data = os.read(fd, 65536)
                except OSError:
                    break
                if not data:
                    break
                stream.feed(decoder.decode(data))

    def key(value, delay=0.08):
        os.write(fd, value.encode())
        drain(delay)

    def text():
        return "\n".join(screen.display)

    def capture(name):
        pathlib.Path(f"/tmp/vaayu-typing-{name}.txt").write_text(text())

    try:
        drain(0.6)
        key("jj$i")
        for index, char in enumerate("comp"):
            key(char)
            capture(f"{index + 1}-{char}")
            assert screen.display[2].count("comp"[: index + 1]) == 1, text()
        assert "completion_candidate" in text(), text()

        # Closing the popup must repaint every row that it covered.
        key(" ")
        capture("popup-closed")
        assert "underlay_one" in screen.display[3], text()
        assert "underlay_two" in screen.display[4], text()
        assert "underlay_three" in screen.display[5], text()
        assert "buf completion_candidate" not in text(), text()

        # Deletion must not leave terminal cells from the longer text behind.
        key("ghost")
        for index in range(5):
            key("\x7f")
            capture(f"backspace-{index + 1}")
        assert "ghost" not in screen.display[2], text()
        assert screen.display[2].count("comp ") == 1, text()

        key("\x1b")
        capture("normal")
        assert "INSERT" not in text(), text()

        # Exercise the terminal scroll-region fast path. Every visible source
        # line must remain consecutive; stale terminal rows show up here as a
        # duplicate, gap or reversal.
        key("30j")
        key("\x04")
        capture("half-page-scroll")
        numbers = [
            int(match.group(1))
            for line in screen.display[:-2]
            if (match := re.search(r"scroll_line_(\d+)", line))
        ]
        assert len(numbers) >= 8, text()
        assert numbers == list(range(numbers[0], numbers[0] + len(numbers))), text()

        key(":qa!\r")
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            done, status = os.waitpid(pid, os.WNOHANG)
            if done:
                assert os.waitstatus_to_exitcode(status) == 0
                pid = None
                break
            drain(0.03)
        assert pid is None, "Editor failed to exit"
    finally:
        if pid:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
        os.close(fd)

print("Typing UI PTY passed")
