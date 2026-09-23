#!/usr/bin/env python3
"""Dragging a vertical split's divider with the mouse resizes the panes.
Sends SGR mouse press/drag/release on the `│` column and asserts it moves. PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(100,24),(140,40)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-rs-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        a=root/"a.txt"; a.write_text("".join(f"aaaa {i}\n" for i in range(40)))
        b=root/"b.txt"; b.write_text("".join(f"bbbb {i}\n" for i in range(40)))
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(a)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.3):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                r,_,_=select.select([fd],[],[],.05)
                if r:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.3):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        row=rows//2
        def divider_col():
            for x in range(cols):
                if screen.buffer[row][x].data=="│":
                    return x
            return None
        def mouse(cb,x,y,press=True):
            # SGR mouse: ESC [ < Cb ; Cx ; Cy (M press/motion | m release), 1-based.
            os.write(fd,f"\x1b[<{cb};{x+1};{y+1}{'M' if press else 'm'}".encode())
        try:
            drain(.5)
            key(":vsplit b.txt\r",.6)
            col0=divider_col()
            assert col0 is not None, ("a vertical divider should be drawn\n"+text())
            target=col0-15
            assert target>2, "geometry too small for this test"
            mouse(0,col0,row)            # press left on the divider
            drain(.1)
            for cx in range(col0-1,target-1,-2):
                mouse(32,cx,row)         # drag left (motion bit set)
                drain(.03)
            mouse(0,target,row,press=False)  # release
            drain(.4)
            col1=divider_col()
            assert col1 is not None, ("the divider is still drawn after resize\n"+text())
            assert col1<col0, (f"dragging left should move the divider left ({col1} !< {col0})\n"+text())
            assert abs(col1-target)<=2, (f"divider should land near the drop point (got {col1}, wanted ~{target})\n"+text())
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
    print(f"split resize PTY passed: {cols}x{rows}")
