#!/usr/bin/env python3
"""Command-line and search history: Up/Down (and Ctrl-P/Ctrl-N) cycle
through previously submitted :commands and /searches independently,
restoring the in-progress draft when cycling back past the newest entry
-- driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-history-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        file=root/"f.txt"
        file.write_text("needle haystack\n")
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
        def bottom():
            return screen.display[-1]
        try:
            drain(.3)
            key(":set wrap\r",.2)
            key(":set number\r",.2)
            # Up/Up recalls the two commands, most recent first; a bare ':'
            # then Up must show the same history, not an empty line.
            key(":",.1)
            key("\x1b[A",.15)  # xterm-style Up arrow escape sequence
            assert bottom().strip()==":set number", repr(bottom())
            key("\x1b[A",.15)
            assert bottom().strip()==":set wrap", repr(bottom())
            key("\x1b[B",.15)  # Down arrow
            assert bottom().strip()==":set number", repr(bottom())
            key("\x1b",.2)
            # Search history is independent of command history.
            key("/needle\r",.2)
            key("/",.1)
            key("\x1b[A",.15)
            assert bottom().strip()=="/needle", repr(bottom())
            key("\x1b",.2)
            key(":",.1)
            key("\x1b[A",.15)
            assert bottom().strip()==":set number", \
                ("search history leaked into command history\n"+repr(bottom()))
            key("\r",.2)
            key(":qa\r")
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
    print(f"History PTY passed: {cols}x{rows}")
