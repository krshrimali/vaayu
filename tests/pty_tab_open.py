#!/usr/bin/env python3
"""Ctrl-T in the file picker and in any results/quickfix list opens the
selection into a brand-new tab (instead of the current pane or a split)
-- driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-tabopen-") as tmp:
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
        def ctrlt(seconds=.3):os.write(fd,b"\x14");drain(seconds)
        def text():
            return "\n".join(screen.display)
        try:
            drain(.4)
            # File picker: Ctrl-T opens the match into a new tab, replacing
            # the visible content with b.txt (not splitting the pane).
            key("\x10",.3)  # Ctrl-P: fuzzy file picker
            key("b.txt",.3)
            ctrlt()
            assert " 2 " in text(), ("Ctrl-t in the picker should open a tabline with a 2nd tab\n"+text())
            assert "unique_marker_b" in text() and "file a content" not in text(), \
                ("Ctrl-t should show b.txt, not split alongside a.txt\n"+text())
            key("gT",.2)  # back to tab 1
            assert "file a content" in text() and "unique_marker_b" not in text(), \
                ("gT did not return to the original tab with a.txt\n"+text())
            key(":tabclose\r",.2)  # discard the tab we just proved, back to a single tab
            key(":tabonly\r",.2)
            # Live grep results: Ctrl-T opens the match into a new tab too.
            key(",/unique_marker_b\r",.3)
            ctrlt()
            assert " 2 " in text(), ("Ctrl-t in results should open a tabline with a 2nd tab\n"+text())
            assert "unique_marker_b" in text() and "file a content" not in text(), \
                ("Ctrl-t in results should show b.txt, not split alongside a.txt\n"+text())
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
    print(f"Tab-open PTY passed: {cols}x{rows}")
