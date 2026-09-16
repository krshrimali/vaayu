#!/usr/bin/env python3
"""Ctrl-V/Ctrl-X open a picker/results selection into a new vertical or
horizontal split instead of replacing the current pane -- driven through
a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-splitopen-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        a=root/"a.txt"; a.write_text("file a content\n")
        b=root/"b.txt"; b.write_text("unique_marker_b content\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(a)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.25):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                ready,_,_=select.select([fd],[],[],min(.02,max(0,end-time.monotonic())))
                if ready:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.2):os.write(fd,s.encode());drain(seconds)
        def text():
            return "\n".join(screen.display)
        try:
            drain(.4)
            # File picker: Ctrl-V opens the match into a vertical split,
            # keeping file a visible in the original pane.
            key("\x10",.3)  # Ctrl-P: fuzzy file picker
            key("b.txt",.3)
            key("\x16",.4)  # Ctrl-V
            assert "file a content" in text() and "unique_marker_b" in text(), \
                ("Ctrl-V in the picker did not split, showing both files\n"+text())
            # Ctrl-V's new pane is focused and appears on the right; move
            # back to the original (a.txt) pane before discarding the rest,
            # so :only keeps a.txt, not the just-opened b.txt.
            key("\x17h",.2)
            key(":only\r",.3)
            assert "file a content" in text() and "unique_marker_b" not in text(), \
                (":only did not leave a.txt as the sole pane\n"+text())
            # Live grep results: Ctrl-X opens the match into a horizontal split.
            key(",/unique_marker_b\r",.3)
            key("\x18",.4)  # Ctrl-X
            assert "file a content" in text() and "unique_marker_b" in text(), \
                ("Ctrl-X in results did not split, showing both files\n"+text())
            key(":qa\r")
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
    print(f"Split-open PTY passed: {cols}x{rows}")
