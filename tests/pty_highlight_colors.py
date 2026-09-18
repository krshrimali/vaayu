#!/usr/bin/env python3
"""Highlight color fixes: a Results list's plain (non-cursor) rows must
not force White text on the terminal's own default background (that
renders as invisible white-on-white on a light-background terminal
theme), and a search match must force a readable foreground instead of
leaving whatever arbitrary syntax color the token had -- driven through
a real PTY, inspecting pyte's per-cell fg/bg attributes directly."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-hlcolors-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        f=root/"f.rs"
        f.write_text('fn keyword_match() {\n    let x = 1;\n}\n')
        g=root/"g.txt"; g.write_text("x\n")
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
            drain(.3)
            # Search-match foreground: "keyword_match" is not a real Rust
            # keyword (no syntax color), but the *matched* word must still
            # come out with a readable forced foreground, not left as
            # whatever the underlying (here: default) color was combined
            # with a default background -- i.e. fg must actually change
            # once the DarkYellow highlight background is applied.
            key("/keyword\r",.3)
            assert wait_for(lambda: screen.buffer[0][9].bg not in ("default",)), \
                ("search match should have a highlighted background\n"+text())
            assert screen.buffer[0][9].fg not in ("default",), \
                ("search match should force an explicit readable foreground, "
                 "not leave the default one sitting on a colored background\n"
                 +text())
            key("\x1b",.2)

            # Plain Results-list rows on the terminal's own default
            # background must not force a hardcoded White foreground.
            key(f":e {g}\r",.3)
            key(":ls\r",.4)
            assert wait_for(lambda: "results ·" in text()), \
                ("buffers list never opened\n"+text())
            # Row 2 (index 3 on screen: header, hint, then entries) is a
            # non-cursor row -- its background should be the terminal's
            # own default, and so should its foreground.
            row = 3
            cells = [screen.buffer[row][x] for x in range(0, 6)]
            assert all(c.bg == "default" for c in cells), \
                ("a non-cursor results row should sit on the default "
                 "background\n"+text())
            assert all(c.fg == "default" for c in cells if c.data.strip()), \
                ("a non-cursor results row on the default background must "
                 "not force a White foreground -- invisible on a "
                 "light-background terminal theme\n"+text())
            key("\x1b",.2)
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
    print(f"Highlight colors PTY passed: {cols}x{rows}")
