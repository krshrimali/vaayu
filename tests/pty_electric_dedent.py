#!/usr/bin/env python3
"""smartindent electric dedent: a closing brace typed alone on an indented line
snaps back to its block's indent. Type `}` on a 4-space blank line -> "}"."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(80,24),(120,40)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-ed-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"code.rs"; f.write_text("fn f() {\n    body();\n    \n")
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
                r,_,_=select.select([fd],[],[],.05)
                if r:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.3):os.write(fd,s.encode());drain(seconds)
        # A fixed-width gutter precedes content even with number off; locate it
        # from where line 0's text starts, then read each line's content area.
        def content(y):
            g=screen.display[0].index("fn")
            return screen.display[y][g:].rstrip()
        def wait_for(pred,timeout=4.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.1)
            return False
        try:
            drain(.5)
            assert content(0)=="fn f() {", ("file loaded\n"+"\n".join(screen.display[:4]))
            assert content(2)=="", ("line 2 starts blank/indented\n"+repr(screen.display[2]))
            key("3G",.2)   # to the blank indented line
            key("A",.2)    # append at its end (col 4)
            key("}",.3)    # electric dedent snaps it to the block's indent
            assert wait_for(lambda: content(2)=="}"), \
                ("the closing brace should dedent to column 0\n"+repr(screen.display[2]))
            key("\x1b",.2)
            key(":wq\r",.4)
            end=time.monotonic()+3
            while time.monotonic()<end:
                done,status=os.waitpid(pid,os.WNOHANG)
                if done:
                    assert os.waitstatus_to_exitcode(status)==0;pid=None;break
                drain(.05)
            assert pid is None,"Editor failed to exit"
            assert f.read_text()=="fn f() {\n    body();\n}\n", \
                ("on-disk result: %r"%f.read_text())
        finally:
            if pid:
                os.kill(pid,signal.SIGKILL);os.waitpid(pid,0)
            os.close(fd)
    print(f"electric dedent PTY passed: {cols}x{rows}")
