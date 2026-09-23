#!/usr/bin/env python3
"""Diff mode: `:diffthis` on two buffers highlights their differing lines with
a background tint; equal lines stay plain; `:diffoff` clears. Driven via a PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(80,14),(120,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-diff-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        (root/"a.txt").write_text("same\nOLDLINE\ntail\n")
        (root/"b.txt").write_text("same\nNEWLINE\ntail\n")
        f=root/"a.txt"
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
        def row_tinted(y):
            return sum(1 for x in range(cols) if screen.buffer[y][x].bg!="default") > cols//2
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.4)
            key(":diffthis\r",.3)              # mark buffer a
            key(":e b.txt\r",.4)               # open b
            key(":diffthis\r",.4)              # mark b -> diff computes
            # In b, line 1 (NEWLINE) differs -> tinted; line 0 (same) does not.
            assert wait_for(lambda: row_tinted(1)), \
                ("the differing line should be highlighted\n"+text())
            assert not row_tinted(0), ("an equal line stays plain\n"+text())
            key(":diffoff\r",.3)
            assert wait_for(lambda: not row_tinted(1)), \
                (":diffoff should clear the highlight\n"+text())
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
    print(f"diff PTY passed: {cols}x{rows}")
