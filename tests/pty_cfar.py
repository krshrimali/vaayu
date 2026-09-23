#!/usr/bin/env python3
"""Project-wide find & replace (`:cfar`): a live `:grep` builds a results list
spanning several files, then `:cfar/pat/repl/g` rewrites every one of them on
disk and refocuses the original buffer. Driven via a PTY against the binary."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(90,24),(120,30),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-cfar-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        files={"a.txt":"old alpha\nold beta\n",
               "b.txt":"gamma old\n",
               "c.txt":"no match here\n"}   # in the tree but not in the grep list
        for n,c in files.items():
            (root/n).write_text(c)
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
        def key(s,seconds=.5):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        def wait_for(pred,timeout=4.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.5)
            key(":grep old\r",1.2)
            # The grep list should span both matching files.
            assert wait_for(lambda: "a.txt" in text() and "b.txt" in text()), \
                ("grep should list both matching files\n"+text())
            key("\x1b",.4)                 # leave the results list
            key(":cfar/old/new/g\r",1.0)
            assert wait_for(lambda: "replaced in 2" in text()), \
                ("cfar should report replacing in 2 files\n"+text())
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
        # On-disk assertions: every match rewritten, the non-listed file intact.
        assert (root/"a.txt").read_text()=="new alpha\nnew beta\n", (root/"a.txt").read_text()
        assert (root/"b.txt").read_text()=="gamma new\n", (root/"b.txt").read_text()
        assert (root/"c.txt").read_text()=="no match here\n", (root/"c.txt").read_text()
    print(f"cfar PTY passed: {cols}x{rows}")
