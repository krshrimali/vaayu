#!/usr/bin/env python3
"""`:set list` reveals a leading tab as `>` + `-` fill and trailing spaces as
`·`; `:set nolist` restores the plain view. Driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-list-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        # tabstop 4 so a leading tab renders as 4 cells; number=false for a
        # stable gutter. expandtab off so the literal tab in the file survives.
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nnumber=false\ntabstop=4\nexpandtab=false\n')
        f=root/"f.txt"; f.write_text("\tcode\ntrailing   \nplain line\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(f)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.3):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                ready,_,_=select.select([fd],[],[],min(.02,max(0,end-time.monotonic())))
                if ready:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.3):os.write(fd,s.encode());drain(seconds)
        def row(y):return screen.display[y]
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.4)
            # Off by default: no tab markers, no dot glyphs.
            assert ">" not in row(0) and "·" not in row(1), ("list off by default\n"+"\n".join(screen.display))
            key(":set list\r",.3)
            # Row 0: leading tab becomes ">" then "-" fill before "code"
            # (after the gutter columns), e.g. "  >---code".
            assert wait_for(lambda: ">" in row(0) and "---" in row(0) and "code" in row(0)), \
                ("leading tab should show >--- lead+fill\n"+"\n".join(screen.display))
            assert row(0).index(">") < row(0).index("code"), \
                ("tab markers precede the content\n"+row(0))
            # Row 1: trailing spaces become dots.
            assert wait_for(lambda: "·" in row(1)), \
                ("trailing whitespace should show dots\n"+"\n".join(screen.display))
            # Row 2: no trailing ws / tabs -> unchanged, no dots.
            assert "·" not in row(2), ("clean line unaffected\n"+row(2))
            key(":set nolist\r",.3)
            assert wait_for(lambda: ">" not in row(0) and "·" not in row(1)), \
                ("nolist restores plain view\n"+"\n".join(screen.display))
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
    print(f"listchars PTY passed: {cols}x{rows}")
