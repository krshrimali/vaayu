#!/usr/bin/env python3
"""incsearch: typing `/pat` live-previews the match (scrolls the view to it),
Esc restores the original view, and <CR> lands on it. A neighbour marker only
visible when the view scrolls to the match distinguishes a real preview from
the search prompt's own echo. Driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-incsearch-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\nscrolloff=0\n'
        )
        # A file taller than any tested viewport, with the match far down and a
        # unique neighbour on the line above it.
        lines=[f"row{i:03d}" for i in range(120)]
        lines[100]="NEIGHBOURMARK"
        lines[101]="UNIQUEHIT"
        f=root/"f.txt"; f.write_text("\n".join(lines)+"\n")
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
        def text():return "\n".join(screen.display)
        try:
            drain(.4)
            assert "row000" in text() and "NEIGHBOURMARK" not in text(), \
                ("view should start at the top\n"+text())
            # Type the query WITHOUT Enter: incsearch scrolls to preview the
            # match, so the neighbour line (never in the prompt) becomes visible.
            key("/UNIQUEHIT",.4)
            assert "NEIGHBOURMARK" in text(), \
                ("incsearch should scroll the view to preview the match\n"+text())
            # Esc restores the original view.
            key("\x1b",.3)
            assert "NEIGHBOURMARK" not in text() and "row000" in text(), \
                ("Esc should restore the pre-search view\n"+text())
            # Submitting lands on the match (neighbour visible again).
            key("/UNIQUEHIT\r",.4)
            assert "NEIGHBOURMARK" in text(), \
                ("submitting the search should land on the match\n"+text())
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
    print(f"incsearch PTY passed: {cols}x{rows}")
