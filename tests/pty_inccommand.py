#!/usr/bin/env python3
"""inccommand: typing `:%s/pat/...` highlights the pattern's matches live (cell
backgrounds set), Esc clears the preview, and <CR> applies the substitution.
Driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-inccmd-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"f.txt"; f.write_text("foo bar\nbaz foo\nqux\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(f)])
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
        def text():return "\n".join(screen.display)
        def row_has_highlight(y):
            return any(screen.buffer[y][x].bg!='default' for x in range(cols))
        try:
            drain(.4)
            assert not row_has_highlight(0), ("no highlight before searching\n"+text())
            # Type the substitute WITHOUT Enter -> the matches highlight live.
            key(":%s/foo/",.4)
            assert row_has_highlight(0) or row_has_highlight(1), \
                ("inccommand should highlight the pattern's matches\n"+text())
            # Esc clears the preview.
            key("\x1b",.3)
            assert not row_has_highlight(0) and not row_has_highlight(1), \
                ("Esc should clear the inccommand preview\n"+text())
            # Submitting applies the substitution.
            key(":%s/foo/BAR/g\r",.3)
            assert "BAR bar" in text() and "baz BAR" in text(), \
                ("submit should apply the substitution\n"+text())
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
    print(f"inccommand PTY passed: {cols}x{rows}")
