#!/usr/bin/env python3
"""Inline ghost text (`:set ghosttext`): typing a prefix that matches an earlier
line shows the rest as dimmed virtual text; Ctrl-l accepts it (inserting it for
real). The suggestion is virtual until accepted. PTY-driven."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-ghost-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\nghost_text=true\n')
        f=root/"a.txt"; f.write_text("hello world foo\n\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(f)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.4):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                r,_,_=select.select([fd],[],[],.05)
                if r:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def row(y):return "".join(screen.buffer[y][x].data for x in range(cols))
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.5)
            os.write(fd,b"jihello wo"); drain(.5)  # line 2: type the matching prefix
            # The ghost completes it inline: row 1 reads "hello world foo".
            assert wait_for(lambda: "hello world foo" in row(1)), \
                ("ghost text should complete the line inline\n"+"\n".join(screen.display))
            os.write(fd,b"\x0c"); drain(.4)         # Ctrl-l accepts
            os.write(fd,b"\x1b"); drain(.25)        # Esc closes the completion popup
            os.write(fd,b"\x1b"); drain(.25)        # Esc leaves insert mode
            os.write(fd,b":w\r"); drain(.4)
            os.write(fd,b":q!\r"); drain(.3)
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
        # After accept, the suggestion is really in the file.
        assert f.read_text()=="hello world foo\nhello world foo\n", f.read_text()
    print(f"ghost text PTY passed: {cols}x{rows}")
