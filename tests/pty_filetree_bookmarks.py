#!/usr/bin/env python3
"""File tree: m toggles a bookmark (shown with a star marker),
:treebookmarks lists them, and Enter on a file opens it while Enter on
a directory reveals it in the tree -- driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-filetree-bm-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-filetree-bm-cfg-") as cfg:
        root=pathlib.Path(proj)
        cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        (root/"marked_dir").mkdir()
        starter=root/"starter.txt"; starter.write_text("start here\n")
        target=root/"target_file.txt"; target.write_text("unique_target_content\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(cfg))
            os.execv(binary,[binary,str(starter)])
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
        def right_text():
            half = cols // 2
            return "\n".join(row[half:] for row in screen.display)
        try:
            drain(.3)
            key(",ft",.3)
            # nodes: marked_dir (dir, sorts first), starter.txt (revealed --
            # the currently open file, so the cursor starts here, not at
            # index 0), target_file.txt.
            key("k",.2)  # up onto marked_dir
            key("m",.2)  # bookmark marked_dir
            assert "★" in right_text(), ("m should mark the node with a star\n"+right_text())
            key("j",.2); key("j",.2)  # marked_dir -> starter.txt -> target_file.txt
            key("m",.2)  # bookmark target_file.txt too
            key(":treebookmarks\r",.3)
            assert "marked_dir" in text() and "target_file.txt" in text(), \
                (":treebookmarks should list both bookmarks\n"+text())
            # Bookmarks are ordered by path ("marked_dir" < "target_file.txt"),
            # so entries[0] is the directory, entries[1] the file.
            key("j",.2)  # onto the target_file.txt entry
            key("\r",.3)  # open it
            # "unique_target" (not the full "unique_target_content") --
            # at 40 columns the word soft-wraps across two display rows,
            # and pyte's per-row text doesn't preserve that continuity.
            assert "unique_target" in text(), \
                ("Enter on a file bookmark should open it\n"+text())
            # Now the directory bookmark: reveals in the tree instead of
            # trying (and failing) to open it as a buffer.
            key(":treebookmarks\r",.3)
            key("\r",.3)  # entries[0]: marked_dir
            assert "marked_dir" in right_text(), \
                ("Enter on a directory bookmark should reveal it in the tree\n"+right_text())
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
    print(f"File tree bookmarks PTY passed: {cols}x{rows}")
