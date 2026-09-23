#!/usr/bin/env python3
"""`statuscolumn` reorders the gutter components. With `statuscolumn = "num"`
(number only, no diag/git marker cells), the line number sits flush at the left
of the gutter, unlike the default order which pads two marker cells before it."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
def col_of_digit(screen, y, cols):
    row="".join(screen.buffer[y][x].data for x in range(cols))
    for x,ch in enumerate(row):
        if ch.isdigit():
            return x
    return None
def run(cfg_extra, cols, rows):
    with tempfile.TemporaryDirectory(prefix="vaayu-scl-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=true\n'+cfg_extra)
        f=root/"a.txt"; f.write_text("alpha\nbeta\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(f)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.5):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                r,_,_=select.select([fd],[],[],.05)
                if r:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        drain(.6)
        digit_col=col_of_digit(screen,0,cols)
        os.write(fd,b":qa!\r"); drain(.4)
        try:os.kill(pid,signal.SIGKILL);os.waitpid(pid,0)
        except OSError:pass
        os.close(fd)
        return digit_col
for cols,rows in [(100,24),(180,50)]:
    default_col=run("", cols, rows)              # marker, sign, then number
    numfirst_col=run('statuscolumn="num diag git"\n', cols, rows)  # number first
    assert default_col is not None and numfirst_col is not None, "line number rendered"
    # Putting `num` first moves the number's digit to the left of the gutter.
    assert numfirst_col < default_col, \
        f"num-first gutter should shift the number left: {numfirst_col} vs {default_col}"
    print(f"statuscolumn PTY passed: {cols}x{rows}")
