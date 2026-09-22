#!/usr/bin/env python3
"""Results-list preview pane (`p` toggle, `w` wrap, Ctrl-e/Ctrl-y scroll):
shows real surrounding file content around a grep hit, not just the hit's
own matched line -- driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-respreview-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        file=root/"f.txt"
        file.write_text("alpha\nNEEDLE beta\ngamma\ndelta\nepsilon\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(file)])
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
        def wait_for(predicate, timeout=5):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if predicate():
                    return True
                drain(.15)
            return False
        try:
            drain(.3)
            key(":grep NEEDLE\r",.5)
            assert wait_for(lambda: "NEEDLE beta" in text()), \
                ("grep did not find the match\n"+text())
            key("\x1b",.3)  # leave live-grep's query-editing sub-mode
            # Preview starts off: only the hit's own line/detail shows, not
            # its neighboring source lines.
            assert "alpha" not in text() and "gamma" not in text(), \
                ("preview should start off -- no neighboring lines yet\n"+text())
            key("p",.3)  # toggle preview on
            assert wait_for(lambda: "alpha" in text() and "gamma" in text()), \
                ("preview on should show real neighboring source lines\n"+text())
            assert "> " in text() or ">1" in text() or ">N" in text(), \
                ("the matched line should be marked in the preview\n"+text())
            # A clear labelled rule separates the results list from the preview.
            assert "── preview" in text(), \
                ("the search preview should have a visible border\n"+text())
            # Scroll down one: the matched line becomes the top of the
            # preview window, so the line before it (alpha) drops out.
            key("\x05",.3)  # Ctrl-e
            assert wait_for(lambda: "alpha" not in text()), \
                ("Ctrl-e should scroll the preview down, dropping alpha\n"+text())
            assert "NEEDLE beta" in text() and "gamma" in text(), \
                ("scrolled preview should still show the hit and what follows\n"+text())
            key("\x19",.3)  # Ctrl-y scrolls back up
            assert wait_for(lambda: "alpha" in text()), \
                ("Ctrl-y should scroll the preview back up\n"+text())
            key("w",.2)  # toggle wrap -- exercised for crash-safety; the
                          # wrapping math itself is covered by unit tests
            key("p",.3)  # toggle preview back off
            assert wait_for(lambda: "gamma" not in text()), \
                ("preview off should hide neighboring source lines again\n"+text())
            assert "── preview" not in text(), \
                ("the border should disappear with the preview\n"+text())
            key("\x1b",.2)
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
    print(f"Results preview PTY passed: {cols}x{rows}")
