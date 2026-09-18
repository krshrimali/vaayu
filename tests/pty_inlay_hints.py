#!/usr/bin/env python3
""",li requests textDocument/inlayHint and splices each hint's label
inline at its own column -- among the real characters, not appended
after them like line-blame/code-lens virtual text -- and Esc clears
them. Driven through a real PTY against a real (mock) language server."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-inlayhints-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-inlayhints-cfg-") as cfg:
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
        # Matches mock_lsp.py's fixed inlay-hint fixture: a hint right
        # after "one" on line 0, and a parts-shaped one at the start of
        # line 1.
        main_rs=root/"main.rs"; main_rs.write_text("one two three\nfour five six\n")
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
            key(",li",.5)
            assert wait_for(lambda: "2 inlay hint" in text()), \
                ("requesting inlay hints should report how many came back\n"+text())
            # The hint on line 0 sits right before the space after "one"
            # (character 3), splicing into the real text rather than
            # after it -- the real space that follows is untouched, so
            # "one : Type two three" is what that looks like once spliced.
            assert wait_for(lambda: "one : Type two three" in text()), \
                ("the type hint should be spliced in right after 'one', "
                 "among the real characters\n"+text())
            assert wait_for(lambda: "param: four five six" in text()), \
                ("a parts-shaped label should concatenate its values and "
                 "splice in at its own column\n"+text())
            key("\x1b",.3)  # Esc clears them
            assert wait_for(lambda: "one two three" in text() and "Type" not in text()), \
                ("Esc should clear inlay hints, restoring the plain text\n"+text())
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
    print(f"Inlay hints PTY passed: {cols}x{rows}")
