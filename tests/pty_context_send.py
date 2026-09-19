#!/usr/bin/env python3
""",cx builds structured context (current file, selection, clipboard,
symbol, diagnostics) and copies it to the clipboard register, also
typing it directly into an attached agent session's input if one
exists -- driven through a real PTY, standing /bin/cat in for
claude/codex (neither is installed in this sandbox) so its own pane
echoes back exactly what was written to its stdin."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-contextsend-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            'agent_commands = { testagent = ["/bin/cat"] }\n'
        )
        file=root/"f.txt"; file.write_text("CONTEXT_MARKER_LINE\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"),SHELL="/bin/sh")
            os.execv(binary,[binary,str(file)])
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
        def text():
            return "\n".join(screen.display)
        def wait_for(pred, timeout=6.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():
                    return True
                drain(.1)
            return False
        try:
            drain(.4)
            key(":agent testagent\r",.5)  # opens a split, focuses the cat pane
            key("\x1b",.2)                # Esc: Terminal -> Normal, still on that pane
            key("\x17w",.3)               # Ctrl-W w: cycle focus back to the file pane
            key(",cx",.4)                 # context picker
            assert wait_for(lambda: "Current file" in text()), \
                ("the context picker should list its kinds\n"+text())
            key("\r",.5)                  # Enter on "Current file" (first entry)
            assert wait_for(lambda: "sent to" in text().lower()), \
                ("sending should report reaching the attached agent session\n"+text())
            assert wait_for(lambda: "CONTEXT_MARKER_LINE" in text()), \
                ("cat should have echoed the file content it received on its own stdin\n"+text())

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
    print(f"Context send PTY passed: {cols}x{rows}")
