#!/usr/bin/env python3
"""Minimap: `:set minimap` reserves a strip on the right of the pane showing a
compressed file silhouette (a separator bar + block glyphs), with the rows
covering the current viewport tinted; `:set nominimap` reclaims the width."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
MAP_W=12
# A tall file (so the viewport covers only part of it) with a mix of flush and
# indented lines, and no box-drawing chars of its own.
lines=[]
for i in range(80):
    if i%4==0:   lines.append(f"fn item_{i}() {{")
    elif i%4==1: lines.append("    let value = compute(i);")
    elif i%4==2: lines.append("        return value + 1;")
    else:        lines.append("}")
CONTENT="\n".join(lines)+"\n"
for cols,rows in [(90,24),(120,30),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-mmp-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"code.txt"; f.write_text(CONTENT)  # .txt: no LSP, keeps the test focused
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
        def key(s,seconds=.4):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        region=range(cols-MAP_W,cols)          # the reserved minimap columns
        # Pane content rows only: the pane's own status bar sits at rows-2 (it
        # always has a background) and the global message line at rows-1.
        content_rows=range(0,rows-2)
        def sep_count():
            return sum(1 for y in content_rows for x in region
                       if screen.buffer[y][x].data=="│")
        def block_count():
            return sum(1 for y in content_rows for x in region
                       if screen.buffer[y][x].data=="▪")
        def tinted_rows():
            out=[]
            for y in content_rows:
                if any(screen.buffer[y][x].bg!="default" for x in region):
                    out.append(y)
            return out
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.4)
            key(":set minimap\r",.4)
            assert wait_for(lambda: sep_count()>=3), \
                ("minimap separator bar should appear on the right\n"+text())
            assert block_count()>0, ("minimap should show code blocks\n"+text())
            # Viewport indicator: the top rows (visible lines) are tinted, but
            # not every minimap row is (the file is far taller than the pane).
            tinted=tinted_rows()
            assert len(tinted)>0, ("viewport rows in the minimap should be tinted\n"+text())
            assert len(tinted)<len(list(content_rows)), \
                ("out-of-view minimap rows should stay untinted\n"+text())
            key(":set nominimap\r",.4)
            assert wait_for(lambda: sep_count()==0 and len(tinted_rows())==0), \
                ("nominimap should reclaim the strip\n"+text())
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
    print(f"minimap PTY passed: {cols}x{rows}")
