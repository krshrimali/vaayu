#!/usr/bin/env python3
"""File tree: a modified or untracked file shows its git status letter
(M/?), a clean tracked file shows none, refreshed on open and R --
driven through a real PTY against a real git repository."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, subprocess, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-filetree-gitstatus-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-filetree-gitstatus-cfg-") as cfg:
        root=pathlib.Path(proj)
        cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        def git(*args):
            subprocess.run(["git", *args], cwd=root, check=True, capture_output=True)
        git("init", "-q")
        git("config", "user.name", "Vaayu test")
        git("config", "user.email", "vaayu-test@example.invalid")
        clean=root/"clean.txt"; clean.write_text("a\n")
        modified=root/"modified.txt"; modified.write_text("a\n")
        git("add", ".")
        git("commit", "-qm", "fixture")
        modified.write_text("b\n")
        untracked=root/"untracked.txt"; untracked.write_text("x\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(cfg))
            os.execv(binary,[binary,str(clean)])
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
            assert "modified.txt M" in right_text(), \
                ("modified.txt should show its M status\n"+right_text())
            assert "untracked.txt ?" in right_text(), \
                ("untracked.txt should show its ? status\n"+right_text())
            assert "clean.txt M" not in right_text() and "clean.txt ?" not in right_text(), \
                ("clean.txt must not show a status marker\n"+right_text())
            # Fix the modification, then R refreshes to reflect it.
            modified.write_text("a\n")
            key("R",.3)
            assert "modified.txt M" not in right_text(), \
                ("R should refresh git status once the file matches HEAD again\n"+right_text())
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
    print(f"File tree git status PTY passed: {cols}x{rows}")
