#!/usr/bin/env python3
"""File tree: a .gitignore'd path is hidden by default (an entirely
ignored directory collapses to one hidden entry, not read_dir'd into);
! toggles showing it -- driven through a real PTY against a real git
repository."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, subprocess, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-filetree-gi-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-filetree-gi-cfg-") as cfg:
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
        (root/"target").mkdir()
        (root/"target"/"built.txt").write_text("x\n")
        (root/".gitignore").write_text("target/\n")
        kept=root/"kept.txt"; kept.write_text("x\n")
        git("add", ".gitignore")
        git("commit", "-qm", "fixture")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(cfg))
            os.execv(binary,[binary,str(kept)])
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
            assert "kept.txt" in right_text(), ("tree missing kept.txt\n"+right_text())
            assert "target" not in right_text(), \
                ("gitignored target/ should be hidden by default\n"+right_text())
            key("!",.2)
            assert "target" in right_text(), ("! should reveal target/\n"+right_text())
            key("!",.2)
            assert "target" not in right_text(), ("! again should hide it\n"+right_text())
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
    print(f"File tree gitignore PTY passed: {cols}x{rows}")
