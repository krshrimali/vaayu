#!/usr/bin/env python3
"""`:workspacediagnostics` pulls project-wide diagnostics from a capable server
(workspace/diagnostic) and merges them into the shared diagnostics list, so
`:diagnostics` shows problems from files that were never opened. Driven through
a real PTY against the mock LSP (--pull)."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-wsd-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-wsd-cfg-") as cfg:
        root=pathlib.Path(proj); cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        log=root/"lsp.jsonl"
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            '[lsp.fixture]\n'
            f'cmd = ["python3", "{mock_lsp}", "{log}", "--pull"]\n'
            'filetypes = ["rust"]\n')
        main_rs=root/"main.rs"; main_rs.write_text("abc\n")
        # The files the mock reports workspace diagnostics for.
        (root/"wsa.txt").write_text("aaa\n")
        (root/"wsb.txt").write_text("x\nbbb\n")
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
        def key(s,seconds=.3):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        def wait_for(pred,timeout=10):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.15)
            return False
        try:
            drain(.3)
            # main.rs gets a severity-1 pull diagnostic on open -> "E" gutter
            # marker on row 0, which confirms the LSP is live.
            assert wait_for(lambda: "E" in screen.display[0][:2]), \
                ("mock LSP never became ready\n"+text())
            key(":workspacediagnostics\r",.6)
            key(":diagnostics\r",.6)
            # Both files' workspace diagnostics show in the shared list.
            assert wait_for(lambda: "WSDIAG_A" in text() and "WSDIAG_B" in text()), \
                ("workspace diagnostics should populate the list\n"+text())
            assert "wsa.txt" in text() and "wsb.txt" in text(), \
                ("both unopened files are listed\n"+text())
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
    print(f"workspace diagnostics PTY passed: {cols}x{rows}")
