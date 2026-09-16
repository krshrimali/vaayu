#!/usr/bin/env python3
"""File tree: a file (or an unexpanded ancestor directory) with LSP
diagnostics shows an E/W/I marker next to it -- driven through a real
PTY against a real (mock) language server."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-treediag-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-treediag-cfg-") as cfg:
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
        (root/"sub").mkdir()
        warned=root/"sub"/"warned.rs"; warned.write_text("fn main() {}\n")
        clean=root/"clean.rs"; clean.write_text("fn other() {}\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(cfg))
            os.execv(binary,[binary,str(warned)])
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
        def right_text():
            half = cols // 2
            return "\n".join(row[half:] for row in screen.display)
        def wait_for(pred, timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():
                    return True
                drain(.05)
            return False
        try:
            drain(.3)
            # Wait for the mock LSP's automatic didOpen diagnostic (a
            # Warning, per mock_lsp.py's fixed publishDiagnostics reply)
            # to actually land, using the main buffer's own gutter marker
            # as the readiness proxy -- a fixed sleep here would be flaky
            # on a loaded machine (see pty_outline.py's own history).
            def left_text():
                half = cols // 2
                return "\n".join(row[:half] for row in screen.display)
            assert wait_for(lambda: "W" in left_text()), \
                ("mock LSP diagnostic never arrived on the open buffer\n"+left_text())
            # Open the tree with sub/ collapsed: warned.rs itself is not
            # visible yet, but sub/ should already show the marker.
            key(",ft",.3)
            assert wait_for(lambda: "sub" in right_text()), ("tree never showed sub/\n"+right_text())
            # Give the diagnostic time to arrive and the tree a redraw.
            assert wait_for(lambda: " W" in right_text()), \
                ("collapsed sub/ should show the inherited W marker\n"+right_text())
            assert "clean.rs" in right_text() and "clean.rs W" not in right_text(), \
                ("clean.rs has no diagnostics and must not get a marker\n"+right_text())
            # Expanding sub/ reveals warned.rs, which should carry its own marker.
            key("l",.2)
            assert "warned.rs W" in right_text() or ("warned.rs" in right_text() and "W" in right_text()), \
                ("warned.rs itself should show the W marker once expanded\n"+right_text())
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
    print(f"File tree diagnostics PTY passed: {cols}x{rows}")
