#!/usr/bin/env python3
"""File tree operations: create (a), rename (r), and the two-press delete
(d, d) with unsaved-buffer protection -- driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-filetree-ops-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-filetree-ops-cfg-") as cfg:
        root=pathlib.Path(proj)
        cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        existing=root/"existing.txt"; existing.write_text("keep me\n")
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
            # Create a new file at the root.
            key("a",.15)
            key("created.txt\r",.3)
            assert (root/"created.txt").exists(), "a + name + Enter did not create the file"
            assert "created.txt" in right_text(), right_text()
            # Rename it.
            key("r",.15)
            key("renamed.txt\r",.3)
            assert not (root/"created.txt").exists()
            assert (root/"renamed.txt").exists(), "rename did not move the file"
            assert "renamed.txt" in right_text(), right_text()
            # Delete requires two presses; a single d must not remove it.
            key("d",.2)
            assert (root/"renamed.txt").exists(), "a single d must only arm delete"
            key("d",.2)
            assert not (root/"renamed.txt").exists(), "second d must delete it"
            # Deleting a file with unsaved changes must be refused.
            # existing.txt is the only remaining node; opening it with 'l'
            # focuses the *other* (buffer) pane automatically.
            key("G",.2)
            key("l",.3)
            key("ihello \x1b",.2)  # dirty the buffer
            # Ctrl-W l: back to the tree pane (opened on the right of the split).
            key("\x17l",.3)
            key("d",.2)
            key("d",.2)
            assert (root/"existing.txt").exists(), \
                ("deleting a file with unsaved changes must be refused\n"+right_text())
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
    print(f"File tree ops PTY passed: {cols}x{rows}")
