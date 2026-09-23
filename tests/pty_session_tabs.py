#!/usr/bin/env python3
"""Multi-tab session: `:sessionsave` with two tabs, restart, `:sessionload`
restores both tabs (and the active one). Driven end-to-end via a PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
def run(root, cfg, cols, rows, script):
    pid,fd=pty.fork()
    if pid==0:
        os.chdir(root)
        os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(cfg))
        os.execv(binary,[binary,str(root/"a.txt")])
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
    drain(.5)
    text=script(fd,drain,screen)
    end=time.monotonic()+3
    global _pid; ok=False
    while time.monotonic()<end:
        done,status=os.waitpid(pid,os.WNOHANG)
        if done: ok=(os.waitstatus_to_exitcode(status)==0); break
        drain(.05)
    if not ok:
        try:os.kill(pid,signal.SIGKILL);os.waitpid(pid,0)
        except OSError:pass
    os.close(fd)
    return text,ok

for cols,rows in [(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-st-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-st-cfg-") as cfgd:
        root=pathlib.Path(proj); cfg=pathlib.Path(cfgd)
        (cfg/"vaayu").mkdir(parents=True)
        (cfg/"vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        (root/"a.txt").write_text("ALPHA content\n")
        (root/"b.txt").write_text("BETA content\n")
        # Session 1: open a, new tab, open b, save session, quit.
        def build(fd,drain,screen):
            os.write(fd,b":tabnew\r");drain(.4)
            os.write(fd,b":e b.txt\r");drain(.4)
            os.write(fd,b":sessionsave\r");drain(.4)
            os.write(fd,b":qa!\r")
            return "\n".join(screen.display)
        run(root,cfg,cols,rows,build)
        assert (root/".vaayu/session.json").exists(), "session file written"
        # Session 2: fresh start, load session, expect two tabs (tabline shows both).
        def restore(fd,drain,screen):
            os.write(fd,b":sessionload\r");drain(.6)
            txt="\n".join(screen.display)
            # The active (2nd) tab shows b.txt.
            assert "BETA content" in txt, ("restored active tab shows b.txt\n"+txt)
            # Switch to the previous tab -> a.txt.
            os.write(fd,b"gT");drain(.4)
            txt2="\n".join(screen.display)
            assert "ALPHA content" in txt2, ("first tab shows a.txt\n"+txt2)
            os.write(fd,b":qa!\r")
            return txt2
        _,ok=run(root,cfg,cols,rows,restore)
        assert ok,"editor exited cleanly after restore"
    print(f"session tabs PTY passed: {cols}x{rows}")
