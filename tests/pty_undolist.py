#!/usr/bin/env python3
"""`:undolist` opens the undo-history viewer; the current state is marked and
selecting an older state jumps the buffer back to it. PTY, 2 geometries."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(120,24),(160,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-ul-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"orig.txt"; f.write_text("hello\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(f)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.35):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                r,_,_=select.select([fd],[],[],.05)
                if r:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.35):os.write(fd,s.encode());drain(seconds)
        def esc():
            # The completion popup can eat the first Esc, so send two as
            # separate writes with a drain between (crossterm mis-parses a
            # single \x1b\x1b write).
            key("\x1b",.15); key("\x1b",.15)
        def text():return "\n".join(screen.display)
        try:
            drain(.5)
            key("A world"); esc()      # edit 1: "hello world"
            key("oSECOND"); esc()      # edit 2: add a line
            assert "hello world" in text() and "SECOND" in text(), ("edits applied\n"+text())
            key(":undolist\r",.5)
            assert "Undo history" in text(), ("viewer opens\n"+text())
            assert "current" in text(), ("current state marked\n"+text())
            # The panel opens on entry #0 (the oldest state); Enter jumps there.
            key("\r",.5)
            assert "world" not in text(), ("jumping to #0 drops edit 1\n"+text())
            assert "SECOND" not in text(), ("jumping to #0 drops edit 2\n"+text())
            assert "hello" in text(), ("original text restored\n"+text())
            key(":q!\r")
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
    print(f"undolist PTY passed: {cols}x{rows}")
