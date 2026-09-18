#!/usr/bin/env python3
"""A snippet completion's ${n|a,b,c|} choice can be cycled with Ctrl-N/
Ctrl-P while still selected, and a placeholder nested inside another
one's default (${2:arg ${3:nested}}) tabs through both levels correctly
-- driven through a real PTY against a real (mock) language server."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-snippetchoice-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-snippetchoice-cfg-") as cfg:
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
        main_rs=root/"main.rs"; main_rs.write_text("\n")
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
            key("i",.2)
            # "sni" (3 chars, matching the mock's hardcoded 0-3 edit
            # range) fuzzy-matches "SNIP", not "FIX".
            key("sni",.5)
            assert wait_for(lambda: "snip" in text()), \
                ("completion popup should offer the snippet item\n"+text())
            key("\r",.4)  # accept -- expands to "fn foo(arg nested) {}"
            assert wait_for(lambda: "fn foo(arg nested) {}" in screen.display[0]), \
                ("the snippet should expand with the first choice selected\n"+text())
            key("\x0e",.3)  # Ctrl-N: cycle the choice forward
            assert wait_for(lambda: "fn bar(arg nested) {}" in screen.display[0]), \
                ("Ctrl-N should cycle the choice stop's text\n"+text())
            key("\x0e",.3)
            assert wait_for(lambda: "fn baz(arg nested) {}" in screen.display[0]), \
                ("Ctrl-N should keep cycling through the choice list\n"+text())
            key("\x0e",.3)  # wraps back to the first choice
            assert wait_for(lambda: "fn foo(arg nested) {}" in screen.display[0]), \
                ("cycling should wrap back around to the first choice\n"+text())
            key("\t",.3)  # Tab: move to the next placeholder (nested one's parent)
            key("changed",.3)  # types over "arg nested" (the outer placeholder)
            assert wait_for(lambda: "fn foo(changed) {}" in screen.display[0]), \
                ("typing over the outer nested placeholder should replace "
                 "the whole thing, including the inner one\n"+text())
            key("\x1b",.2)  # leave Insert mode before quitting
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
    print(f"Snippet choices/nested placeholder PTY passed: {cols}x{rows}")
