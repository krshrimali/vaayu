#!/usr/bin/env python3
""",la lists code actions with a disabled one shown (not hidden) and its
reason, the isPreferred one marked and sorted first; opening the disabled
one refuses with a message instead of applying anything. ,lI (organize
imports) applies its one action directly, without a picker. Driven
through a real PTY against a real (mock) language server."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-codeactions-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-codeactions-cfg-") as cfg:
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
        main_rs=root/"main.rs"; main_rs.write_text("abc\n")
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
            key(",la",.5)
            assert wait_for(lambda: "* Fix fixture" in text()), \
                ("the preferred action should sort first and be marked\n"+text())
            # "not applicable here" may be truncated at narrow widths, so
            # just check the "(disabled: ..." marker itself is present.
            assert wait_for(lambda: "Disabled fixture" in text() and "(disabled:" in text()), \
                ("the disabled action should still be shown, with its reason\n"+text())
            key("j",.2)  # onto the disabled entry
            key("\r",.3)
            assert wait_for(lambda: "This action is disabled:" in text()), \
                ("selecting a disabled action should refuse with its reason\n"+text())
            key("q",.2)  # close the list to see the buffer underneath
            assert wait_for(lambda: "abc" in text() and "FIX" not in text()), \
                ("a disabled action must never modify the buffer\n"+text())
            key(",la",.5)  # reopen; cursor resets to the top (preferred) entry
            assert wait_for(lambda: "* Fix fixture" in text())
            key("\r",.3)
            assert wait_for(lambda: "FIX" in text()), \
                ("the preferred action should actually apply when selected\n"+text())
            key(",lI",.5)  # organize imports: applies directly, no picker
            assert wait_for(lambda: "ORGANIZED" in text()), \
                ("organize imports should apply its one action directly\n"+text())
            assert "Code actions" not in text(), \
                ("organize imports must never show the code-actions picker\n"+text())
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
    print(f"Code actions PTY passed: {cols}x{rows}")
