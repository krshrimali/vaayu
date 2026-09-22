#!/usr/bin/env python3
"""The file-tree sidebar (,ft) scrolls: j/k move the cursor and the viewport
follows it, and the mouse wheel scrolls the pane -- so a file list taller
than the pane is fully reachable. Driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())

def sgr(button, col, row):  # xterm SGR (1006) mouse press, 1-based coords
    return f"\x1b[<{button};{col};{row}M"

N=80  # more nodes than any tested pane height, so scrolling always matters
cfg=tempfile.mkdtemp(prefix="vaayu-cfg-")            # config OUTSIDE the project
pathlib.Path(cfg,"vaayu").mkdir(parents=True)
pathlib.Path(cfg,"vaayu/config.toml").write_text("jk_escape=false\nnumber=false\n")
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-treescroll-") as tmp:
        root=pathlib.Path(tmp)
        for i in range(N):
            (root/f"f{i:02d}.txt").write_text("x\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=cfg)
            os.execv(binary,[binary,str(root/"f00.txt")])
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
        def key(s,seconds=.12):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        # Only look at the tree pane (right of the split separator), so the
        # open file's name in the buffer's status line can't be mistaken for a
        # visible tree node.
        def tree_col():return "\n".join(r.split("│")[-1] for r in screen.display)
        def shows(name):return f"{name}.txt" in tree_col()
        col=cols-5  # a column inside the right-hand tree pane
        try:
            drain(.4)
            key(",ft",.5)
            assert shows("f00"), ("tree should open showing the first files\n"+text())
            assert not shows("f79"), ("the last file can't be on the first screen\n"+text())
            # Keyboard: move the cursor to the bottom (Ctrl-D jumps 10 at a time)
            # and the viewport must follow so the last files become visible.
            for _ in range(12):
                key("\x04",.05)   # Ctrl-D
            drain(.3)
            assert shows("f79"), ("j/Ctrl-D must scroll the view to the last file\n"+text())
            assert not shows("f00"), ("the view should have scrolled off the top\n"+text())
            # Mouse wheel up scrolls back toward the top.
            for _ in range(30):
                os.write(fd,sgr(64,col,4).encode());drain(.02)   # wheel up
            drain(.3)
            assert shows("f00"), ("mouse wheel up should scroll back to the top\n"+text())
            # Mouse wheel down scrolls into the middle of the list.
            for _ in range(10):
                os.write(fd,sgr(65,col,4).encode());drain(.02)   # wheel down
            drain(.3)
            assert shows("f30"), ("mouse wheel down should scroll into the list\n"+text())
            key(":qa!\r",.3)
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
    print(f"file-tree scroll PTY passed: {cols}x{rows}")
import shutil;shutil.rmtree(cfg,ignore_errors=True)
