#!/usr/bin/env python3
"""Vim-parity keys and Ex commands, driven through a real PTY: case
operators, Insert-mode Ctrl keys, Replace mode (and its statusline label),
H/L/M, :sp / Ctrl-W q, = reindent, :r !cmd, :w !cmd, ZZ."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte

binary = str(pathlib.Path(sys.argv[1]).resolve())
for cols, rows in [(60, 14), (100, 24), (180, 50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-parity-") as tmp:
        root = pathlib.Path(tmp)
        (root / "config/vaayu").mkdir(parents=True)
        (root / "config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\nscrolloff=0\n'
        )
        file = root / "f.txt"
        file.write_text("hello world\nfoo\nfn a() {\nx();\n}\n")
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(root)
            os.environ.update(
                TERM="xterm-256color",
                XDG_CONFIG_HOME=str(root / "config"),
                XDG_STATE_HOME=str(root / "state"),
                HOME=str(root),
            )
            os.execv(binary, [binary, str(file)])
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        screen = pyte.Screen(cols, rows)
        stream = pyte.Stream(screen)
        decoder = codecs.getincrementaldecoder("utf-8")("replace")

        def drain(seconds=.2):
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

        def key(s, seconds=.15):
            os.write(fd, s.encode())
            drain(seconds)

        def saved():
            key(":w\r", .3)
            return file.read_text()

        def text():
            return "\n".join(screen.display)

        try:
            drain(.4)
            # Case operator and Insert-mode Ctrl-W.
            key("gUiw")
            key("jA\x17bar\x1b")
            assert saved() == "HELLO world\nbar\nfn a() {\nx();\n}\n", saved()
            # Replace mode shows its own label and Backspace restores.
            key("ggR")
            assert "REPLACE" in text(), text()
            key("ab\x7f\x1b")
            assert saved().startswith("aELLO world\n"), saved()
            # = reindents a block.
            key("3G=2j")
            assert "\n    x();\n" in saved(), saved()
            # :sp opens a split, Ctrl-W q closes it again.
            key(":sp\r")
            assert sum("f.txt" in line for line in screen.display) >= 2, text()
            key("\x17q")
            # :r !cmd reads command output below the cursor.
            key("gg:r !echo READ\r", .4)
            assert saved().split("\n")[1] == "READ", saved()
            # :w !cmd pipes to the command and writes nothing new.
            key(":w !wc -l\r", .5)
            assert not (root / "!wc").exists()
            key("\x1b")
            # L / H move to the bottom / top of the screen.
            key("ggL")
            key("ix\x1b")
            assert saved().split("\n")[-2].startswith("x"), saved()
            key("uH")
            # ZZ writes and quits.
            key("A!\x1b")
            key("ZZ", .3)
            end = time.monotonic() + 3
            while time.monotonic() < end:
                done, status = os.waitpid(pid, os.WNOHANG)
                if done:
                    assert os.waitstatus_to_exitcode(status) == 0
                    pid = None
                    break
                drain(.05)
            assert pid is None, "Editor failed to exit on ZZ"
            assert file.read_text().startswith("aELLO world!\n"), file.read_text()
        finally:
            if pid:
                os.kill(pid, signal.SIGKILL)
                os.waitpid(pid, 0)
            os.close(fd)
    print(f"Vim parity PTY passed: {cols}x{rows}")
