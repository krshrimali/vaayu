#!/usr/bin/env python3
"""`:set cursorline` tints the background of the active window's cursor line,
and the tint follows the cursor (and is absent by default). Driven through a
real PTY, asserting on cell backgrounds."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-cul-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"f.txt"; f.write_text("line zero\nline one\nline two\nline three\n")
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
        # A row is "tinted" if most of its cells carry a non-default background
        # (the cursorline fills the whole width, unlike a short highlight).
        def tinted(y):
            return sum(1 for x in range(cols) if screen.buffer[y][x].bg!="default") > cols//2
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.4)
            assert "line zero" in text(), ("initial content\n"+text())
            # Off by default: the cursor row (0) is not fully tinted.
            assert not tinted(0), ("cursorline should be off by default\n"+text())
            key(":set cursorline\r",.3)
            assert wait_for(lambda: tinted(0)), ("row 0 should tint when enabled\n"+text())
            assert not tinted(2), ("a non-cursor row stays untinted\n"+text())
            key("jj",.3)                 # move cursor to row 2
            assert wait_for(lambda: tinted(2)), ("the tint follows the cursor to row 2\n"+text())
            assert not tinted(0), ("row 0 no longer tinted after moving\n"+text())
            key(":set nocursorline\r",.3)
            assert wait_for(lambda: not tinted(2)), ("disabling removes the tint\n"+text())
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
    print(f"cursorline PTY passed: {cols}x{rows}")
