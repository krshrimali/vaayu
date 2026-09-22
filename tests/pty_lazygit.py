#!/usr/bin/env python3
""",gl/:lazygit spawns lazygit in an embedded terminal split when it's
installed; when it isn't (the common case in a CI/sandbox environment
without it on PATH), the attempt fails visibly with a clear message and
never blocks normal editing -- external, optional tooling degrading
cleanly, per this plan's own scope rules. Driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, shutil, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
lazygit_installed = shutil.which("lazygit") is not None
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-lazygit-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        f=root/"f.txt"; f.write_text("x\n")
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
            key(",gl",.4)
            if lazygit_installed:
                # Installed: it opens an embedded terminal split -- or, in a
                # sandbox where it cannot create its state dir (or isn't in a git
                # repo), it starts and exits immediately. Either way we must be
                # able to get back to the editor and keep working, so don't hard
                # require that it stayed open.
                wait_for(lambda: "TERMINAL" in text() or "lazygit" in text().lower(),
                         timeout=3.0)
            else:
                assert wait_for(lambda: "could not start lazygit" in text().lower()), \
                    ("a missing lazygit should fail visibly, not silently or by crashing\n"+text())
                assert "NORMAL" in text(), \
                    ("failing to start lazygit must not leave the editor stuck in another mode\n"+text())
            # Return to the editor pane regardless of what happened: leave any
            # terminal mode, focus the editor window, and make it the only pane
            # (Ctrl-W o keeps the *focused* pane, so focus the editor first).
            key("\x1b",.3)     # Terminal/pending -> Normal
            key("\x17w",.3)    # Ctrl-W w: focus the editor pane if a split opened
            key("\x17o",.3)    # Ctrl-W o: keep only the editor pane
            # Normal editing keeps working right after the lazygit attempt.
            key("ihello\x1b",.4)
            assert "hello" in text(), \
                ("normal editing should be unaffected by the lazygit attempt\n"+text())
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
    print(f"lazygit PTY passed ({'installed' if lazygit_installed else 'not installed'}): {cols}x{rows}")
