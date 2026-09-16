#!/usr/bin/env python3
"""Outline sidebar (,lO): `h` collapses a symbol with children, hiding its
descendants but not its siblings; `l` expands it back -- driven through a
real PTY against a real (mock) language server returning a nested
documentSymbol response."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-outlinecol-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-outlinecol-cfg-") as cfg:
        root=pathlib.Path(proj)
        cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        log=root/"lsp.jsonl"
        config = (
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            '[lsp.fixture]\n'
            f'cmd = ["python3", "{mock_lsp}", "{log}", "--nested-symbol"]\n'
            'filetypes = ["rust"]\n'
        )
        (cfg/"vaayu/config.toml").write_text(config)
        main_rs=root/"main.rs"; main_rs.write_text("struct Parent;\nfn Child() {}\nfn Sibling() {}\n")
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
            drain(.4)
            key(",lO",.3)
            assert wait_for(lambda: "Parent" in right_text() and "Child" in right_text() and "Sibling" in right_text()), \
                ("outline sidebar never showed the nested symbols\n"+right_text())
            # Cursor starts on Parent (the first node); h collapses it.
            key("h",.2)
            assert "Parent" in right_text() and "Sibling" in right_text() and "Child" not in right_text(), \
                ("h should collapse Parent, hiding Child but not Sibling\n"+right_text())
            # l on the still-selected (now collapsed) Parent expands it again.
            key("l",.2)
            assert "Child" in right_text(), ("l should expand Parent again\n"+right_text())
            # h on Sibling (a leaf) is a no-op -- nothing should disappear.
            key("j",.2); key("j",.2) # Parent -> Child -> Sibling
            key("h",.2)
            assert "Parent" in right_text() and "Child" in right_text() and "Sibling" in right_text(), \
                ("h on a leaf must be a no-op\n"+right_text())
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
    print(f"Outline collapse PTY passed: {cols}x{rows}")
