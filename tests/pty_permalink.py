#!/usr/bin/env python3
""",gp copies a GitHub permalink (blob URL pinned to HEAD's commit, with
a #L<n> or #L<n>-L<m> fragment) for the cursor line or a Visual
selection; :permalink does the same for just the cursor line -- driven
through a real PTY against a real git repository with a github.com
origin remote."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, subprocess, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-permalink-") as tmp:
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
        git("remote", "add", "origin", "git@github.com:acme/widgets.git")
        f=root/"f.txt"
        f.write_text("one\ntwo\nthree\n")
        git("add", "f.txt")
        git("commit", "-qm", "fixture")
        sha=subprocess.run(["git","rev-parse","HEAD"],cwd=root,check=True,
                            capture_output=True,text=True).stdout.strip()
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
            # Cursor starts on line 1 ("one"); ,gp should link just that line.
            key(",gp",.3)
            assert wait_for(lambda: "Copied permalink" in text()), \
                ("permalink message never appeared\n"+text())
            # A narrow terminal right-truncates the message line well
            # before the URL's tail (the 40-hex-char SHA alone makes the
            # full URL over 100 columns long), so only check progressively
            # further into it on wider terminals where more of it fits.
            if cols >= 100:
                assert wait_for(lambda: "acme/widgets/blob" in text()), \
                    ("permalink URL missing owner/repo/blob\n"+text())
            if cols >= 180:
                assert wait_for(lambda: f"{sha}/f.txt#L1" in text()), \
                    ("permalink URL missing commit/path/line\n"+text())
            # :permalink from line 3 should link #L3, not #L1.
            key("jj",.2)  # onto "three" (line 3)
            key(":permalink\r",.3)
            assert wait_for(lambda: "Copied permalink" in text()), \
                ("colon permalink message never appeared\n"+text())
            if cols >= 180:
                assert wait_for(lambda: "#L3" in text()), \
                    ("colon permalink should use the cursor's own line\n"+text())
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
    print(f"Permalink PTY passed: {cols}x{rows}")
