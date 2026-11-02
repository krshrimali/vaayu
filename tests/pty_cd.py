#!/usr/bin/env python3
""":cd changes the working directory and project root: :pwd reports it,
relative :e paths and the file picker follow it, Tab completes only
directories, :cd - goes back, and a missing directory is an error --
driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(60,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-cd-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-cd-other-") as other, \
         tempfile.TemporaryDirectory(prefix="vaayu-cd-cfg-") as cfg:
        root=pathlib.Path(proj).resolve()
        other_root=pathlib.Path(other).resolve()
        cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        f=root/"f.txt"; f.write_text("hello\n")
        (root/"old_only.txt").write_text("x\n")
        (other_root/"zzmarker.txt").write_text("marker contents\n")
        (other_root/"subdir").mkdir()
        (other_root/"subfile.txt").write_text("x\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(cfg))
            os.execv(binary,[binary,str(f)])
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
        def text():
            return "\n".join(screen.display)
        def flat():
            # A long path can wrap or be cut at narrow widths; compare the
            # screen with line breaks and padding removed.
            return "".join(line.rstrip() for line in screen.display)
        def wait_for(pred, timeout=5.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():
                    return True
                drain(.1)
            return False
        try:
            drain(.3)
            # Index the old root first so a stale file list would show up.
            key("\x10",.5)
            assert wait_for(lambda: "old_only" in text()), ("picker should list the launch root\n"+text())
            key("\x1b",.2)

            key(":cd /definitely/not/here\r",.3)
            assert wait_for(lambda: "E344" in text()), ("a missing directory should be an error\n"+text())

            key(f":cd {other_root}\r",.3)
            assert wait_for(lambda: "Switched project root" in text()), ("cd should confirm\n"+text())
            key(":pwd\r",.3)
            assert wait_for(lambda: other_root.name in flat()), (":pwd should show the new directory\n"+text())

            key("\x10",.5)
            key("zzmark",.5)
            assert wait_for(lambda: "zzmarker" in text()), ("picker should rescan at the new root\n"+text())
            assert "old_only" not in text(), ("picker kept the old root's files\n"+text())
            key("\x1b",.2)
            key("\x1b",.2)

            key(":e zzmarker.txt\r",.4)
            assert wait_for(lambda: "marker contents" in text()), ("relative :e should resolve from the new cwd\n"+text())

            key(":cd sub\t",.3)
            assert ":cd subdir/" in text(), ("Tab should complete the directory\n"+text())
            assert "subfile" not in text(), ("Tab should not offer files for :cd\n"+text())
            key("\x15\x1b",.2)  # clear the line, leave the command line

            key(":cd -\r",.3)
            key(":pwd\r",.3)
            assert wait_for(lambda: root.name in flat()), (":cd - should return to the launch directory\n"+text())

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
    print(f":cd PTY passed: {cols}x{rows}")
