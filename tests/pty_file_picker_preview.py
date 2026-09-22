#!/usr/bin/env python3
"""Ctrl-r toggles a content-preview pane in the fuzzy file picker, showing
the currently-selected match's own file content below the list -- off by
default, and updating as the selection moves. Driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(60,16),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-pickerpreview-") as tmp:
        base=pathlib.Path(tmp)
        root=base/"project"; root.mkdir()
        (base/"config/vaayu").mkdir(parents=True)
        (base/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        (root/"alpha.txt").write_text("ALPHA_CONTENT_MARKER\n")
        (root/"zzz_other.txt").write_text("unrelated content\n")
        entry=root/"zzz_other.txt"
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(base/"config"))
            os.execv(binary,[binary,str(entry)])
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
        def wait_for(pred, timeout=6.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():
                    return True
                drain(.1)
            return False
        try:
            drain(.4)
            key("\x10",.3)  # Ctrl-P: fuzzy file picker
            key("alpha",.3)
            assert wait_for(lambda: "alpha.txt" in text()), \
                ("query should narrow to alpha.txt\n"+text())
            assert "ALPHA_CONTENT_MARKER" not in text(), \
                ("no preview content before Ctrl-r\n"+text())

            key("\x12",.3)  # Ctrl-r: toggle preview on
            assert wait_for(lambda: "ALPHA_CONTENT_MARKER" in text()), \
                ("preview pane should show the selected file's content\n"+text())
            # A clear labelled rule separates the list from the preview pane.
            assert "── preview" in text(), \
                ("preview pane should have a visible border\n"+text())

            key("\x12",.3)  # Ctrl-r: toggle preview back off
            assert wait_for(lambda: "ALPHA_CONTENT_MARKER" not in text()), \
                ("toggling off should stop rendering the preview\n"+text())
            assert "── preview" not in text(), \
                ("the border should disappear with the preview\n"+text())

            key("\x1b",.2)  # Esc: close the picker
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
    print(f"File picker preview PTY passed: {cols}x{rows}")
