#!/usr/bin/env python3
"""Winbar (`:set winbar`): a per-pane top row showing the file's relative path
and, inside a function, the enclosing declaration as a breadcrumb. It shifts
content down one row; `:set nowinbar` reclaims it. Driven via a PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
CONTENT="fn compute(x: i32) -> i32 {\n    let y = x + 1;\n    y * 2\n}\n"
for cols,rows in [(80,14),(120,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-winbar-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"widget.rs"; f.write_text(CONTENT)
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
        def row(y):return "".join(screen.buffer[y][x].data for x in range(cols))
        def text():return "\n".join(screen.display)
        def row_has_bg(y):
            return sum(1 for x in range(cols) if screen.buffer[y][x].bg!="default")>cols//2
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.5)
            # Without a winbar, the function is on the top row.
            assert "fn compute" in row(0), ("expected content on row 0\n"+text())
            key(":set winbar\r",.5)
            # Row 0 becomes the winbar (path + tint); content shifts to row 1.
            assert wait_for(lambda: "widget.rs" in row(0) and row_has_bg(0)), \
                ("winbar row should show the path with a background\n"+text())
            assert wait_for(lambda: "fn compute" in row(1)), \
                ("content should shift down under the winbar\n"+text())
            # Move into the function body: the breadcrumb names the enclosing fn.
            key("jj",.4)
            assert wait_for(lambda: "›" in row(0) and "compute" in row(0)), \
                ("winbar should show the enclosing-symbol breadcrumb\n"+text())
            # Turn it off: content returns to the top row, no winbar tint.
            key(":set nowinbar\r",.5)
            assert wait_for(lambda: "fn compute" in row(0) and not row_has_bg(0)), \
                ("nowinbar should reclaim the top row\n"+text())
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
    print(f"winbar PTY passed: {cols}x{rows}")
