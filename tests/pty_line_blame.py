#!/usr/bin/env python3
""",gB toggles line-blame virtual text: the current line shows its
commit's short hash and author/date appended after the source text,
computed asynchronously so the editor never blocks on it -- driven
through a real PTY against a real git repository."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, subprocess, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-lineblame-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        def git(*args):
            subprocess.run(["git", *args], cwd=root, check=True, capture_output=True)
        git("init", "-q")
        git("config", "user.name", "Vaayu test")
        git("config", "user.email", "vaayu-test@example.invalid")
        f=root/"f.txt"
        f.write_text("alpha\nbeta\ngamma\n")
        git("add", "f.txt")
        git("commit", "-qm", "fixture")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(f)])
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
            key("j",.2)  # onto "beta"
            key(",gB",.4)
            assert wait_for(lambda: "Vaayu test" in text()), \
                ("line-blame should show the committing author on the "
                 "current line\n"+text())
            assert "alpha" in text() and "gamma" in text(), \
                ("other lines' own text must still be intact\n"+text())
            # Only the current line gets the annotation -- line 0 (alpha)
            # should not also show it, since blame is per-cursor-line.
            assert "Vaayu test" not in screen.display[0], \
                ("blame text should only appear on the current line\n"+text())
            key("j",.3)  # move to "gamma" -- blame should follow
            assert wait_for(lambda: "Vaayu test" in screen.display[2]), \
                ("blame annotation should follow the cursor to the new line\n"+text())
            key(",gB",.3)  # toggle off
            assert wait_for(lambda: "Vaayu test" not in text()), \
                ("toggling off should remove the blame annotation\n"+text())
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
    print(f"Line blame PTY passed: {cols}x{rows}")
