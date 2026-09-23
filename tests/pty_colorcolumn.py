#!/usr/bin/env python3
"""`:set colorcolumn=N` draws a vertical ruler: exactly one tinted cell per row
at that column, both over text and past end-of-line, and gone when disabled.
Driven through a real PTY, asserting on cell backgrounds."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-cc-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        # number=false so buffer column 0 maps to a stable screen gutter offset.
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"f.txt"; f.write_text("abcdefghij\nshort\n\nx\n")
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
        def text():return "\n".join(screen.display)
        # Count tinted (non-default bg) cells in a buffer row.
        def tinted_cells(y):
            return [x for x in range(cols) if screen.buffer[y][x].bg!="default"]
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.4)
            assert "abcdefghij" in text(), ("initial content\n"+text())
            assert tinted_cells(0)==[], ("no ruler by default\n"+text())
            key(":set colorcolumn=5\r",.3)
            # Row 0 (long line): exactly one tinted cell.
            assert wait_for(lambda: len(tinted_cells(0))==1), \
                ("row 0 should have exactly one ruler cell\n"+text())
            col0=tinted_cells(0)[0]
            # Row 1 ("short", 5 chars): ruler still shows past end-of-line, same column.
            assert wait_for(lambda: tinted_cells(1)==[col0]), \
                ("ruler should appear past EOL at the same column\n"+text())
            # Row 2 (empty line): ruler still at that column.
            assert tinted_cells(2)==[col0], ("ruler on an empty line too\n"+text())
            key(":set cc=0\r",.3)
            assert wait_for(lambda: tinted_cells(0)==[] and tinted_cells(1)==[]), \
                ("cc=0 should remove the ruler\n"+text())
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
    print(f"colorcolumn PTY passed: {cols}x{rows}")
