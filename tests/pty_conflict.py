#!/usr/bin/env python3
"""Merge-conflict resolution: `:conflictnext` jumps to a `<<<<<<<` block and
`:conflicttheirs` keeps their side, dropping the markers. Verified on disk."""
import os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time, fcntl, codecs
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
CONFLICT=("top\n<<<<<<< HEAD\nours line\n=======\ntheirs line\n>>>>>>> branch\nbottom\n")
for cols,rows in [(80,24),(120,40)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-mc-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"merge.txt"; f.write_text(CONFLICT)
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
        try:
            drain(.5)
            assert "<<<<<<<" in text(), ("conflict markers visible\n"+text())
            key(":conflictnext\r",.3)   # jump to the <<<<<<< marker
            key(":conflicttheirs\r",.4)  # keep their side
            assert "<<<<<<<" not in text() and ">>>>>>>" not in text(), \
                ("markers should be gone after resolve\n"+text())
            key(":wq\r",.5)
            end=time.monotonic()+3
            while time.monotonic()<end:
                done,status=os.waitpid(pid,os.WNOHANG)
                if done:
                    assert os.waitstatus_to_exitcode(status)==0;pid=None;break
                drain(.05)
            assert pid is None,"Editor failed to exit"
            got=f.read_text()
            assert got=="top\ntheirs line\nbottom\n", ("resolved file: %r"%got)
        finally:
            if pid:
                os.kill(pid,signal.SIGKILL);os.waitpid(pid,0)
            os.close(fd)
    print(f"merge conflict PTY passed: {cols}x{rows}")
