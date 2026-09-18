#!/usr/bin/env python3
""":projects lists recently-launched-from directories (excluding the
current one) from a small persisted state file; Enter switches
project_root and drops any open file tree so a later ,ft rebuilds it at
the new root -- driven through a real PTY."""
import codecs, fcntl, json, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-projpicker-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-projpicker-other-") as other, \
         tempfile.TemporaryDirectory(prefix="vaayu-projpicker-cfg-") as cfg:
        root=pathlib.Path(proj)
        other_root=pathlib.Path(other)
        cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        # Pre-seed the recent-projects state file the same shape
        # projects::record_recent_project would have written.
        (cfg/"vaayu/recent_projects.json").write_text(json.dumps([str(other_root), str(root)]))
        f=root/"f.txt"; f.write_text("hello\n")
        (other_root/"unique_marker.txt").write_text("x\n")
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
        def wait_for(pred, timeout=5.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():
                    return True
                drain(.1)
            return False
        try:
            drain(.3)
            key(":projects\r",.4)
            assert wait_for(lambda: str(other_root) in text()), \
                ("projects list should show the other recent directory\n"+text())
            assert str(root) not in text(), \
                ("the current project root should be excluded from the list\n"+text())
            key("\r",.3)  # switch to it
            assert wait_for(lambda: "Switched project root" in text()), \
                ("switching should show a confirmation message\n"+text())
            key(",ft",.4)
            assert wait_for(lambda: "unique_marker.txt" in text()), \
                ("the file tree should rebuild at the new project root\n"+text())
            key(",ft",.2)
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
    print(f"Projects picker PTY passed: {cols}x{rows}")
