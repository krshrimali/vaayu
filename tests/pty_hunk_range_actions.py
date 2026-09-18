#!/usr/bin/env python3
""",gs and ,gx in Visual mode act on only the selected lines within a
saved hunk (via a reconstructed sub-patch), not the whole hunk -- ,gs
stages just those lines into the index, ,gx shows a confirmation prompt
and, on Enter, resets just those lines in the working tree, both
leaving the hunk's other changed lines untouched. Driven through a real
PTY against a real git repository with two separate three-line hunks."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, subprocess, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-hunkrange-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        def git(*args, **kw):
            return subprocess.run(["git", *args], cwd=root, check=True,
                                   capture_output=True, text=True, **kw)
        git("init", "-q")
        git("config", "user.name", "Vaayu test")
        git("config", "user.email", "vaayu-test@example.invalid")
        f=root/"f.txt"
        original=[f"line{i}\n" for i in range(30)]
        f.write_text("".join(original))
        git("add", "f.txt")
        git("commit", "-qm", "fixture")
        # Two three-line change blocks far enough apart (13+ lines) that
        # --unified=3's context windows stay separate hunks. Each block
        # is a straight one-for-one line replacement, so a Visual
        # selection over its middle+last line exercises the paired
        # sub-patch split against a real ,gs (stage) and ,gx (reset).
        changed=list(original)
        changed[5]="STAGE_A\n"; changed[6]="STAGE_B\n"; changed[7]="STAGE_C\n"
        changed[20]="RESET_A\n"; changed[21]="RESET_B\n"; changed[22]="RESET_C\n"
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
            # Stage only STAGE_B and STAGE_C (not STAGE_A) via ,gs.
            key("/STAGE_B\r",.3)
            key("Vj",.2)  # select STAGE_B and STAGE_C
            key(",gs",.4)
            assert wait_for(lambda: "staged" in text().lower()), \
                ("Visual ,gs should report a stage\n"+text())

            # Reset only RESET_B and RESET_C (not RESET_A) via ,gx.
            key("/RESET_B\r",.3)
            key("Vj",.2)  # select RESET_B and RESET_C
            key(",gx",.4)
            assert wait_for(lambda: "reset" in text().lower()), \
                ("Visual ,gx should show a range confirmation prompt\n"+text())
            # The sub-patch's changed lines may sit past what fits on
            # screen without scrolling -- search within the prompt list
            # to bring one into view, same technique pty_hunk_reset.py
            # already uses for a whole-hunk preview.
            key("/RESET_B\r",.3)
            assert wait_for(lambda: "RESET_B" in text()), \
                ("range reset prompt should show the selected lines' sub-patch\n"+text())
            key("\r",.4)  # confirm: discard just the selected lines
            assert wait_for(lambda: "reset" in text().lower()), \
                ("confirming should report the range was reset\n"+text())

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

        staged=git("show", ":f.txt").stdout
        assert "STAGE_B" in staged and "STAGE_C" in staged, \
            ("the selected lines should be staged\n"+staged)
        assert "STAGE_A" not in staged, \
            ("the unselected line in the same hunk should stay unstaged\n"+staged)

        on_disk=f.read_text()
        assert "line20\n" not in on_disk and "RESET_A\n" in on_disk, \
            ("the unselected line should be left alone\n"+on_disk)
        assert "line21\n" in on_disk and "line22\n" in on_disk, \
            ("the selected lines should be reset back to HEAD\n"+on_disk)
        assert "RESET_B" not in on_disk and "RESET_C" not in on_disk, \
            ("the reset lines' changes should be gone\n"+on_disk)
    print(f"Hunk range actions PTY passed: {cols}x{rows}")
