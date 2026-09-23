#!/usr/bin/env python3
""":gitfilehistory lists the commits that touched the current file as a Results
picker, and Enter on an entry shows that commit's diff -- driven through a real
PTY against a real git repository."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, subprocess, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-gitfh-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n')
        def git(*args):
            subprocess.run(["git", *args], cwd=root, check=True, capture_output=True)
        git("init", "-q")
        git("config", "user.name", "Vaayu test")
        git("config", "user.email", "vaayu-test@example.invalid")
        f=root/"f.txt"
        f.write_text("one\n"); git("add", "f.txt"); git("commit", "-qm", "ADDFILECOMMIT")
        f.write_text("one\nDIFFMARKER\n"); git("add", "f.txt"); git("commit", "-qm", "EDITFILECOMMIT")
        # An unrelated commit that does NOT touch f.txt.
        (root/"other.txt").write_text("x\n"); git("add", "other.txt"); git("commit", "-qm", "OTHERCOMMIT")
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
        def text():return "\n".join(screen.display)
        def wait_for(pred, timeout=5.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.1)
            return False
        try:
            drain(.4)
            key(":gitfilehistory\r",.3)
            assert wait_for(lambda: "EDITFILECOMMIT" in text()), \
                ("gitfilehistory should list commits touching f.txt\n"+text())
            assert "ADDFILECOMMIT" in text(), ("both file-touching commits listed\n"+text())
            assert "OTHERCOMMIT" not in text(), ("unrelated commit must be excluded\n"+text())
            key("\r",.3)                    # Enter shows the top commit's diff
            key("/DIFFMARKER\r",.3)
            assert wait_for(lambda: "DIFFMARKER" in text()), \
                ("Enter on a commit should show its diff\n"+text())
            key("q",.2)
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
    print(f"git-file-history PTY passed: {cols}x{rows}")
