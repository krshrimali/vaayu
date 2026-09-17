#!/usr/bin/env python3
""",b/:buffer lists open buffers most-recently-activated first, not
insertion order -- driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-buffermru-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        a=root/"a.txt"; a.write_text("a\n")
        b=root/"b.txt"; b.write_text("b\n")
        c=root/"c.txt"; c.write_text("c\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(a)])
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
        def key(s,seconds=.2):os.write(fd,s.encode());drain(seconds)
        def text():
            return "\n".join(screen.display)
        def row_order():
            # Return the buffer list rows in on-screen top-to-bottom order.
            return [l for l in screen.display if ".txt" in l]
        try:
            drain(.4)
            key(f":e {b}\r",.3)
            key(f":e {c}\r",.3)   # order opened: a, b, c (c current)
            key(f":e {a}\r",.3)   # switch back to a -- activation order now: c, a (a most recent)
            key(":buffer\r",.3)
            buf_rows = row_order()
            assert len(buf_rows) >= 3, ("expected 3 buffer rows\n"+text())
            ia, ic, ib = (next(i for i,l in enumerate(buf_rows) if n in l) for n in ("a.txt","c.txt","b.txt"))
            assert ia < ic < ib, \
                ("buffer list should be MRU order (a, c, b), not insertion order (a, b, c)\n"+text())
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
    print(f"Buffer MRU PTY passed: {cols}x{rows}")
