#!/usr/bin/env python3
"""File tree sidebar (,ft): toggle open/close, reveal the current file,
expand/collapse directories, and open a file into the adjacent pane --
driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-filetree-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-filetree-cfg-") as cfg:
        root=pathlib.Path(proj)
        cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\nlsp.rust_analyzer.enabled=false\n'
        )
        (root/"src").mkdir()
        main_rs=root/"src"/"main.rs"; main_rs.write_text("fn main() {}\n")
        readme=root/"README.md"; readme.write_text("hello readme\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(cfg))
            os.execv(binary,[binary,str(main_rs)])
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
            # The tree opens as the right pane of a vertical split: only
            # look at the right half of the screen, so the left pane's
            # status line/buffer content (which also mentions the open
            # file's name) can never be mistaken for the tree's own state.
            half = cols // 2
            return "\n".join(row[half:] for row in screen.display)
        try:
            drain(.3)
            # Opening the tree reveals the currently open file (main.rs
            # inside src/, which must already be expanded and selected).
            key(",ft",.3)
            assert "src" in right_text() and "main.rs" in right_text(), \
                ("tree did not reveal the open file\n"+text())
            assert "README.md" in right_text(), ("tree missing root-level file\n"+text())
            # Cursor starts on main.rs (revealed); h on a file jumps to its
            # parent dir, h again (now on the dir itself) collapses it.
            key("h",.2)
            key("h",.2)
            assert "main.rs" not in right_text(), \
                ("h did not collapse the directory\n"+text())
            key("l",.2)
            assert "main.rs" in right_text(), \
                ("l did not re-expand the directory\n"+text())
            # README.md sorts last (directories first, then alphabetical),
            # so G jumps straight to it; Enter opens it into the other pane.
            key("G",.2)
            key("\r",.3)
            assert "hello readme" in text(), \
                ("opening a file from the tree did not load it\n"+text())
            # Toggling again closes the sidebar pane.
            key(",ft",.3)
            assert "hello readme" in text(), text()
            key(":qa\r")
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
    print(f"File tree PTY passed: {cols}x{rows}")
