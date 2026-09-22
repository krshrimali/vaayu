#!/usr/bin/env python3
"""The status bar re-aligns to the terminal width after a resize (what a font
zoom does), including when the resize is picked up by the per-frame size poll
rather than only a Resize event. Driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix="vaayu-resize-") as tmp:
    root=pathlib.Path(tmp)
    (root/"config/vaayu").mkdir(parents=True)
    (root/"config/vaayu/config.toml").write_text("jk_escape=false\n")
    f=root/"hello.txt"; f.write_text("line one\nline two\n")
    pid,fd=pty.fork()
    if pid==0:
        os.chdir(root)
        os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
        os.execv(binary,[binary,str(f)])
    screen=pyte.Screen(80,24);stream=pyte.Stream(screen);dec=codecs.getincrementaldecoder("utf-8")("replace")
    def setsize(cols,rows):
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen.resize(rows,cols)
    def drain(sec=.5):
        end=time.monotonic()+sec
        while time.monotonic()<end:
            r,_,_=select.select([fd],[],[],min(.02,max(0,end-time.monotonic())))
            if r:
                try:data=os.read(fd,65536)
                except OSError:break
                if not data:break
                stream.feed(dec.decode(data))
    def status_row(cols):
        disp=screen.display
        row=next((r for r in disp if "NORMAL" in r), None)
        assert row is not None, "no status bar found\n"+"\n".join(disp)
        # Spans the full width, and the cursor position sits flush-right.
        assert len(row)==cols, f"status width {len(row)} != {cols}\n|{row}|"
        assert row.rstrip().endswith("1:1"), f"cursor pos not flush-right\n|{row}|"
    setsize(80,24); drain(.5); status_row(80)
    for c,r in [(120,32),(52,16),(100,28),(140,40)]:
        setsize(c,r); drain(.6); status_row(c)
    os.write(fd,b":qa!\r"); drain(.3)
    end=time.monotonic()+3
    while time.monotonic()<end:
        done,status=os.waitpid(pid,os.WNOHANG)
        if done:
            assert os.waitstatus_to_exitcode(status)==0;pid=None;break
        drain(.05)
    if pid:os.kill(pid,9);os.waitpid(pid,0)
    os.close(fd)
print("resize status-bar alignment PTY passed")
