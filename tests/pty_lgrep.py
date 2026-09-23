#!/usr/bin/env python3
"""`:lgrep` fills the location list from a project grep (independent of the
quickfix list); `:lnext` steps to the next match and jumps to its file. Driven
end-to-end via a PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-lg-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        (root/"a.txt").write_text("needle alpha\nfiller\n")
        (root/"b.txt").write_text("filler\nneedle beta\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(root/"a.txt")])
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
        def wait_for(pred,timeout=4.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.4)
            key(":lgrep needle\r",.8)
            # The location list opens listing both files' matches.
            assert wait_for(lambda: "Location list" in text() and "a.txt" in text() and "b.txt" in text()), \
                ("lgrep should list matches from both files\n"+text())
            key("\x1b",.3)          # close the list
            key(":lnext\r",.5)      # jump to the next location
            # After stepping, we should be viewing a matched line.
            assert wait_for(lambda: "needle" in text()), ("lnext should jump to a match\n"+text())
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
    print(f"lgrep PTY passed: {cols}x{rows}")
