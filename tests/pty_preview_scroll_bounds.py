#!/usr/bin/env python3
"""Ctrl-e (scroll a preview pane down) used to have no upper bound at
all: in both the Results-list preview (,gh/live grep/etc.) and the
file-picker preview, holding Ctrl-e scrolled past the source's own
last line into permanently blank space with no way back except
pressing Ctrl-y exactly as many times -- reported as "scrolling
infinitely". This drives both surfaces well past their content and
confirms the last real line stays on screen instead. Driven through a
real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-scrollbounds-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        f=root/"f.txt"
        f.write_text("needle\nsecond\nthird\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(f)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.2):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                ready,_,_=select.select([fd],[],[],min(.02,max(0,end-time.monotonic())))
                if ready:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.15):os.write(fd,s.encode());drain(seconds)
        def text():
            return "\n".join(screen.display)
        def wait_for(pred, timeout=6.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():
                    return True
                drain(.1)
            return False
        try:
            drain(.3)
            key(",/",.2); key("needle",.4); key("\x1b",.2)  # live grep, browsing
            key("p",.3)  # preview on
            assert wait_for(lambda: "third" in text()), \
                ("preview should show the file's content before scrolling\n"+text())
            for _ in range(30):
                key("\x05",.02)  # Ctrl-e, way past the file's 3 lines
            drain(.3)
            assert "third" in text(), \
                ("the file's last real line should stay visible once maximally "
                 "scrolled, not disappear into permanent blank space\n"+text())
            key("\x1b\x1b",.3)

            # Same check for the file-picker's own preview.
            key("\x10",.3)  # Ctrl-P
            key("f.txt",.3)
            key("\x12",.3)  # Ctrl-r preview on
            assert wait_for(lambda: "needle" in text()), \
                ("file picker preview should show content before scrolling\n"+text())
            for _ in range(30):
                key("\x05",.02)
            drain(.3)
            assert "needle" in text() or "third" in text(), \
                ("the file picker's own preview should also stay bounded, not "
                 "scroll into permanent blank space\n"+text())
            key("\x1b",.2)

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
    print(f"Preview scroll bounds PTY passed: {cols}x{rows}")
