#!/usr/bin/env python3
"""The fuzzy file picker's status line reports "<shown>/<matched> files" --
proving the bounded top-k matcher's truncation is real and visible, not
just an internal implementation detail: with more matching candidates
than the picker keeps, the shown count must be capped while the matched
count keeps counting all of them. Driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
TAKE=500
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-pickerrank-") as tmp:
        base=pathlib.Path(tmp)
        # Config lives *outside* the project root (unlike most other PTY
        # tests, which don't care) since this test asserts exact scanned
        # file counts and the picker doesn't exclude an in-project config
        # directory from its inventory.
        root=base/"project"; root.mkdir()
        (base/"config/vaayu").mkdir(parents=True)
        (base/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        # More than TAKE files match "hay", plus a handful that don't --
        # matched should count only the ones that actually match.
        total_matching=TAKE+80
        for i in range(total_matching):
            (root/f"hay_{i:04d}.txt").write_text("x")
        for i in range(5):
            (root/f"needle_{i}.txt").write_text("x")
        total_files=total_matching+5
        entry=root/"hay_0000.txt"
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(base/"config"))
            os.execv(binary,[binary,str(entry)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.25):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                ready,_,_=select.select([fd],[],[],min(.02,max(0,end-time.monotonic())))
                if ready:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.2):os.write(fd,s.encode());drain(seconds)
        def text():
            return "\n".join(screen.display)
        def wait_for(pred, timeout=6.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():
                    return True
                drain(.1)
            return False
        try:
            drain(.4)
            key("\x10",.3)  # Ctrl-P: fuzzy file picker
            assert wait_for(lambda: f"{TAKE}/{total_files} files" in text()), \
                ("initial (empty-query) picker should report every scanned "
                 f"file as matched, capped at {TAKE} shown\n"+text())
            key("hay",.3)
            assert wait_for(lambda: f"{TAKE}/{total_matching} files" in text()), \
                ("querying 'hay' should show all matching candidates in "
                 f"'matched' but cap the shown/kept count at {TAKE}\n"+text())
            key("\x7f\x7f\x7f",.2)  # backspace out "hay"
            key("needle",.3)
            assert wait_for(lambda: "5/5 files" in text()), \
                ("querying 'needle' (fewer matches than the take limit) "
                 "should show the same count on both sides\n"+text())
            key("\x1b",.2)  # Esc: close the picker
            key(":qa!\r")
            end=time.monotonic()+3
            while time.monotonic()<end:
                done,status=os.waitpid(pid,os.WNOHANG)
                if done:
                    assert os.waitstatus_to_exitcode(status)==0;pid=None;break
                drain(.05)
            assert pid is None,"Editor failed to exit"
        finally:
            if pid:
                os.kill(pid,signal.SIGKILL);os.waitpid(pid,0)
            os.close(fd)
    print(f"Picker ranking instrumentation PTY passed: {cols}x{rows}")
