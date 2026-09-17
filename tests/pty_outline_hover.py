#!/usr/bin/env python3
"""Outline hover preview (K): shows hover documentation for the symbol
under the outline cursor without navigating -- the buffer pane's own
cursor position is left untouched -- driven through a real PTY against
a real (mock) language server."""
import codecs, fcntl, os, pathlib, pty, re, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-outlinehover-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-outlinehover-cfg-") as cfg:
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
        main_rs=root/"main.rs"; main_rs.write_text("fn main() {\n    body();\n}\n")
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
            # Move the buffer's real cursor first, so a bug that actually
            # navigates (instead of just peeking) would be caught below.
            def cursor_pos():
                m = re.search(r"\b(\d+:\d+)\b", screen.display[-2])
                return m.group(1) if m else None
            key("j$",.2)
            before = cursor_pos()
            key(",lO",.6)
            assert wait_for(lambda: "symbol" in right_text()), \
                ("outline sidebar never showed the symbol\n"+right_text())
            key("K",.5)
            assert wait_for(lambda: "fixture hover" in text()), \
                ("K in the outline should show the hover text\n"+text())
            key("q",.3)  # close the hover results, back to the outline
            assert wait_for(lambda: "symbol" in right_text()), \
                ("closing hover should return to the outline sidebar\n"+text())
            key("\x17w",.3)  # Ctrl-w w back to the buffer pane
            after = cursor_pos()
            assert before is not None and before == after, \
                ("hovering from the outline must not move the buffer's cursor\n"
                 f"before: {before!r}\nafter:  {after!r}")
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
    print(f"Outline hover PTY passed: {cols}x{rows}")
