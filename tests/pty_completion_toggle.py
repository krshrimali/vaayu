#!/usr/bin/env python3
"""completion_enabled = false suppresses the automatic completion popup
entirely, even with a real buffer-word match available -- driven
through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-compltoggle-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        f=root/"f.txt"; f.write_text("needle\n\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(f)])
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
        try:
            drain(.3)
            # No config.toml at all -- completion_enabled defaults true --
            # so typing a prefix that matches "needle" should pop up "buf".
            key("jo",.2)
            key("nee",.3)
            assert " buf " in text() or "buf needle" in text(), \
                ("completion should pop up by default\n"+text())
            key("\x1b",.2)
            key("dd",.2)  # clear the line for a clean retry
            # Now write a config disabling it, and exit without saving so
            # the next launch of the same binary picks it up fresh --
            # simplest way to change config mid-test in this harness is a
            # fresh process, so relaunch.
            key(":qa!\r")
            end=time.monotonic()+3
            while time.monotonic()<end:
                done,status=os.waitpid(pid,os.WNOHANG)
                if done:
                    pid=None;break
                drain(.05)
            (root/"config/vaayu/config.toml").write_text("completion_enabled=false\n")
            pid,fd=pty.fork()
            if pid==0:
                os.chdir(root)
                os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
                os.execv(binary,[binary,str(f)])
            fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
            screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
            drain(.3)
            key("jo",.2)
            key("nee",.3)
            assert " buf " not in text() and "buf needle" not in text(), \
                ("completion_enabled=false should suppress the popup\n"+text())
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
    print(f"Completion toggle PTY passed: {cols}x{rows}")
