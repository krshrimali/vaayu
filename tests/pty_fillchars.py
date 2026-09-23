#!/usr/bin/env python3
"""Configurable `fillchars`: a custom `eob` glyph replaces the `~` end-of-buffer
marker, and a custom `vert` glyph replaces the `│` split separator. PTY-driven."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-fc-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nnumber=false\nfillchars="eob:%,vert:!"\n')
        f=root/"a.txt"; f.write_text("only line\n")
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
        def key(s,seconds=.4):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        def col0():return "".join(screen.buffer[y][0].data for y in range(rows))
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.5)
            # Custom eob '%' appears below the single content line; no '~'.
            assert wait_for(lambda: "%" in col0()), ("custom eob glyph should show\n"+text())
            assert "~" not in col0(), ("default ~ eob must be replaced\n"+text())
            # Open a vertical split; the custom separator '!' appears.
            key(":vsplit\r",.6)
            assert wait_for(lambda: any("!" in row for row in screen.display)), \
                ("custom vert separator should show\n"+text())
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
    print(f"fillchars PTY passed: {cols}x{rows}")
