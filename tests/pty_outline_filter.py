#!/usr/bin/env python3
"""Outline sidebar (,lO): `f` cycles a symbol-kind filter (all -> fn ->
var -> back to all), narrowing the displayed nodes without re-requesting
symbols -- driven through a real PTY against a real (mock) language
server returning symbols of two different kinds."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-outlinefilt-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-outlinefilt-cfg-") as cfg:
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
        main_rs=root/"main.rs"; main_rs.write_text("fn a_fn() {}\nfn b_fn() {}\nlet c_var = 1;\n")
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
            assert wait_for(lambda: "a_fn" in right_text() and "c_var" in right_text()), \
                ("outline sidebar never showed all symbols\n"+right_text())
            # f cycles to the first kind seen ("fn"): only a_fn/b_fn remain.
            key("f",.2)
            assert "a_fn" in right_text() and "b_fn" in right_text() and "c_var" not in right_text(), \
                ("f should filter down to fn symbols only\n"+right_text())
            # f again cycles to "var": only c_var remains.
            key("f",.2)
            assert "c_var" in right_text() and "a_fn" not in right_text() and "b_fn" not in right_text(), \
                ("f again should filter down to var symbols only\n"+right_text())
            # f again wraps back to "all".
            key("f",.2)
            assert "a_fn" in right_text() and "b_fn" in right_text() and "c_var" in right_text(), \
                ("f should wrap back to showing all symbols\n"+right_text())
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
    print(f"Outline kind-filter PTY passed: {cols}x{rows}")
