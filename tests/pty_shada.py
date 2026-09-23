#!/usr/bin/env python3
"""shada: the cursor position is remembered across sessions. Open a file, jump
to line 10, quit; relaunch the same file in the same project and the cursor is
restored there. Two real editor processes over a shared temp dir."""
import codecs, fcntl, os, pathlib, pty, re, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
def cursor_line(screen,cols):
    for row in reversed(screen.display):
        m=re.findall(r"(\d+):(\d+)", row)
        if m:
            return int(m[-1][0])
    return None
def run(root, keys_before_quit, cols, rows):
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
        for k in keys_before_quit:
            key(k,.3)
        line=cursor_line(screen,cols)
        key(":q\r")
        end=time.monotonic()+3
        ok=False
        while time.monotonic()<end:
            done,_=os.waitpid(pid,os.WNOHANG)
            if done:
                pid=None;ok=True;break
            drain(.05)
        return line,ok
    finally:
        if pid:
            os.kill(pid,signal.SIGKILL);os.waitpid(pid,0)
        os.close(fd)
for cols,rows in [(60,24),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-shada-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        (root/"f.txt").write_text("".join(f"line {i}\n" for i in range(1,21)))
        # Session 1: jump to line 10, then quit (saves shada).
        line1,ok1=run(root,["10G"],cols,rows)
        assert line1==10, ("session 1 should be on line 10, got %s"%line1)
        assert ok1, "session 1 failed to exit"
        # Session 2: reopen; cursor restored to line 10 (no motion sent).
        line2,ok2=run(root,[],cols,rows)
        assert line2==10, ("session 2 should restore line 10, got %s"%line2)
        assert ok2, "session 2 failed to exit"
    print(f"shada PTY passed: {cols}x{rows}")
