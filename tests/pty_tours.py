#!/usr/bin/env python3
"""Code tours: `:tour` starts a `.tours/*.tour`, jumping to the first step and
showing its description; `:tournext` advances to the next step in another file.
Driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, re, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
def cursor_line(screen):
    for row in reversed(screen.display):
        m=re.findall(r"(\d+):(\d+)", row)
        if m:
            return int(m[-1][0])
    return None
for cols,rows in [(80,14),(120,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-tours-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        (root/".tours").mkdir()
        (root/"alpha.txt").write_text("a1\na2\na3\na4\na5\n")
        (root/"beta.txt").write_text("b1\nb2\nb3\n")
        (root/".tours/intro.tour").write_text(
            '{"title":"Intro","steps":['
            '{"file":"alpha.txt","line":4,"description":"FIRSTSTEPDESC"},'
            '{"file":"beta.txt","line":2,"description":"SECONDSTEPDESC"}]}')
        f=root/"start.txt"; f.write_text("start\n")
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
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.4)
            key(":tour intro\r",.4)
            assert wait_for(lambda: "alpha.txt" in text() and "FIRSTSTEPDESC" in text()), \
                (":tour should open the first step\n"+text())
            assert cursor_line(screen)==4, ("cursor at alpha.txt line 4, got %s"%cursor_line(screen))
            key(":tournext\r",.4)
            assert wait_for(lambda: "beta.txt" in text() and "SECONDSTEPDESC" in text()), \
                (":tournext should advance to the second step\n"+text())
            assert cursor_line(screen)==2, ("cursor at beta.txt line 2, got %s"%cursor_line(screen))
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
    print(f"tours PTY passed: {cols}x{rows}")
