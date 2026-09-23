#!/usr/bin/env python3
"""`:set stickyscroll` pins the enclosing function's declaration at the top of
the pane once it scrolls off (tree-sitter). Off by default. Driven via a PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(80,14),(120,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-sticky-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        # Long enough that the function scrolls off even on a 50-row terminal.
        body="".join(f"    stmt_{i:03}();\n" for i in range(1,120))
        (root/"code.rs").write_text("fn OUTERFUNC() {\n"+body+"}\n")
        f=root/"code.rs"
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
        def row0():return screen.display[0]
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.6)  # tree-sitter parse
            key("G",.3)   # scroll to the end; the fn line is now off-screen
            assert "OUTERFUNC" not in row0(), ("no sticky header by default\n"+"\n".join(screen.display))
            key(":set stickyscroll\r",.4)
            assert wait_for(lambda: "OUTERFUNC" in row0()), \
                ("the function signature should be pinned at the top\n"+"\n".join(screen.display))
            key(":set nosticky\r",.4)
            assert wait_for(lambda: "OUTERFUNC" not in row0()), \
                ("disabling should remove the sticky header\n"+"\n".join(screen.display))
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
    print(f"stickyscroll PTY passed: {cols}x{rows}")
