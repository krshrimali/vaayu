#!/usr/bin/env python3
""":everything combines keymaps, commands and recent projects into one
list, reusing each source's own existing action tags with no new
dispatch logic; f (Results filtering) narrows across all of them at
once -- driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, re, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-everything-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        file=root/"f.txt"; file.write_text("hello\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(file)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.2):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                ready,_,_=select.select([fd],[],[],min(.02,max(0,end-time.monotonic())))
                if ready:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.15):os.write(fd,s.encode());drain(seconds)
        def text():
            return "\n".join(screen.display)
        def wait_for(pred, timeout=5.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():
                    return True
                drain(.1)
            return False
        def total_count():
            m = re.search(r"·\s*(\d+)\s*results", text())
            return int(m.group(1)) if m else None
        try:
            drain(.3)
            key(":everything\r",.4)
            assert wait_for(lambda: "[keymap]" in text() and total_count() is not None), \
                (":everything should open a combined list\n"+text())
            total = total_count()
            key("f",.2)
            key("grep",.3)
            assert wait_for(lambda: ":grep" in text()), \
                ("filtering for grep should surface the :grep command\n"+text())
            m = re.search(r"·\s*(\d+)/(\d+)\s*results", text())
            assert m is not None, ("expected a narrowed N/total count\n"+text())
            assert int(m.group(2)) == total, "the total should match the unfiltered count"
            assert int(m.group(1)) < total, "the filtered count should be smaller"
            key("\n",.2)
            key("\x1b",.2)
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
    print(f"Everything picker PTY passed: {cols}x{rows}")
