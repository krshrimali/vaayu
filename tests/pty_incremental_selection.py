#!/usr/bin/env python3
"""Tree-sitter incremental selection: ,= expands the selection to the enclosing
syntax node (growing the reverse-video selection), ,- shrinks it back. Driven
through a real PTY on a Rust file."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-incsel-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\nleader=","\n')
        f=root/"code.rs"; f.write_text("fn main() {\n    let x = foo(1, 2);\n}\n")
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
        def reversed_cells():
            return sum(1 for y in range(rows-1) for x in range(cols) if screen.buffer[y][x].reverse)
        try:
            drain(.5)  # let tree-sitter parse
            key("2G",.2); key("f1",.2)   # cursor on the `1` inside foo(1, 2)
            key(",=",.3)                 # expand: select the token
            a=reversed_cells()
            assert a>=1, ("first expand should select a node\n"+"\n".join(screen.display))
            key(",=",.3)                 # expand to a bigger node
            b=reversed_cells()
            assert b>a, (f"expand should grow selection {a}->{b}\n"+"\n".join(screen.display))
            key(",=",.3)
            c=reversed_cells()
            assert c>=b, (f"expand should keep growing {b}->{c}")
            key(",-",.3)                 # shrink back
            d=reversed_cells()
            assert d<c, (f"shrink should reduce selection {c}->{d}")
            key("\x1b",.2)               # leave visual
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
    print(f"incremental selection PTY passed: {cols}x{rows}")
