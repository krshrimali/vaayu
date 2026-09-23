#!/usr/bin/env python3
"""`gv` reselects the last visual selection. Select "HEL" charwise, leave
visual, move the cursor away, then `gv` + `d` must delete exactly that span —
proving the prior selection was restored. Driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-gv-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"f.txt"; f.write_text("HELLOWORLD\nsecondline\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(f)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.3):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                ready,_,_=select.select([fd],[],[],min(.02,max(0,end-time.monotonic())))
                if ready:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.3):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        try:
            drain(.4)
            assert "HELLOWORLD" in text(), ("initial content\n"+text())
            key("gg0",.2)             # top-left
            key("vll",.2)             # charwise select "HEL" (cols 0..=2)
            assert "VISUAL" in text(), ("should be in VISUAL mode\n"+text())
            key("\x1b",.2)            # leave visual
            key("j0",.2)              # move the cursor to another line
            key("gv",.3)              # reselect the prior span
            assert "VISUAL" in text(), ("gv should re-enter VISUAL mode\n"+text())
            key("d",.3)               # delete the reselected span
            assert "LOWORLD" in text(), ("gv+d should delete exactly 'HEL'\n"+text())
            assert "HELLOWORLD" not in text(), ("the original word is gone\n"+text())
            assert "secondline" in text(), ("the other line is untouched\n"+text())
            key(":q!\r")
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
    print(f"gv PTY passed: {cols}x{rows}")
