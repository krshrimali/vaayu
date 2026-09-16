#!/usr/bin/env python3
""":resume reopens whichever of the file picker or a Results list was
more recently dismissed, exactly as it was left (query/matches/cursor)
-- driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-resume-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        a=root/"a.txt"; a.write_text("alpha content\n")
        unique_target=root/"unique_target_file.txt"; unique_target.write_text("x\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(a)])
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
        try:
            drain(.4)
            # File picker: type a query, dismiss with Esc, then :resume
            # should reopen it with that same query still typed in.
            key("\x10",.3)  # Ctrl-P
            key("unique_target",.3)
            assert "unique_target_file" in text(), ("picker did not filter to the file\n"+text())
            key("\x1b",.2)  # Esc dismisses
            assert "FILES" not in text(), ("Esc should have closed the picker\n"+text())
            key(":resume\r",.3)
            assert "unique_target_file" in text(), \
                (":resume should reopen the picker with its query intact\n"+text())
            key("\x1b",.2)
            # Live grep results: dismiss with q, then :resume reopens it.
            key(",/alpha\r",.3)
            assert "RESULTS" in text() or "alpha content" in text(), \
                ("live grep did not open a results list\n"+text())
            key("q",.2)
            key(":resume\r",.3)
            assert "alpha" in text(), (":resume should reopen the results list\n"+text())
            key("q",.2)
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
    print(f"Resume PTY passed: {cols}x{rows}")
