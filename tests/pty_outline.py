#!/usr/bin/env python3
"""Outline/symbol sidebar (,lO): opens against a real (mock) language
server, shows its documentSymbol response, and jumps to a symbol into
the adjacent pane -- driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-outline-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-outline-cfg-") as cfg:
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
        try:
            def wait_for(predicate, timeout=10):
                end=time.monotonic()+timeout
                while time.monotonic()<end:
                    if predicate():
                        return True
                    drain(.15)
                return False
            drain(.3)
            # Wait for the mock server's published diagnostic (a gutter
            # 'W' on line 1) as a readiness proxy, the same way
            # regression.rs's mock-LSP tests wait for diagnostics before
            # issuing a request -- otherwise ,lO can race a client that
            # hasn't finished initializing yet and get "no server" instead
            # of ever retrying.
            assert wait_for(lambda: "W" in screen.display[0][:2]), \
                ("mock LSP client never became ready\n"+"\n".join(screen.display))
            key(",lO",.6)
            assert wait_for(lambda: "symbol" in right_text()), \
                ("outline sidebar never showed the symbol\n"+right_text())
            assert "fn" in right_text(), ("outline sidebar missing the symbol kind\n"+right_text())
            key("\r",.3)
            assert "NORMAL" in screen.display[-2], \
                ("jumping from the outline did not focus the buffer pane\n"+"\n".join(screen.display))
            # ,lO toggles regardless of which pane has focus: pressed again
            # from the buffer pane, the sidebar (already open) closes.
            key(",lO",.4)
            assert "│" not in text(), \
                ("second ,lO from the buffer pane did not close the sidebar\n"+text())
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
    print(f"Outline sidebar PTY passed: {cols}x{rows}")
