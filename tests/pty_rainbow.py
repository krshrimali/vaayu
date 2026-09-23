#!/usr/bin/env python3
"""`:set rainbow` colorizes brackets by nesting depth: nested `(` get different
foreground colors, and a matching pair shares its color. Off by default.
Driven through a real PTY, asserting on cell foregrounds."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-rainbow-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"f.txt"; f.write_text("((x))\n")   # .txt: no syntax coloring to interfere
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
        def paren_fgs():
            return [screen.buffer[0][x].fg for x in range(cols) if screen.buffer[0][x].data=="("]
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.4)
            assert "((x))" in text(), ("initial content\n"+text())
            # Off by default: both '(' share the default foreground.
            fgs=paren_fgs()
            assert len(fgs)==2 and fgs[0]==fgs[1]=="default", ("rainbow off by default\n"+text())
            key(":set rainbow\r",.3)
            # On: the two nested '(' get distinct, non-default foregrounds.
            assert wait_for(lambda: len(paren_fgs())==2 and len(set(paren_fgs()))==2), \
                ("nested brackets should get distinct colors\n"+text())
            fgs=paren_fgs()
            assert all(c!="default" for c in fgs), ("both brackets colored\n"+text())
            key(":set norainbow\r",.3)
            assert wait_for(lambda: paren_fgs()==["default","default"]), \
                ("norainbow restores default coloring\n"+text())
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
    print(f"rainbow PTY passed: {cols}x{rows}")
