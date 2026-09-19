#!/usr/bin/env python3
"""`f` (filter) works on a live grep list, not just a frozen quickfix
snapshot -- narrowing the currently-fetched matches by a substring, and
staying active across a fresh batch of ripgrep results triggered by
editing the live query again. Driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-livegrepfilter-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        file=root/"f.txt"
        file.write_text("needle in a haystack\nanother needle here\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(file)])
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
            drain(.3)
            key(",gw",.5)  # live grep the word under the cursor ("needle")
            assert wait_for(lambda: "haystack" in text() and "another needle" in text()), \
                ("live grep should find both matches first\n"+text())
            key("\x1b",.2)  # leave query-editing, into browsing

            key("f",.2)
            key("haystack",.4)
            key("\x1b",.2)  # confirm the filter, stay in the results list
            assert wait_for(lambda: "haystack" in text() and "another needle" not in text()), \
                ("filtering a live list should narrow its current matches\n"+text())

            # Editing the live query again (still matching both lines)
            # forces a fresh ripgrep batch; the filter must still apply
            # to it afterwards, not get silently dropped.
            key("i",.2)
            key("\x7f",.3)  # backspace the query down to empty...
            key("\x7f",.3)
            key("\x7f",.3)
            key("\x7f",.3)
            key("\x7f",.3)
            key("\x7f",.3)
            key("needle",.4)  # ...and retype it, triggering a new search
            key("\x1b",.2)  # back to browsing
            assert wait_for(lambda: "haystack" in text() and "another needle" not in text()), \
                ("the filter should still narrow the fresh batch of live results\n"+text())

            key("\x1b",.2)
            key(":qa\r")
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
    print(f"Live grep filter PTY passed: {cols}x{rows}")
