#!/usr/bin/env python3
"""shada also persists named registers across sessions: yank into register `a`
in one session, then paste from `a` in a fresh session on a different file.
Two real editor processes over a shared project (temp dir)."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
def session(root, target, keys, cols, rows):
    pid,fd=pty.fork()
    if pid==0:
        os.chdir(root)
        os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
        os.execv(binary,[binary,str(target)])
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
        txt="\n".join(screen.display)
        key(":q!\r")
        end=time.monotonic()+3
        ok=False
        while time.monotonic()<end:
            done,_=os.waitpid(pid,os.WNOHANG)
            if done:pid=None;ok=True;break
            drain(.05)
        return txt,ok
    finally:
        if pid:
            os.kill(pid,signal.SIGKILL);os.waitpid(pid,0)
        os.close(fd)
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-shadareg-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\nclipboard_unnamedplus=false\n')
        src=root/"src.txt"; src.write_text("SECRETLINE\n")
        target=root/"target.txt"; target.write_text("first\n")
        # Session 1: yank the line into register a, then quit (persists shada).
        _,ok1=session(root, src, ['"ayy'], cols, rows)
        assert ok1, "session 1 failed to exit"
        # Session 2: a different file; paste from register a — content survives.
        txt,ok2=session(root, target, ['"ap'], cols, rows)
        assert ok2, "session 2 failed to exit"
        assert "SECRETLINE" in txt, ("register a should persist across sessions\n"+txt)
    print(f"shada-registers PTY passed: {cols}x{rows}")
