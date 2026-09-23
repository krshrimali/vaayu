#!/usr/bin/env python3
"""Folding: `:{range}fold` collapses a line range to a tinted foldtext row
(first line + hidden-line count), hiding the inner lines; `za` toggles it open
(inner lines return) and closed again. Driven via a PTY against the binary."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
CONTENT="AAA0\nBBB1\nCCC2\nDDD3\nEEE4\nFFF5\nGGG6\nHHH7\n"
for cols,rows in [(80,14),(120,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-fold-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"a.txt"; f.write_text(CONTENT)
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
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.4)
            assert "DDD3" in text() and "EEE4" in text()
            # Fold 1-based lines 3..6 (CCC2..FFF5).
            key(":3,6fold\r",.5)
            assert wait_for(lambda: "⋯" in text() and "4 lines" in text()), \
                ("closed fold should show a foldtext summary\n"+text())
            # The fold's first line stays; the inner lines are hidden.
            assert "CCC2" in text(), ("fold start line stays visible\n"+text())
            assert "DDD3" not in text() and "EEE4" not in text() and "FFF5" not in text(), \
                ("inner fold lines should be hidden\n"+text())
            # Lines after the fold are still shown (pulled up).
            assert "GGG6" in text() and "HHH7" in text(), \
                ("lines after the fold remain visible\n"+text())
            # za opens the fold (cursor is on the fold start): inner lines return.
            key("za",.4)
            assert wait_for(lambda: "DDD3" in text() and "FFF5" in text()), \
                ("za should reopen the fold\n"+text())
            assert "⋯" not in text(), ("an open fold shows no foldtext\n"+text())
            # za closes it again.
            key("za",.4)
            assert wait_for(lambda: "⋯" in text() and "DDD3" not in text()), \
                ("za should re-close the fold\n"+text())
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
    print(f"folding PTY passed: {cols}x{rows}")
