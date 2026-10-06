#!/usr/bin/env python3
"""Global leader mappings remain reachable from modal picker and Results UIs."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary = str(pathlib.Path(sys.argv[1]).resolve())
for cols, rows in [(80, 18), (120, 28)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-leader-modal-") as tmp:
        root = pathlib.Path(tmp)
        cfg = root / "config"
        (cfg / "vaayu").mkdir(parents=True)
        (cfg / "vaayu/config.toml").write_text(
            'jk_escape=false\nnumber=false\ntree_icons=false\n'
        )
        f = root / "fixture.txt"
        f.write_text("alpha\nbeta\n")
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color", XDG_CONFIG_HOME=str(cfg))
            os.execv(binary, [binary, str(f)])
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        screen = pyte.Screen(cols, rows)
        stream = pyte.Stream(screen)
        decoder = codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.2):
            end = time.monotonic() + seconds
            while time.monotonic() < end:
                ready, _, _ = select.select([fd], [], [], .02)
                if ready:
                    try:
                        data = os.read(fd, 65536)
                    except OSError:
                        break
                    if not data:
                        break
                    stream.feed(decoder.decode(data))
        def key(seq, delay=.2):
            os.write(fd, seq.encode())
            drain(delay)
        def text():
            return "\n".join(screen.display)
        def wait_for(predicate, timeout=4):
            end = time.monotonic() + timeout
            while time.monotonic() < end:
                if predicate():
                    return True
                drain(.05)
            return False
        try:
            drain(.3)
            key(",ff", .4)       # picker query starts in Insert sub-mode
            key(",", .1)         # the leader is literal text while query insertion is active
            assert "> ," in text(), "leader character should be editable picker text\n" + text()
            key("\x15", .1)      # clear the temporary query
            key("\x1b", .2)     # normal query sub-mode
            key(",e", .4)       # global leader action from the picker
            assert wait_for(lambda: "fixture.txt" in text() and "│" in text()), \
                "leader mapping was swallowed by the picker\n" + text()
            key(",e", .3)       # close the tree and return to the buffer
            key("yy", .2)        # create a register entry for :reg
            key(":reg\r", .4)   # open a Results list
            assert "Registers" in text(), "register Results list did not open\n" + text()
            key(",e", .4)       # global leader action from Results
            assert wait_for(lambda: "│" in text()), \
                "leader mapping was swallowed by Results\n" + text()
            key(":qa!\r", .3)
            end = time.monotonic() + 3
            while time.monotonic() < end:
                done, status = os.waitpid(pid, os.WNOHANG)
                if done:
                    assert os.waitstatus_to_exitcode(status) == 0
                    pid = None
                    break
                drain(.05)
            assert pid is None, "Editor failed to quit"
        finally:
            if pid is not None:
                os.kill(pid, signal.SIGKILL)
                os.waitpid(pid, 0)
            os.close(fd)
    print(f"Global leader modal PTY passed: {cols}x{rows}")
