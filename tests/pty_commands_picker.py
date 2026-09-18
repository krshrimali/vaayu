#!/usr/bin/env python3
""":commands lists every ex command; Enter pre-fills the command line
with the selected one (not executing it immediately, since most
commands need arguments) so the user can add args and press Enter
themselves -- driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-commands-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        file=root/"f.txt"
        file.write_text("needle in a haystack\nno match here\n")
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
        def wait_for(pred, timeout=5.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():
                    return True
                drain(.1)
            return False
        try:
            drain(.3)
            key(":commands\r",.4)
            assert wait_for(lambda: "results ·" in text()), \
                ("commands list never opened\n"+text())
            key("/:grep \r",.3)  # search within the list for the exact entry
            key("\r",.3)  # Enter selects it
            assert wait_for(lambda: text().endswith("grep ") or ":grep " in text()), \
                ("selecting :grep should pre-fill the command line\n"+text())
            # It must NOT have run yet -- no results list open.
            assert "results ·" not in text(), \
                ("the command should be pre-filled, not already executed\n"+text())
            key("needle\r",.3)  # add the pattern and run it
            assert wait_for(lambda: "needle" in text() and "results ·" in text()), \
                ("finishing the pre-filled command should actually run it\n"+text())
            key("\x1b",.2)
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
    print(f"Commands picker PTY passed: {cols}x{rows}")
