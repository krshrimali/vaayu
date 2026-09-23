#!/usr/bin/env python3
"""`:set globalstatusline`: with a horizontal split, the per-pane status lines
collapse into a single shared status line just above the message line. PTY-driven."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-gsl-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"a.txt"; f.write_text("\n".join(f"line{i}" for i in range(40))+"\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(f)])
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
        def key(s,seconds=.4):os.write(fd,s.encode());drain(seconds)
        def status_rows():
            # Status lines carry a mode label ("NORMAL"/"BUFFER"); this ignores
            # the horizontal split divider row, which is also background-colored.
            out=[]
            for y in range(rows-1):
                line="".join(screen.buffer[y][x].data for x in range(cols))
                if "NORMAL" in line or "BUFFER" in line:
                    out.append(y)
            return out
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.5)
            key(":split\r",.5)  # two stacked panes -> two per-pane status lines
            assert wait_for(lambda: len(status_rows())==2), \
                (f"expected 2 per-pane status lines, got {status_rows()}")
            key(":set globalstatusline\r",.5)
            # Now a single shared status line, at rows-2.
            assert wait_for(lambda: status_rows()==[rows-2]), \
                (f"expected one global status at row {rows-2}, got {status_rows()}")
            assert "NORMAL" in "".join(screen.buffer[rows-2][x].data for x in range(cols)), \
                "global status shows the mode"
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
    print(f"global statusline PTY passed: {cols}x{rows}")
