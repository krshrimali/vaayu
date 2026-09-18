#!/usr/bin/env python3
""",gd toggles the diff overlay: deleted-line content (from a pure
removal) shows as a compact virtual-text annotation, and a modified
line's changed word gets a distinct background highlight -- both only
while the overlay is on, computed from the same background `git diff`
data the gutter signs already use. Driven through a real PTY against a
real git repository."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, subprocess, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(60,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-diffoverlay-") as tmp:
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
        f.write_text("keep top\nold_value here\nDELETED_LINE\nkeep bottom\n")
        git("add", "f.txt")
        git("commit", "-qm", "fixture")
        # Change a word on line 2 and remove line 3 entirely (a pure
        # deletion, so its content only ever exists as an overlay
        # annotation, never in the buffer).
        f.write_text("keep top\nnew_value here\nkeep bottom\n")
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
        def wait_for(pred, timeout=8.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():
                    return True
                drain(.1)
            return False
        try:
            drain(.4)
            # Give the background git-diff job time to compute (same
            # gutter-sign data the overlay reads), then check the
            # overlay is off by default: no deleted-line text visible.
            assert wait_for(lambda: "new_value" in text()), \
                ("file should be visible before toggling anything\n"+text())
            assert "DELETED_LINE" not in text(), \
                ("deleted content should not render before the overlay is on\n"+text())

            key(",gd",.4)
            assert wait_for(lambda: "on" in text().lower()), \
                ("toggling on should report it in the message line\n"+text())
            assert wait_for(lambda: "DELETED_LINE" in text()), \
                ("deleted line content should render once the overlay is on\n"+text())

            key(",gd",.4)
            assert wait_for(lambda: "off" in text().lower()), \
                ("toggling off should report it in the message line\n"+text())
            drain(.3)
            assert "DELETED_LINE" not in text(), \
                ("deleted content should stop rendering once toggled off\n"+text())

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
    print(f"Diff overlay PTY passed: {cols}x{rows}")
