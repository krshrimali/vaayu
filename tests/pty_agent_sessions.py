#!/usr/bin/env python3
""":agent <name> (and :claude/:codex, the same code path) starts a
named long-lived terminal session, toggling it on repeated calls:
detach hides the pane without killing the process (its scrollback
survives), and a later call reattaches the *same* session rather than
starting a new one. :agents lists running sessions and Enter attaches
one. Driven through a real PTY, standing a plain shell in for
claude/codex (neither is installed in this sandbox) via a configured
agent_commands override -- the toggle/detach/reattach logic itself
doesn't care which binary it's running."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-agentsession-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            'agent_commands = { testagent = ["/bin/sh"] }\n'
        )
        file=root/"f.txt"; file.write_text("hello\n")
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
            key(":agent testagent\r",.5)
            assert wait_for(lambda: "$" in text() or "#" in text()), \
                ("starting an agent session should open a real shell pane\n"+text())
            key("echo AGENT_MARKER\n",.4)
            assert wait_for(lambda: "AGENT_MARKER" in text()), \
                ("the session's own shell should actually be running\n"+text())

            key("\x1b",.3)  # Esc: pane navigation, still attached
            key(":agent testagent\r",.5)  # toggle #2: detach
            assert wait_for(lambda: "detached" in text().lower()), \
                ("re-running the same :agent should detach it\n"+text())
            assert "AGENT_MARKER" not in text(), \
                ("the detached pane should no longer be on screen\n"+text())

            key(":agent testagent\r",.5)  # toggle #3: reattach
            assert wait_for(lambda: "reattached" in text().lower()), \
                ("re-running :agent a third time should reattach it\n"+text())
            assert wait_for(lambda: "AGENT_MARKER" in text()), \
                ("reattaching should show the same session's scrollback, "
                 "proving it's the same process, not a fresh one\n"+text())

            key("\x1b",.3)
            key(":agents\r",.4)
            assert wait_for(lambda: "testagent" in text() and "attached" in text()), \
                (":agents should list the running session\n"+text())
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
    print(f"Agent sessions PTY passed: {cols}x{rows}")
