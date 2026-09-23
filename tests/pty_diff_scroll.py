#!/usr/bin/env python3
"""Diff-mode scrollbind: two `:diffthis` buffers shown side by side scroll
together. Scrolling the active pane with Ctrl-e must move the *other* pane to
the same top line (without sync the inactive pane would stay at line 0). PTY."""
import codecs, fcntl, os, pathlib, pty, re, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
A_RE=re.compile(r"a line (\d+)")
B_RE=re.compile(r"b line (\d+)")
for cols,rows in [(120,24),(160,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-ds-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        a=root/"a.txt"; a.write_text("".join(f"a line {i}\n" for i in range(80)))
        b=root/"b.txt"; b.write_text("".join(f"b line {i}\n" for i in range(80)))
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(a)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.35):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                r,_,_=select.select([fd],[],[],.05)
                if r:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.35):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        def min_line(rx):
            ns=[int(m) for m in rx.findall(text())]
            return min(ns) if ns else None
        try:
            drain(.5)
            key(":diffthis\r",.4)          # mark buffer A
            key(":vsplit b.txt\r",.6)      # open B in a second pane (now active)
            key(":diffthis\r",.4)          # mark buffer B -> diff mode on
            # Both panes start at line 0.
            assert min_line(A_RE)==0 and min_line(B_RE)==0, \
                ("both panes should start at the top\n"+text())
            # Scroll the active pane (B) down; A must follow via scrollbind.
            for _ in range(15):
                key("\x05",.03)            # Ctrl-e
            drain(.4)
            ma,mb=min_line(A_RE),min_line(B_RE)
            assert ma is not None and mb is not None, ("both panes still visible\n"+text())
            assert mb>0, ("the active pane should have scrolled\n"+text())
            assert ma==mb, (f"panes out of sync: A top={ma} B top={mb}\n"+text())
            key(":diffoff\r",.3)
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
    print(f"diff scrollbind PTY passed: {cols}x{rows}")
