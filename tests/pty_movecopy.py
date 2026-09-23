#!/usr/bin/env python3
"""`:m`/`:t` move/copy lines. `:1m$` moves line 1 to the end; `:1t$` copies
line 1 to the end. Verified on disk through a real PTY."""
import os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time, fcntl, codecs
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(80,24),(120,40)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-mv-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"lines.txt"; f.write_text("one\ntwo\nthree\n")
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
        try:
            drain(.5)
            key(":1m$\r",.4)   # move "one" to the end
            key(":w\r",.3)
            assert f.read_text()=="two\nthree\none\n", ("after move: %r"%f.read_text())
            key(":1t$\r",.4)   # copy "two" to the end
            key(":wq\r",.4)
            end=time.monotonic()+3
            while time.monotonic()<end:
                done,status=os.waitpid(pid,os.WNOHANG)
                if done:
                    assert os.waitstatus_to_exitcode(status)==0;pid=None;break
                drain(.05)
            assert pid is None,"Editor failed to exit"
            assert f.read_text()=="two\nthree\none\ntwo\n", ("after copy: %r"%f.read_text())
        finally:
            if pid:
                os.kill(pid,signal.SIGKILL);os.waitpid(pid,0)
            os.close(fd)
    print(f"move/copy PTY passed: {cols}x{rows}")
