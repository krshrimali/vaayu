#!/usr/bin/env python3
"""`:q` from within a Results-panel overlay (here `:undolist`) must dismiss the
panel and return to the buffer, NOT quit the whole editor. A later `:q` from the
plain buffer then quits normally. Driven through a real PTY."""
import os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time, fcntl, codecs
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(100,24),(140,40)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-qr-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"doc.txt"; f.write_text("alpha\nbeta\ngamma\n")
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
        def text():return "\n".join(screen.display)
        def alive():
            done,_=os.waitpid(pid,os.WNOHANG)
            return done==0
        try:
            drain(.5)
            key(":undolist\r",.5)   # open a Results-panel overlay
            assert "Undo history" in text(), ("undolist panel should open\n"+text())
            key(":q\r",.5)          # :q from the panel -> should NOT quit
            assert alive(), "editor must still be running after :q in a results panel"
            assert "Undo history" not in text(), ("panel dismissed, buffer shown\n"+text())
            assert "alpha" in text(), ("the buffer is shown again\n"+text())
            # A normal :q from the plain buffer now quits.
            key(":q\r",.5)
            end=time.monotonic()+3
            while time.monotonic()<end:
                done,status=os.waitpid(pid,os.WNOHANG)
                if done:
                    assert os.waitstatus_to_exitcode(status)==0;pid=None;break
                drain(.05)
            assert pid is None,"editor should exit on :q from the plain buffer"
        finally:
            if pid:
                os.kill(pid,signal.SIGKILL);os.waitpid(pid,0)
            os.close(fd)
    print(f"quit-from-results PTY passed: {cols}x{rows}")
