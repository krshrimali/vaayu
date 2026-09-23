#!/usr/bin/env python3
"""`:testnearest` runs the test function under the cursor via the task runner.
Here the enclosing `def test_thing` yields `pytest -k test_thing`, run through
the shell; the results panel echoes the command that ran. PTY-driven."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(120,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-tn-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"t.py"; f.write_text("def helper():\n    return 1\n\ndef test_thing():\n    assert helper() == 1\n")
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
        def key(s,seconds=.4):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        def wait_for(pred,timeout=8.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.15)
            return False
        try:
            drain(.6)
            # Move into the second function's body.
            key("G",.3)     # last line (inside test_thing)
            key(":testnearest\r",.6)
            # The task runner echoes the constructed command (title/output).
            assert wait_for(lambda: "pytest" in text() and "test_thing" in text()), \
                (":testnearest should run 'pytest -k test_thing'\n"+text())
            key(":qa!\r")
            end=time.monotonic()+4
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
    print(f"testnearest PTY passed: {cols}x{rows}")
