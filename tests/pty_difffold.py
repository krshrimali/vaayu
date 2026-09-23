#!/usr/bin/env python3
"""`:difffold [N]` collapses unchanged regions in diff mode, keeping N context
lines around each change. The changed line + context stay visible; far-away
unchanged lines fold into a `⋯ N lines` summary row. PTY, 2 geometries."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(120,24),(160,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-df-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        la=[f"line {i}" for i in range(30)]
        lb=la.copy(); lb[15]="CHANGED"
        a=root/"a.txt"; a.write_text("\n".join(la)+"\n")
        b=root/"b.txt"; b.write_text("\n".join(lb)+"\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(a)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.35):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                r,_,_=select.select([fd],[],[],.05)
                if r:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.35):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        try:
            drain(.5)
            key(":diffthis\r",.4)
            key(":vsplit b.txt\r",.6)
            key(":diffthis\r",.4)
            key(":difffold 3\r",.6)
            t=text()
            assert "CHANGED" in t, ("the change is shown\n"+t)
            assert "line 15" in t, ("the unchanged side of the change is shown\n"+t)
            assert "line 12" in t and "line 18" in t, ("context lines stay visible\n"+t)
            assert "⋯" in t, ("a fold summary row appears\n"+t)
            assert "line 5" not in t, ("a far leading line is folded away\n"+t)
            assert "line 25" not in t, ("a far trailing line is folded away\n"+t)
            key(":diffoff\r",.3)
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
    print(f"difffold PTY passed: {cols}x{rows}")
