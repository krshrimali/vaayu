#!/usr/bin/env python3
"""gy/gI/gD (type definition/implementation/declaration) jump to the
single location a real (mock) language server returns -- driven
through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-gotolsp-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-gotolsp-cfg-") as cfg:
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
        main_rs=root/"main.rs"
        main_rs.write_text("one\ntwo\nthree\nfour\ntarget_line\nsix\n")
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
        def wait_for(pred, timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():
                    return True
                drain(.05)
            return False
        try:
            drain(.3)
            # Wait for the mock LSP's automatic didOpen diagnostic, using
            # the gutter marker as a readiness proxy (a fixed sleep would
            # be flaky on a loaded machine).
            assert wait_for(lambda: "W" in text()), ("mock LSP never became ready\n"+text())
            for keyseq, label in (("gy","gy (type definition)"), ("gI","gI (implementation)"), ("gD","gD (declaration)")):
                key("gg",.2)  # back to the top before each jump
                key(keyseq,.3)
                key("x",.2)  # delete the char under cursor (col 2 -> 'r') to prove landing position
                assert "taget_line" in text() and "target_line" not in text(), \
                    (f"{label} should jump to target_line\n"+text())
                key("u",.2)  # undo for the next iteration
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
    print(f"Goto LSP PTY passed: {cols}x{rows}")
