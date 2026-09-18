#!/usr/bin/env python3
""",lc requests textDocument/codeLens: only the lens with a real `command`
is shown (the resolve-only one is skipped), it's rendered as virtual text
after its own line in the buffer, and Enter on it in the "Code lenses"
list actually runs its command via workspace/executeCommand -- driven
through a real PTY against a real (mock) language server."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-codelens-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-codelens-cfg-") as cfg:
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
        main_rs=root/"main.rs"; main_rs.write_text("fn main() {}\nlet x = 1;\n")
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
        try:
            drain(.3)
            assert wait_for(lambda: "W" in screen.display[0][:2]), \
                ("mock LSP client never became ready\n"+text())
            key(",lc",.5)
            assert wait_for(lambda: "Run fixture" in text()), \
                ("code lenses list should show the runnable lens's title\n"+text())
            assert "1 results" in text(), \
                ("the resolve-only lens (no command) should be skipped, "
                 "leaving exactly one entry\n"+text())
            key("q",.2)  # close the list; the lens should still render inline
            assert wait_for(lambda: "» ▶ Run fixture" in text()), \
                ("the runnable lens should also show as virtual text on its "
                 "own line after closing the list\n"+text())
            key(",lc",.5)  # reopen and run it
            assert wait_for(lambda: "Run fixture" in text())
            key("\r",.4)
            assert wait_for(lambda: "Code action completed" in text()), \
                ("running a code lens should reuse apply_code_action's "
                 "existing completion message\n"+text())
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
    print(f"Code lens PTY passed: {cols}x{rows}")
