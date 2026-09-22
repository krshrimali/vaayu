#!/usr/bin/env python3
""":reg / :marks / :messages viewers, driven through a real PTY: yanked text
shows in :reg; a set mark shows in :marks and Enter jumps to it; a message
(from `u`) shows in :messages."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-viewers-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        f=root/"f.txt"; f.write_text("alpha\nbeta\ngamma\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(f)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.25):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                ready,_,_=select.select([fd],[],[],min(.02,max(0,end-time.monotonic())))
                if ready:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.2):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        try:
            drain(.4)
            # :reg shows a yanked register.
            key("yy",.2)          # yank "alpha\n" into the unnamed register
            key(":reg\r",.3)
            assert "Registers" in text(), ("reg viewer should open\n"+text())
            assert "alpha" in text(), ("yanked text should appear in :reg\n"+text())
            key("q",.2)           # close the results list

            # :marks lists a set mark; Enter jumps to it.
            key("3G",.2)          # go to line 3 (gamma)
            key("ma",.2)          # set mark a
            key("gg",.2)          # back to line 1
            key(":marks\r",.3)
            assert "Marks" in text(), ("marks viewer should open\n"+text())
            assert "'a" in text(), ("mark a should be listed\n"+text())
            key("\r",.3)          # Enter: jump to mark a (line 3)
            assert "3:1" in text(), ("Enter on a mark should jump to it\n"+text())

            # :messages shows the message history (u -> "undo").
            key("u",.2)
            key(":messages\r",.3)
            assert "Messages" in text(), ("messages viewer should open\n"+text())
            assert "undo" in text(), ("a shown message should be in :messages\n"+text())
            key("q",.2)

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
    print(f"viewers PTY passed: {cols}x{rows}")
