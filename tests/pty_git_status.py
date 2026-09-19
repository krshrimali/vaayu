#!/usr/bin/env python3
""":gitstatus opens the Git workspace (staged/unstaged/untracked
sections); s/u move a file between staged and unstaged, and c commits
staged changes via a command-line-prefilled message -- driven through a
real PTY against a real git repository."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, subprocess, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(60,16),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-gitstatus-") as tmp:
        base=pathlib.Path(tmp)
        # Config lives *outside* the git repo (unlike most other PTY
        # tests, which don't care) since this test asserts an exact
        # "Untracked (N)" count and the config dir would otherwise show
        # up as untracked files of its own.
        root=base/"project"; root.mkdir()
        (base/"config/vaayu").mkdir(parents=True)
        (base/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        def git(*args):
            subprocess.run(["git", *args], cwd=root, check=True, capture_output=True)
        git("init", "-q")
        git("config", "user.name", "Vaayu test")
        git("config", "user.email", "vaayu-test@example.invalid")
        tracked=root/"tracked.txt"
        tracked.write_text("original\n")
        git("add", "tracked.txt")
        git("commit", "-qm", "fixture")
        tracked.write_text("changed\n")
        (root/"new.txt").write_text("untracked content\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(base/"config"))
            os.execv(binary,[binary,str(tracked)])
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
            key(":gitstatus\r",.4)
            assert wait_for(lambda: "Unstaged (1)" in text() and "tracked.txt" in text()), \
                ("git status should list the modified tracked file as unstaged\n"+text())
            assert "Untracked (1)" in text() and "new.txt" in text(), \
                ("git status should list the untracked file\n"+text())
            assert "Staged" not in text(), \
                ("nothing is staged yet\n"+text())

            key("j",.2)  # onto "M tracked.txt"
            key("s",.4)  # stage it
            assert wait_for(lambda: "Staged (1)" in text()), \
                ("s should stage the file under the cursor\n"+text())
            assert "Unstaged" not in text(), \
                ("tracked.txt should no longer show as unstaged\n"+text())

            key("j",.2)  # onto "M tracked.txt" again, now under Staged
            key("c",.3)  # commit prompt
            assert wait_for(lambda: ":gitcommit" in text()), \
                ("c should prefill the :gitcommit command line\n"+text())
            key("a real commit message\r",.5)
            assert wait_for(lambda: "Staged" not in text()), \
                ("committing should clear the staged section\n"+text())
            assert "Untracked (1)" in text() and "new.txt" in text(), \
                ("the untracked file should remain untouched by the commit\n"+text())

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
        log=subprocess.run(["git","log","--oneline"],cwd=root,capture_output=True,text=True).stdout
        assert "a real commit message" in log, \
            ("the commit should actually exist in history\n"+log)
    print(f"Git status workspace PTY passed: {cols}x{rows}")
