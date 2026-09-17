#!/usr/bin/env python3
"""File tree: / live-filters the currently-loaded nodes by substring;
Backspace narrows back, Esc clears the filter, Enter keeps it while
returning to normal navigation -- driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-filetree-filt-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-filetree-filt-cfg-") as cfg:
        root=pathlib.Path(proj)
        cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        apple=root/"apple.txt"; apple.write_text("x\n")
        (root/"banana.txt").write_text("x\n")
        (root/"cherry.txt").write_text("x\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(cfg))
            os.execv(binary,[binary,str(apple)])
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
        def right_text():
            half = cols // 2
            return "\n".join(row[half:] for row in screen.display)
        try:
            drain(.3)
            key(",ft",.3)
            assert "apple.txt" in right_text() and "banana.txt" in right_text() \
                and "cherry.txt" in right_text(), ("tree missing files\n"+right_text())
            key("/",.2)
            key("an",.2)
            assert "banana.txt" in right_text(), ("filter 'an' should keep banana.txt\n"+right_text())
            assert "apple.txt" not in right_text() and "cherry.txt" not in right_text(), \
                ("filter 'an' should hide apple.txt and cherry.txt\n"+right_text())
            key("\x7f",.2)  # Backspace: back to "a"
            assert "apple.txt" in right_text() and "banana.txt" in right_text(), \
                ("backspacing to 'a' should show apple.txt and banana.txt again\n"+right_text())
            assert "cherry.txt" not in right_text()
            key("\x1b",.2)  # Esc clears the filter entirely
            assert "cherry.txt" in right_text(), \
                ("Esc should clear the filter, restoring cherry.txt\n"+right_text())
            # Enter keeps the filter applied while returning to navigation.
            key("/",.2)
            key("ban",.2)
            key("\r",.2)
            assert "banana.txt" in right_text() and "apple.txt" not in right_text(), \
                ("Enter should keep the filter applied\n"+right_text())
            key("j",.2)  # normal navigation must work again after Enter
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
    print(f"File tree filter PTY passed: {cols}x{rows}")
