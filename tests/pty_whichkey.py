#!/usr/bin/env python3
"""Which-key prefix popup and :keymaps command-palette execution."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-whichkey-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        # A short delay keeps the test fast without changing what the popup
        # asserts: it must still be absent before the delay and present after.
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nwhichkey_delay_ms=120\n'
        )
        file=root/"f.txt"
        file.write_text("hello world\nsecond line\n")
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
        def key(s,seconds=.2):os.write(fd,s.encode());drain(seconds)
        try:
            drain(.3)
            # Completed mapping before the delay must never show the popup
            # and must still run: ",h" clears search highlighting immediately.
            key("/world\r",.15)
            key(",h",.05)
            text="\n".join(screen.display)
            assert "Diagnostics list" not in text, \
                ("popup leaked into a completed mapping\n"+text)
            # A bare prefix left hanging past the delay shows the popup with
            # every ,l* continuation, but not unrelated bindings.
            key(",l",0)
            drain(.4)
            text="\n".join(screen.display)
            assert "Diagnostics list" in text, ("which-key popup missing\n"+text)
            assert "Format buffer" in text, ("which-key popup incomplete\n"+text)
            assert "Write current buffer" not in text, \
                ("which-key popup leaked unrelated bindings\n"+text)
            key("\x1b",.2)  # cancel back to Normal
            # :keymaps opens a searchable palette generated from the same
            # registry, and selecting an entry actually executes it.
            key(":keymaps\r",.3)
            text="\n".join(screen.display)
            assert "Keymaps" in text and "results" in text, \
                ("keymaps palette did not open\n"+text)
            key("/Toggle line wrap\r",.2)
            key("\r",.3)
            text="\n".join(screen.display)
            assert "wrap: false" in text, ("selecting a keymap entry did not run it\n"+text)
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
    print(f"Which-key PTY passed: {cols}x{rows}")
