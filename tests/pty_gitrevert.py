#!/usr/bin/env python3
""":gitrevert HEAD undoes the last commit as a new commit and the open buffer
reloads to the reverted content -- driven through a real PTY against a real git
repository."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, subprocess, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-gitrevert-") as tmp:
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
        f.write_text("ORIGINALCONTENT\n"); git("add", "f.txt"); git("commit", "-qm", "first")
        f.write_text("CHANGEDCONTENT\n"); git("add", "f.txt"); git("commit", "-qm", "second")
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
            assert "CHANGEDCONTENT" in text(), ("buffer shows the committed change\n"+text())
            key(":gitrevert HEAD\r",.5)
            assert wait_for(lambda: "ORIGINALCONTENT" in text()), \
                ("revert should restore the original content in the buffer\n"+text())
            assert "CHANGEDCONTENT" not in text(), ("changed content should be gone\n"+text())
            # A new revert commit exists on disk.
            log=subprocess.run(["git","log","--oneline"],cwd=root,capture_output=True,text=True).stdout
            assert len(log.strip().splitlines())==3, ("revert should add a commit\n"+log)
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
    print(f"gitrevert PTY passed: {cols}x{rows}")
