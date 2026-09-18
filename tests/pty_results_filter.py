#!/usr/bin/env python3
"""f in a Results list opens a live filter (case-insensitive substring
against each entry's text), narrowing what's displayed without losing
the full backing set -- a fresh f clears the previous filter -- driven
through a real PTY."""
import codecs, fcntl, os, pathlib, pty, re, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-resfilter-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        file=root/"f.txt"; file.write_text("hello\n")
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
        def text():
            return "\n".join(screen.display)
        def wait_for(pred, timeout=5.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():
                    return True
                drain(.1)
            return False
        def total_count():
            m = re.search(r"·\s*(\d+)\s*results", text())
            return int(m.group(1)) if m else None
        def narrowed_count():
            m = re.search(r"·\s*(\d+)/(\d+)\s*results", text())
            return (int(m.group(1)), int(m.group(2))) if m else None
        try:
            drain(.3)
            key(":commands\r",.4)
            assert wait_for(lambda: "results ·" in text()), \
                ("commands list never opened\n"+text())
            total = total_count()
            assert total and total > 1, ("could not read the total count\n"+text())
            key("f",.2)
            assert wait_for(lambda: "Filter:" in text()), \
                ("f should open the filter input\n"+text())
            key("gitblame",.3)
            assert wait_for(lambda: ":gitblame" in text() and narrowed_count() is not None), \
                ("typing a filter should narrow the list and show N/total\n"+text())
            assert narrowed_count()[1] == total and narrowed_count()[0] < total, \
                ("narrowed count should be smaller than the real total\n"+text())
            assert ":gitstash" not in text(), \
                ("non-matching entries should be hidden by the filter\n"+text())
            key("\r",.3)  # leave filter-input, keeping the filter applied
            assert "Filter:" not in text(), \
                ("Enter should leave the filter-input line\n"+text())
            assert narrowed_count() is not None, \
                ("the narrowed count should persist after leaving input mode\n"+text())
            key("f",.2)  # a fresh f clears the previous filter
            assert wait_for(lambda: total_count() == total and narrowed_count() is None), \
                ("a fresh f should clear the previous filter, restoring everything\n"+text())
            key("\r",.2)
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
    print(f"Results filter PTY passed: {cols}x{rows}")
