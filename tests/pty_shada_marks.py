#!/usr/bin/env python3
"""shada persists marks across sessions: set mark `a` on line 3 in one session
(quitting from line 10), then in a fresh session jump to it with `` `a ``.
Because the per-file cursor also restores line 10, the jump to line 3 proves
the mark itself survived. Two real editor processes over a shared project."""
import codecs, fcntl, os, pathlib, pty, re, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
def cursor_line(screen):
    for row in reversed(screen.display):
        m=re.findall(r"(\d+):(\d+)", row)
        if m:
            return int(m[-1][0])
    return None
def session(root, keys, cols, rows):
    f=root/"f.txt"
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
    try:
        drain(.5)
        for k in keys:
            key(k,.3)
        line=cursor_line(screen)
        key(":q\r")
        end=time.monotonic()+3
        ok=False
        while time.monotonic()<end:
            done,_=os.waitpid(pid,os.WNOHANG)
            if done:pid=None;ok=True;break
            drain(.05)
        return line,ok
    finally:
        if pid:
            os.kill(pid,signal.SIGKILL);os.waitpid(pid,0)
        os.close(fd)
for cols,rows in [(60,24),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-shadamark-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        (root/"f.txt").write_text("".join(f"line {i}\n" for i in range(1,11)))
        # Session 1: mark line 3 as `a`, move to line 10, quit.
        _,ok1=session(root, ["3G","ma","G"], cols, rows)
        assert ok1, "session 1 failed to exit"
        # Session 2: opens at restored line 10; `a jumps to the mark (line 3).
        line2,ok2=session(root, ["`a"], cols, rows)
        assert ok2, "session 2 failed to exit"
        assert line2==3, ("`a should jump to the persisted mark on line 3, got %s"%line2)
    print(f"shada-marks PTY passed: {cols}x{rows}")
