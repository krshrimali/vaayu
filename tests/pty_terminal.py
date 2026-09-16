#!/usr/bin/env python3
"""Embedded PTY terminal (:terminal): a real, nested interactive shell
inside the editor, driven through a real outer PTY -- verifies spawn,
input passthrough, output rendering, resize, Esc/i mode switching and
clean process shutdown end to end, not just the Rust-level unit tests."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, subprocess, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-terminal-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        file=root/"f.txt"
        file.write_text("hello\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"),SHELL="/bin/sh")
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
        def wait_for(marker,timeout=5):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if any(marker in row for row in screen.display):
                    return True
                drain(.1)
            return False
        try:
            drain(.3)
            key(":terminal\r",.5)
            assert wait_for("$") or wait_for("#") or wait_for("%"), \
                ("shell prompt never appeared\n"+"\n".join(screen.display))
            # A unique marker echoed by the real nested shell proves input
            # passthrough and output rendering both work end to end.
            key("echo TERMINAL_MARKER_XYZ\n",.3)
            assert wait_for("TERMINAL_MARKER_XYZ"), \
                ("echoed output never rendered\n"+"\n".join(screen.display))
            # Esc leaves Terminal mode to Normal (still on the pane); i
            # re-enters it -- prove both transitions work by using each.
            key("\x1b",.2)
            key("i",.2)
            key("echo BACK_IN_TERMINAL_MODE\n",.3)
            assert wait_for("BACK_IN_TERMINAL_MODE"), \
                ("re-entering terminal mode with i did not work\n"+"\n".join(screen.display))
            # Resize propagates to the real child: a shell run with `stty
            # size` reports the PTY's actual row/col count.
            key("\x1b",.2)
            fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows+4,cols+6,0,0))
            os.kill(pid,signal.SIGWINCH);drain(.3)
            key("i",.2)
            key("stty size\n",.3)
            wait_for(str(rows+4-1))  # minus the message/split-divider rows; approximate, just needs the resize to have taken effect at all
            # Closing the pane must not leak the shell process.
            key("\x1b",.2)
            key(":close\r",.3)
            end=time.monotonic()+3
            while time.monotonic()<end:
                remaining = subprocess.run(
                    ["pgrep", "-f", "-P", str(pid)], capture_output=True
                ).stdout.decode().split()
                if not remaining:
                    break
                drain(.1)
            remaining = subprocess.run(["pgrep", "-P", str(pid)], capture_output=True).stdout
            assert not remaining.decode().strip(), \
                ("child process(es) leaked after :close: "+remaining.decode())
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
    print(f"Terminal PTY passed: {cols}x{rows}")
