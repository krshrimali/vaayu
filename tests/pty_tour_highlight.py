#!/usr/bin/env python3
"""Code-tour step highlighting + range anchoring:
  - a step with `pattern` re-anchors to the line containing it,
  - `endLine` highlights the whole range with a row background,
  - `]t` advances the tour and moves the highlight,
  - the step panel shows the progress dots + hints.
"""
import codecs, fcntl, os, pathlib, pty, select, struct, sys, termios, time
import pyte

binary = str(pathlib.Path(sys.argv[1]).resolve())


def run(cols, rows):
    import tempfile
    with tempfile.TemporaryDirectory(prefix="vaayu-tourhl-") as tmp:
        root = pathlib.Path(tmp)
        (root / "config/vaayu").mkdir(parents=True)
        (root / "config/vaayu/config.toml").write_text("jk_escape=false\nnumber=false\n")
        (root / ".tours").mkdir()
        (root / "f.rs").write_text("l0\nfn target() {\n  body\n}\nl4\nl5\nl6\n")
        (root / ".tours/i.tour").write_text(
            '{"title":"Demo","steps":['
            '{"file":"f.rs","line":1,"endLine":4,"pattern":"fn target","description":"the **fn**"},'
            '{"file":"f.rs","line":6,"description":"later"}]}'
        )
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color", XDG_CONFIG_HOME=str(root / "config"))
            os.execv(binary, [binary, "f.rs"])
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        screen = pyte.Screen(cols, rows)
        stream = pyte.Stream(screen)
        dec = codecs.getincrementaldecoder("utf-8")("replace")

        def drain(seconds=0.3):
            end = time.monotonic() + seconds
            while time.monotonic() < end:
                r, _, _ = select.select([fd], [], [], 0.02)
                if r:
                    try:
                        data = os.read(fd, 65536)
                    except OSError:
                        break
                    if not data:
                        break
                    stream.feed(dec.decode(data))

        def hl(text):
            # Is the buffer row whose (stripped) content == `text` highlighted?
            for y in range(rows):
                if screen.display[y].strip() == text:
                    return screen.buffer[y][0].bg != "default"
            return None

        try:
            drain(1.0)
            os.write(fd, b":tour i\r")
            drain(0.6)
            disp = "\n".join(screen.display)
            # Step 1 anchored to "fn target" with endLine -> highlights the whole
            # `fn target() { body }` block, not the line above it. (Scroll-safe:
            # look up rows by content, not fixed screen index.)
            assert hl("fn target() {") and hl("body") and hl("}"), (
                "the step's whole range should be highlighted\n" + disp)
            assert hl("l0") is False, ("the line above the range must not be highlighted\n" + disp)
            # Panel: markdown stripped, progress dots present.
            assert "the fn" in disp, ("markdown-lite description expected\n" + disp)
            assert "step 1/2" in disp, ("panel progress expected\n" + disp)
            # `]t` advances: highlight moves to the second step's line.
            os.write(fd, b"]t")
            drain(0.5)
            disp = "\n".join(screen.display)
            assert "step 2/2" in disp, ("]t should advance the tour\n" + disp)
            assert hl("l5") is True, ("the highlight should move to step 2's line\n" + disp)
            assert hl("fn target() {") in (False, None), (
                "the previous step's highlight should clear\n" + disp)
            os.write(fd, b":qa!\r")
            end = time.monotonic() + 3
            while time.monotonic() < end:
                done, _ = os.waitpid(pid, os.WNOHANG)
                if done:
                    pid = None
                    break
                drain(0.05)
        finally:
            if pid is not None:
                try:
                    os.kill(pid, 9)
                    os.waitpid(pid, 0)
                except OSError:
                    pass
    print(f"tour highlight PTY passed: {cols}x{rows}")


for cols, rows in [(80, 16), (140, 40)]:
    run(cols, rows)
