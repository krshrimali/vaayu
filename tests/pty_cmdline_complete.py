#!/usr/bin/env python3
"""Ex command-line Tab-completion + wildmenu, through a real PTY: Tab completes
a command name and a file-path argument, showing candidates in a wildmenu row."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-cmdcomplete-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"f.txt"; f.write_text("hello\n")
        (root/"alpha.txt").write_text("a\n")
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
            # Command-name completion: :regi + Tab -> :registers, with wildmenu.
            key(":regi",.2)
            key("\t",.3)
            assert ":registers" in text(), ("Tab should complete the command name\n"+text())
            assert "[registers]" in text(), ("wildmenu should mark the selection\n"+text())
            key("\x1b",.2)        # cancel command line
            # Path-argument completion: :e al + Tab -> :e alpha.txt.
            key(":e al",.2)
            key("\t",.3)
            assert ":e alpha.txt" in text(), ("Tab should complete the file path\n"+text())
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
    print(f"cmdline completion PTY passed: {cols}x{rows}")
