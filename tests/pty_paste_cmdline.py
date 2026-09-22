#!/usr/bin/env python3
"""A bracketed paste (what Ctrl+Shift+V sends) while typing an Ex command
like `:e <path>` must land in the command line, not the editor buffer --
driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
PATH="/tmp/some/file.rs"
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-paste-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        f=root/"f.txt"; f.write_text("hello world\n")
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
            drain(.3)
            # Enter Ex command mode and start `:e `.
            key(":e ",.3)
            assert "COMMAND" in text(), ("expected command mode\n"+text())
            # Deliver the path exactly as a terminal paste (Ctrl+Shift+V) does:
            # wrapped in bracketed-paste markers, with a trailing newline that
            # must NOT submit the command.
            os.write(fd,("\x1b[200~"+PATH+"\x1b[201~\n").encode());drain(.4)
            scr=text()
            # The path is on the command line...
            assert (":e "+PATH) in scr, ("paste should append to the command line\n"+scr)
            # ...still in command mode (the pasted newline did not run it)...
            assert "COMMAND" in scr, ("a pasted newline must not submit the command\n"+scr)
            # ...and the buffer is untouched (unmodified, path not inserted).
            assert "[+]" not in scr, ("the buffer must not be modified by the paste\n"+scr)
            assert screen.display[0].strip()=="hello world", \
                ("the paste must not land in the buffer\n"+scr)
            # Submit the (nonexistent) path just to confirm the command line is
            # what we built, then discard and quit cleanly.
            key("\r",.3)
            key(":qa!\r",.3)
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
    print(f"paste-to-command-line PTY passed: {cols}x{rows}")
