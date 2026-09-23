#!/usr/bin/env python3
""":make runs a command and routes `file:line:col: message` output into the
quickfix list; Enter on an entry jumps to that location. Driven through a real
PTY with a fake compiler (printf)."""
import codecs, fcntl, os, pathlib, pty, re, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
def cursor_line(screen):
    for row in reversed(screen.display):
        m=re.findall(r"(\d+):(\d+)", row)
        if m:
            return int(m[-1][0])
    return None
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-make-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        (root/"code.rs").write_text("l1\nl2\nl3\nl4\nl5\n")
        f=root/"main.txt"; f.write_text("open me\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(f)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.25):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                ready,_,_=select.select([fd],[],[],min(.02,max(0,end-time.monotonic())))
                if ready:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.25):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        def wait_for(pred,timeout=5.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.1)
            return False
        try:
            drain(.4)
            # Fake compiler prints one error location pointing at code.rs line 4.
            key(r":make printf 'code.rs:4:2: error: boom\n'"+"\r",.4)
            assert wait_for(lambda: "code.rs:4:2" in text()), \
                (":make output should populate the quickfix list\n"+text())
            key("\r",.4)   # Enter jumps to the location
            assert wait_for(lambda: cursor_line(screen)==4 and "code.rs" in text()), \
                ("Enter on the quickfix entry should jump to code.rs:4\n"+text())
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
    print(f"make PTY passed: {cols}x{rows}")
