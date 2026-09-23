#!/usr/bin/env python3
"""inccommand: while typing `:%s/foo/BAR/g` (before Enter), the affected lines
show a live, tinted preview of the replacement; Esc reverts to the original
text; submitting applies it for real. Driven via a PTY against the binary."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(80,14),(120,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-icmd-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"a.txt"; f.write_text("foo one\nfoo two\nkeep me\n")
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
        def key(s,seconds=.35):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        def row_tinted(y):
            return sum(1 for x in range(cols) if screen.buffer[y][x].bg!="default")>cols//3
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.4)
            assert "ZED" not in text()
            # Type the substitute but DO NOT press Enter yet.
            key(":%s/foo/ZED/g",.5)
            assert wait_for(lambda: "ZED one" in text() and "ZED two" in text()), \
                ("live preview should show the replacement\n"+text())
            # The affected rows are tinted; the unaffected "keep me" line is not.
            assert row_tinted(0) and row_tinted(1), ("preview rows should be tinted\n"+text())
            # Esc reverts: the original text is back and the preview is gone.
            key("\x1b",.4)
            assert wait_for(lambda: "ZED" not in text() and "foo one" in text()), \
                ("Esc should revert the preview\n"+text())
            assert not row_tinted(0), ("preview tint should clear on Esc\n"+text())
            # Submit for real this time.
            key(":%s/foo/ZED/g\r",.4)
            assert wait_for(lambda: "ZED one" in text() and "ZED two" in text()), \
                ("submitting should apply the substitution\n"+text())
            # And it's a committed edit, not a preview tint.
            assert not row_tinted(0), ("committed text is not preview-tinted\n"+text())
            key(":w\r",.4)
            key(":q!\r")
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
        assert f.read_text()=="ZED one\nZED two\nkeep me\n", f.read_text()
    print(f"inccommand PTY passed: {cols}x{rows}")
