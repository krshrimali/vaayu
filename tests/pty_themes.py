#!/usr/bin/env python3
"""Full-palette colorschemes paint the editor background and default text,
`:set transparent` drops just the background, `:colorscheme <Tab>` completes
scheme names, and switching back to `default` leaves the terminal's own
colors again. Driven through a real PTY at several sizes."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
GRUVBOX_BG,GRUVBOX_FG,GRUVBOX_BAR="282828","ebdbb2","076678"
DAY_BG="e1e2e7"
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-themes-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\ncursorline=false\n')
        f=root/"code.rs"; f.write_text("fn main() {}\nplain words here\n")
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
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        def cell(x,y):return screen.buffer[y][x]
        # An empty body cell (past the text, above the status line), a plain
        # text cell ("plain" on line 2) and a status-line cell.
        empty=lambda:cell(cols-2,rows//2)
        text=lambda:cell(2,1)
        status=lambda:cell(1,rows-2)
        def dump():return "\n".join(screen.display)
        try:
            drain(.6)
            assert wait_for(lambda:"plain" in screen.display[1]),"file should open\n"+dump()
            assert empty().bg=="default" and text().fg=="default", \
                ("default scheme keeps the terminal's colors",empty().bg,text().fg)
            # Tab completes the scheme name in the command line.
            key(":colorscheme gru")
            key("\t")
            assert wait_for(lambda:screen.display[rows-1].startswith(":colorscheme gruvbox")), \
                "Tab should complete the colorscheme name\n"+dump()
            key("\r",.5)
            assert wait_for(lambda:empty().bg==GRUVBOX_BG), \
                ("gruvbox should paint the empty background",empty().bg,dump())
            assert text().fg==GRUVBOX_FG and text().bg==GRUVBOX_BG, \
                ("plain text gets the scheme's fg on its bg",text().fg,text().bg)
            assert status().bg==GRUVBOX_BAR,("statusline uses the scheme's bar",status().bg)
            # Every row is painted: the message line too.
            assert cell(cols-1,rows-1).bg==GRUVBOX_BG,("message line bg",cell(cols-1,rows-1).bg)
            key(":set transparent\r",.5)
            assert wait_for(lambda:empty().bg=="default"), \
                ("transparent should drop the background",empty().bg)
            assert text().fg==GRUVBOX_FG,("transparent keeps the foreground",text().fg)
            assert status().bg==GRUVBOX_BAR,"transparent keeps UI bars"
            key(":set notransparent\r",.5)
            assert wait_for(lambda:empty().bg==GRUVBOX_BG),"notransparent restores the background"
            key(":colorscheme tokyonight-day\r",.5)
            assert wait_for(lambda:empty().bg==DAY_BG),("light scheme background",empty().bg)
            key(":colorscheme default\r",.5)
            assert wait_for(lambda:empty().bg=="default" and text().fg=="default"), \
                ("back to default leaves the terminal colors",empty().bg,text().fg)
            assert wait_for(lambda:"00ffff" in [cell(x,0).fg for x in range(cols)]), \
                "default keyword is cyan again"
            key(":qa!\r")
            end=time.monotonic()+3
            while time.monotonic()<end:
                done,status_=os.waitpid(pid,os.WNOHANG)
                if done:
                    assert os.waitstatus_to_exitcode(status_)==0;pid=None;break
                drain(.05)
            assert pid is None,"Editor failed to exit"
        finally:
            if pid:
                os.kill(pid,signal.SIGKILL);os.waitpid(pid,0)
            os.close(fd)
    print(f"themes PTY passed: {cols}x{rows}")
