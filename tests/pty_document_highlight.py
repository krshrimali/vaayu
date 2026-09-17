#!/usr/bin/env python3
""",lh requests textDocument/documentHighlight and paints every returned
occurrence with a background highlight; a plain Esc clears it, and an
edit to the buffer makes a previously-shown set stale (no longer
painted) -- driven through a real PTY against a real (mock) language
server."""
import codecs, fcntl, os, pathlib, pty, select, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-dochl-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-dochl-cfg-") as cfg:
        root=pathlib.Path(proj)
        cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        log=root/"lsp.jsonl"
        config = (
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            '[lsp.fixture]\n'
            f'cmd = ["python3", "{mock_lsp}", "{log}"]\n'
            'filetypes = ["rust"]\n'
        )
        (cfg/"vaayu/config.toml").write_text(config)
        # mock_lsp.py's documentHighlight fixture always replies with two
        # fixed ranges: line 0 chars 4-10, line 2 chars 0-6.
        main_rs=root/"main.rs"
        main_rs.write_text("one two three\nfour five six\nseven eight nine\n")
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
        def wait_for(predicate, timeout=10):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if predicate():
                    return True
                drain(.15)
            return False
        # Text rows are offset by 1 (row 0 is the status/mode line is NOT
        # shown here -- this editor draws buffer content starting at row 0
        # for a single full-width pane), matching other buffer-pane PTY
        # tests in this suite.
        def cell_bg(y, x):
            return screen.buffer[y][x].bg
        def line0_highlighted():
            return any(cell_bg(0, x) not in ("default",) for x in range(4, 10))
        def line2_highlighted():
            return any(cell_bg(2, x) not in ("default",) for x in range(0, 6))
        try:
            drain(.3)
            assert wait_for(lambda: "W" in screen.display[0][:2]), \
                ("mock LSP client never became ready\n"+text())
            key(",lh",.5)
            assert wait_for(lambda: "occurrence" in screen.display[-1]), \
                ("documentHighlight message never appeared\n"+text())
            assert wait_for(line0_highlighted), \
                ("line 0's occurrence should be background-highlighted\n"+text())
            assert wait_for(line2_highlighted), \
                ("line 2's occurrence should be background-highlighted\n"+text())
            # An edit makes the highlight stale -- it should stop painting.
            key("x",.3)  # delete a char on line 0 (cursor starts at 0,0)
            assert wait_for(lambda: not line0_highlighted()), \
                ("editing the buffer should make the highlight stale and "
                 "stop painting it\n"+text())
            key("u",.2)  # undo the edit, back to the original text
            # Re-request, then Esc should clear it.
            key(",lh",.5)
            assert wait_for(line0_highlighted), \
                ("re-requesting after undo should re-highlight\n"+text())
            key("\x1b",.3)
            assert wait_for(lambda: not line0_highlighted() and not line2_highlighted()), \
                ("Esc should clear the document highlight\n"+text())
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
                import signal
                os.kill(pid,signal.SIGKILL);os.waitpid(pid,0)
            os.close(fd)
    print(f"Document highlight PTY passed: {cols}x{rows}")
