#!/usr/bin/env python3
""":colder/:cnewer navigate between past quickfix lists (Ctrl-Q exports
the current Results/live-grep list to quickfix, appending to history)
-- driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-qfhist-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        a=root/"a.txt"; a.write_text("alpha_marker line\n")
        b=root/"b.txt"; b.write_text("beta_marker line\n")
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
            key(",/alpha_marker\r",.4)
            assert "alpha_marker" in text(), ("first grep should show alpha_marker\n"+text())
            key("\x11",.3)  # Ctrl-Q: export to quickfix
            key("q",.2)     # dismiss back to normal
            key(",/beta_marker\r",.4)
            assert "beta_marker" in text(), ("second grep should show beta_marker\n"+text())
            key("\x11",.3)  # Ctrl-Q: export to quickfix (history now [alpha, beta])
            key("q",.2)
            key(":colder\r",.3)
            assert "alpha_marker" in text() and "beta_marker" not in text(), \
                (":colder should switch to the alpha quickfix list\n"+text())
            key(":colder\r",.3)
            assert "alpha_marker" in text(), \
                (":colder at the oldest list must stay put, not crash\n"+text())
            key(":cnewer\r",.3)
            assert "beta_marker" in text() and "alpha_marker" not in text(), \
                (":cnewer should switch back to the beta quickfix list\n"+text())
            key("q",.2)
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
    print(f"Quickfix history PTY passed: {cols}x{rows}")
