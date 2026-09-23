#!/usr/bin/env python3
"""`:taskwatch <cmd>` re-runs the command into the quickfix on every save.
Here `:taskwatch echo WATCHEDOK` then a save shows the command's output. PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(120,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-tw-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"a.txt"; f.write_text("one\n")
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
        def key(s,seconds=.4):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        def wait_for(pred,timeout=5.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.15)
            return False
        try:
            drain(.5)
            key(":taskwatch echo WATCHEDOK\r",.6)
            assert wait_for(lambda: "WATCHEDOK" in text()), \
                ("taskwatch should run the command\n"+text())
            # Dismiss the results, edit, and save -> it re-runs.
            key("\x1b",.2)      # close results view if shown
            key("ix\x1b",.3)    # modify the buffer
            key(":w\r",.6)
            assert wait_for(lambda: "WATCHEDOK" in text()), \
                ("save should re-run the watched command\n"+text())
            key(":taskwatchoff\r",.3)
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
    print(f"task watch PTY passed: {cols}x{rows}")
