#!/usr/bin/env python3
"""Real mouse support (click positioning, drag-select, scroll) driven by
actual SGR mouse escape sequences over a PTY, not just direct calls into
the handler -- this exercises crossterm's mouse parsing and
EnableMouseCapture end to end."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())

def sgr(button, col, row, release=False):
    # 1-based terminal coordinates, xterm SGR (1006) mouse protocol.
    return f"\x1b[<{button};{col};{row}{'m' if release else 'M'}"

for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-mouse-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        file=root/"f.txt"
        file.write_text("hello world\nsecond line\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(file)])
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
        def save_and_read():
            key(":w\r",.3)
            return file.read_text()
        try:
            drain(.3)
            # Click on 'w' of "world" (gutter 2 + col 6, 0-based -> 1-based
            # SGR column 9, row 1), then x deletes the char under it.
            key(sgr(0, 9, 1), .2)
            key(sgr(0, 9, 1, release=True), .1)
            key("x")
            assert save_and_read().splitlines()[0]=="hello orld", save_and_read()
            # Undo, then drag-select "hello" (cols 1..5 0-based -> SGR 1..5)
            # and delete the selection.
            key("u",.2)
            assert save_and_read().splitlines()[0]=="hello world", save_and_read()
            key(sgr(0, 3, 1), .1)       # press at col 0 (0-based) -> SGR col 3 minus gutter... see below
            key(sgr(32, 7, 1), .1)      # drag with button held to col 4 (0-based)
            key(sgr(0, 7, 1, release=True), .1)
            key("d")
            saved = save_and_read().splitlines()[0]
            assert saved==" world", saved
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
    print(f"Mouse PTY passed: {cols}x{rows}")

for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-mouse-scroll-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        file=root/"f.txt"
        file.write_text("".join(f"line {i}\n" for i in range(200)))
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(file)])
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
        try:
            drain(.3)
            before = screen.display[0]
            assert "line 0" in before, before
            for _ in range(10):
                key(sgr(65, 5, 5), .05)  # scroll down
            drain(.2)
            after = screen.display[0]
            assert "line 0" not in after, ("scroll wheel did not move the view\n"+after)
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
    print(f"Mouse scroll PTY passed: {cols}x{rows}")
