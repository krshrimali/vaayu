#!/usr/bin/env python3
"""`:zen` hides the line-number gutter and the per-pane status line (reclaiming
that row for content); scrolling still works; toggling back restores both.
Driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(80,14),(120,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-zen-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        # number defaults on, so a gutter is normally present.
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\n')
        f=root/"f.txt"; f.write_text("".join(f"line{i:02}\n" for i in range(1,41)))
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
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.4)
            # Normally: a status line ("NORMAL ...") and a numbered gutter.
            assert "NORMAL" in text(), ("status line should be visible\n"+text())
            assert not screen.display[0].startswith("line01"), \
                ("gutter should offset the content\n"+text())
            key(":zen\r",.4)
            assert wait_for(lambda: "NORMAL" not in text()), \
                ("zen should hide the status line\n"+text())
            assert wait_for(lambda: screen.display[0].startswith("line01")), \
                ("zen should hide the gutter (content at col 0)\n"+text())
            # Scrolling still works in zen.
            key("G",.3)
            assert wait_for(lambda: "line40" in text()), ("G should scroll to the end\n"+text())
            key("gg",.3)
            assert wait_for(lambda: screen.display[0].startswith("line01")), \
                ("gg should return to the top\n"+text())
            # Toggle back: status + gutter return.
            key(":zen\r",.4)
            assert wait_for(lambda: "NORMAL" in text()), ("exiting zen restores the status line\n"+text())
            assert not screen.display[0].startswith("line01"), ("gutter restored\n"+text())
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
    print(f"zen PTY passed: {cols}x{rows}")
