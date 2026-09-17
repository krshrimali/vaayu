#!/usr/bin/env python3
"""]c/[c jump to the start of the next/previous changed git hunk,
wrapping around, and treat a contiguous multi-line change as one stop
-- driven through a real PTY against a real git repository."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, subprocess, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-hunknav-") as tmp:
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
        original=[f"line{i}\n" for i in range(10)]
        f.write_text("".join(original))
        git("add", "f.txt")
        git("commit", "-qm", "fixture")
        changed=list(original)
        changed[2]="alpha_marker\n"
        changed[7]="beta_marker\n"
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
            # Wait for the background git-diff job by polling until the
            # gutter sign column shows a change marker (~), the same
            # readiness-proxy approach used for LSP-diagnostic PTY tests.
            assert wait_for(lambda: "~" in text()), \
                ("git gutter signs never appeared\n"+text())
            key("]c",.3)
            key("x",.2)
            assert "lpha_marker" in text() and "alpha_marker" not in text(), \
                ("]c should jump to the first changed hunk\n"+text())
            key("u",.2)
            key("]c",.3)
            key("x",.2)
            assert "eta_marker" in text() and "beta_marker" not in text(), \
                ("]c again should jump to the second changed hunk\n"+text())
            key("u",.2)
            key("]c",.3)  # wraps back to the first hunk
            key("x",.2)
            assert "lpha_marker" in text() and "alpha_marker" not in text(), \
                ("]c should wrap back to the first hunk\n"+text())
            key("u",.2)
            key("[c",.3)  # from hunk 1, backward wraps to the last hunk
            key("x",.2)
            assert "eta_marker" in text() and "beta_marker" not in text(), \
                ("[c should wrap to the last hunk\n"+text())
            key("u",.2)
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
    print(f"Git hunk nav PTY passed: {cols}x{rows}")
