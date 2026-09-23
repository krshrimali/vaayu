#!/usr/bin/env python3
"""`:set colorswatch` paints an LSP documentColor literal as a background chip
(its own RGB as the background, contrasting foreground) instead of only the
foreground. The mock returns pure red for line 0 chars 0-3, so with colorswatch
on those cells' background becomes ff0000 (white text); with it off they fall
back to red *foreground*. Driven through a real PTY with the mock server."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-sw-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-sw-cfg-") as cfg:
        root=pathlib.Path(proj); cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        log=root/"lsp.jsonl"
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\ncolorswatch=true\n'
            '[lsp.fixture]\n'
            f'cmd = ["python3", "{mock_lsp}", "{log}"]\n'
            'filetypes = ["rust"]\n')
        main_rs=root/"main.rs"; main_rs.write_text("red = 0xff0000;\nother line\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(cfg))
            os.execv(binary,[binary,str(main_rs)])
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
        def text():return "\n".join(screen.display)
        # A diagnostic sign can occupy the first cells, so scan the row rather
        # than assuming the literal sits at column 0.
        def bg_red_cells():
            return [x for x in range(cols) if screen.buffer[0][x].bg=="ff0000"]
        def fg_red_cells():
            return [x for x in range(cols) if screen.buffer[0][x].fg=="ff0000"]
        def row0_bg_red():
            return len(bg_red_cells())>=3
        def row0_fg_red():
            return len(fg_red_cells())>=3
        def wait_for(pred,timeout=4.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.3)
            assert wait_for(lambda: "W" in screen.display[0][:2]), \
                ("mock LSP never became ready\n"+text())
            assert not row0_bg_red(), ("no swatch before requesting\n"+text())
            key(",lC",.5)   # request documentColor
            assert wait_for(row0_bg_red), ("colorswatch paints the literal's background\n"+text())
            assert all(screen.buffer[0][x].fg=="ffffff" for x in bg_red_cells()), \
                ("swatch text uses a contrasting (white) foreground on red\n"+text())
            key("\x1b",.3)  # Esc clears the overlay
            assert wait_for(lambda: not row0_bg_red()), ("Esc clears the swatch\n"+text())
            # With colorswatch off, the same literal paints the *foreground* red.
            key(":set nocolorswatch\r",.3)
            key(",lC",.5)
            assert wait_for(row0_fg_red), ("without colorswatch the literal is a red foreground\n"+text())
            assert not row0_bg_red(), ("no background chip when colorswatch is off\n"+text())
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
    print(f"colorswatch PTY passed: {cols}x{rows}")
