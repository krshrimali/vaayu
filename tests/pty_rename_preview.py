#!/usr/bin/env python3
"""With refactor_preview on, `:rename` shows a diff-style preview of the LSP
WorkspaceEdit and defers it: the buffer is unchanged until `:renameapply`, and
`:renamecancel` drops it. Driven through a real PTY against a mock LSP."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-rnp-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-rnp-cfg-") as cfg:
        root=pathlib.Path(proj); cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        log=root/"lsp.jsonl"
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            'refactor_preview=true\n'
            '[lsp.fixture]\n'
            f'cmd = ["python3", "{mock_lsp}", "{log}"]\n'
            'filetypes = ["rust"]\n')
        main_rs=root/"main.rs"; main_rs.write_text("abc def\n")
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
        def key(s,seconds=.2):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        def wait_for(pred,timeout=10):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.15)
            return False
        try:
            drain(.3)
            assert wait_for(lambda: "W" in screen.display[0][:2]), \
                ("mock LSP client never became ready\n"+text())
            key(":rename ZZZ\r",.5)
            # A preview list appears; the edit is NOT yet applied.
            assert wait_for(lambda: "Rename preview" in text()), \
                ("rename should show a preview list\n"+text())
            assert "ZZZ" in text(), ("preview should show the replacement text\n"+text())
            key("q",.3)  # close the list; buffer underneath is still original
            assert wait_for(lambda: "abc def" in text()), \
                ("the buffer must be unchanged before applying\n"+text())
            # Cancel path first: a fresh preview then :renamecancel keeps it intact.
            key(":rename QQQ\r",.5)
            assert wait_for(lambda: "Rename preview" in text())
            key("q",.2)
            key(":renamecancel\r",.3)
            assert wait_for(lambda: "abc def" in text() and "cancelled" in text()), \
                ("renamecancel should discard the pending edit\n"+text())
            # Now apply for real.
            key(":rename ZZZ\r",.5)
            assert wait_for(lambda: "Rename preview" in text())
            key("q",.2)
            key(":renameapply\r",.4)
            assert wait_for(lambda: "ZZZ def" in text()), \
                ("renameapply should commit the edit\n"+text())
            key(":w\r",.3)
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
        assert main_rs.read_text()=="ZZZ def\n", main_rs.read_text()
    print(f"rename preview PTY passed: {cols}x{rows}")
