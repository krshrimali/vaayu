#!/usr/bin/env python3
"""Outline follow-cursor: with the sidebar open and the buffer pane (not
the sidebar) focused, moving the buffer cursor across symbol boundaries
moves the sidebar's highlighted symbol to match, without any outline-pane
keypress -- driven through a real PTY against a real (mock) language
server."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-outline-follow-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-outline-follow-cfg-") as cfg:
        root=pathlib.Path(proj)
        cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        log=root/"lsp.jsonl"
        config = (
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            '[lsp.fixture]\n'
            f'cmd = ["python3", "{mock_lsp}", "{log}", "--multi-symbol"]\n'
            'filetypes = ["rust"]\n'
        )
        (cfg/"vaayu/config.toml").write_text(config)
        # --multi-symbol replies with a_fn at line 0, b_fn at line 1,
        # c_var at line 2 -- one source line per symbol, plus a trailing
        # line so line 2 (c_var) isn't the buffer's last line either.
        main_rs=root/"main.rs"
        main_rs.write_text("fn a_fn() {}\nfn b_fn() {}\nlet c_var = 1;\n// tail\n")
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
        def key(s,seconds=.15):os.write(fd,s.encode());drain(seconds)
        def text():
            return "\n".join(screen.display)
        def right_text():
            half = cols // 2
            return "\n".join(row[half:] for row in screen.display)
        def wait_for(predicate, timeout=10):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if predicate():
                    return True
                drain(.15)
            return False
        try:
            drain(.3)
            assert wait_for(lambda: "W" in screen.display[0][:2]), \
                ("mock LSP client never became ready\n"+"\n".join(screen.display))
            key(",lO",.6)
            assert wait_for(lambda: "a_fn" in right_text()), \
                ("outline sidebar never showed the symbols\n"+right_text())
            # Move focus back to the buffer pane (outline opened focused).
            key("\x17w",.3)  # Ctrl-w w cycles pane focus
            assert "NORMAL" in screen.display[-2], \
                ("Ctrl-w w did not return focus to the buffer pane\n"+"\n".join(screen.display))
            # Cursor starts on line 0 (a_fn) -- confirm its row is
            # highlighted (reverse video) without any outline keypress.
            def row_reversed(y):
                # only the sidebar half -- the buffer pane's own status/
                # selection styling must not produce a false positive
                half = cols // 2
                return any(getattr(screen.buffer[y][x], "reverse", False) for x in range(half, cols))
            assert wait_for(lambda: row_reversed(0)), \
                ("a_fn's row should start highlighted (cursor is on line 0)\n"+right_text())
            key("j",.3)  # buffer cursor -> line 1 (b_fn)
            assert wait_for(lambda: row_reversed(1) and not row_reversed(0)), \
                ("moving to line 1 should move the highlight to b_fn\n"+right_text())
            key("j",.3)  # buffer cursor -> line 2 (c_var)
            assert wait_for(lambda: row_reversed(2) and not row_reversed(1)), \
                ("moving to line 2 should move the highlight to c_var\n"+right_text())
            key("j",.3)  # buffer cursor -> line 3, past every symbol start
            assert wait_for(lambda: row_reversed(2)), \
                ("past the last symbol, c_var (nearest preceding) stays highlighted\n"+right_text())
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
    print(f"Outline follow-cursor PTY passed: {cols}x{rows}")
