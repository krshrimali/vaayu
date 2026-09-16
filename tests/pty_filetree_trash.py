#!/usr/bin/env python3
"""File tree: t/t moves a node into .vaayu/trash/ (a reversible
alternative to d/d's real delete), refusing under the same
dirty-buffer condition -- driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-filetree-trash-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-filetree-trash-cfg-") as cfg:
        root=pathlib.Path(proj)
        cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        victim=root/"victim.txt"; victim.write_text("keepsake\n")
        existing=root/"existing.txt"; existing.write_text("anchor\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(cfg))
            os.execv(binary,[binary,str(existing)])
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
            assert "victim.txt" in right_text(), ("tree missing victim.txt\n"+right_text())
            # G goes to the last (alphabetically: victim.txt, since
            # directories sort first and these are both plain files
            # sorted alphabetically -- victim.txt sorts after existing.txt).
            key("G",.2)
            key("t",.2)
            assert victim.exists(), "a single t must only arm trash"
            key("t",.2)
            assert not victim.exists(), "second t must move the file out of place"
            trash_dir = root/".vaayu"/"trash"
            trashed = list(trash_dir.iterdir())
            assert len(trashed) == 1, ("expected exactly one trashed file\n"+str(trashed))
            assert trashed[0].name.endswith("-victim.txt")
            assert trashed[0].read_text() == "keepsake\n", "trash must preserve file content"
            assert "victim.txt" not in right_text(), \
                ("trashed file must disappear from the tree\n"+right_text())
            # Trashing a file with unsaved changes must be refused.
            key("gg",.2)  # existing.txt, the only remaining node
            key("l",.3)   # open it into the buffer pane
            key("idirty \x1b",.2)
            key("\x17l",.3)  # back to the tree pane
            key("t",.2)
            key("t",.2)
            assert existing.exists(), \
                ("trashing a file with unsaved changes must be refused\n"+right_text())
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
    print(f"File tree trash PTY passed: {cols}x{rows}")
