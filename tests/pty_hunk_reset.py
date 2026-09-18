#!/usr/bin/env python3
""",gx shows the saved hunk under the cursor as a confirmation prompt and,
on Enter, discards just that hunk back to HEAD -- restoring both the
working-tree file and the already-open buffer -- while leaving any other
hunk in the file untouched. Driven through a real PTY against a real git
repository with two separate hunks."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, subprocess, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-hunkreset-") as tmp:
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
        original=[f"line{i}\n" for i in range(20)]
        f.write_text("".join(original))
        git("add", "f.txt")
        git("commit", "-qm", "fixture")
        # Far enough apart (13 lines) that --unified=3's context windows
        # don't overlap and merge into a single hunk.
        changed=list(original)
        changed[2]="CHANGED_ALPHA\n"
        changed[15]="CHANGED_BETA\n"
        f.write_text("".join(changed))
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
            drain(.4)
            key("2G",.2)  # line 3 (index 2), inside the first hunk only
            key(",gx",.4)
            # The hunk may sit past what fits on screen without
            # scrolling, so search within the prompt list to bring it
            # into view (same technique pty_hunk_preview.py uses).
            key("/CHANGED_ALPHA\r",.3)
            assert wait_for(lambda: "CHANGED_ALPHA" in text()), \
                ("reset prompt should show the hunk under the cursor\n"+text())
            key("\r",.4)  # confirm: discard this hunk back to HEAD
            assert wait_for(lambda: "reset" in text().lower()), \
                ("confirming should report the hunk was reset\n"+text())
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
        on_disk=f.read_text()
        assert "line2\n" in on_disk, \
            ("the reset hunk should be back to HEAD on disk\n"+on_disk)
        assert "CHANGED_ALPHA" not in on_disk, \
            ("the reset hunk's change should be gone\n"+on_disk)
        assert "CHANGED_BETA\n" in on_disk, \
            ("the other, untouched hunk should be left alone\n"+on_disk)
    print(f"Hunk reset PTY passed: {cols}x{rows}")
