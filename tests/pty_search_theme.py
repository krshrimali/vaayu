#!/usr/bin/env python3
"""The search-match highlight color is theme-driven: switching colorscheme
recolors active search matches. `/target` highlights, then `:colorscheme cool`
repaints the matches in the cool scheme's search background (78c8dc)."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(80,24),(120,40)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-st-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"a.txt"; f.write_text("aa target bb\ncc target dd\n")
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
                r,_,_=select.select([fd],[],[],.05)
                if r:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.3):os.write(fd,s.encode());drain(seconds)
        def match_bg():
            # Background of the first char of a highlighted "target" occurrence.
            for y in range(rows):
                row="".join(screen.buffer[y][x].data for x in range(cols))
                idx=row.find("target")
                if idx!=-1:
                    return screen.buffer[y][idx].bg
            return None
        def wait_for(pred,timeout=4.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.1)
            return False
        try:
            drain(.5)
            key("/target\r",.4)   # highlight matches
            default_bg=match_bg()
            assert default_bg not in (None,"default"), \
                (f"search match should be highlighted, got {default_bg}\n"+"\n".join(screen.display[:3]))
            key(":colorscheme cool\r",.5)
            assert wait_for(lambda: match_bg()=="78c8dc"), \
                (f"cool scheme should recolor the search highlight, got {match_bg()}\n"+"\n".join(screen.display[:3]))
            assert default_bg!="78c8dc", "the default and cool search colors should differ"
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
    print(f"search theme PTY passed: {cols}x{rows}")
