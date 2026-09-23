#!/usr/bin/env python3
"""Configurable `listchars`: with `list` on and a custom `listchars` string,
tabs and trailing spaces render with the configured glyphs (not the defaults).
PTY-driven."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(80,14),(120,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-lcc-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nnumber=false\ntabstop=4\nexpandtab=false\n'
            'list=true\nlistchars="tab:!~,trail:@"\n')
        f=root/"a.txt"; f.write_text("\tcode here   \nplain\n")
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
        def text():return "\n".join(screen.display)
        def row(y):return "".join(screen.buffer[y][x].data for x in range(cols))
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.5)
            # Leading tab -> '!' lead then '~' fill; trailing spaces -> '@'.
            assert wait_for(lambda: "!" in row(0) and "~" in row(0) and "@" in row(0)), \
                ("custom listchars glyphs should render\n"+text())
            # The default glyphs must not appear on the content row (the help
            # line legitimately uses `·` as a separator, so scope the check).
            assert "·" not in row(0) and ">" not in row(0), \
                ("defaults must be replaced by the configured glyphs\n"+text())
            os.write(fd,b":qa!\r")
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
    print(f"custom listchars PTY passed: {cols}x{rows}")
