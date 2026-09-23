#!/usr/bin/env python3
"""Sticky-scroll LSP fallback: a buffer with no tree-sitter grammar (`.cpp`)
still pins its enclosing declarations, sourced from `textDocument/documentSymbol`
instead of tree-sitter. The mock returns class Outer (0..79) with method inner
(5..75); after scrolling past both, their declaration lines pin at the top."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(100,24),(140,40)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-ss-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-ss-cfg-") as cfg:
        root=pathlib.Path(proj); cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        log=root/"lsp.jsonl"
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\nsticky_scroll=true\n'
            '[lsp.fixture]\n'
            f'cmd = ["python3", "{mock_lsp}", "{log}", "--sticky-symbol"]\n'
            'filetypes = ["cpp"]\n')
        lines=[]
        for i in range(80):
            if i==0: lines.append("class Outer {")
            elif i==5: lines.append("  void inner() {")
            else: lines.append(f"  stmt {i};")
        main_cpp=root/"main.cpp"; main_cpp.write_text("\n".join(lines)+"\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(cfg))
            os.execv(binary,[binary,str(main_cpp)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.25):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                r,_,_=select.select([fd],[],[],.05)
                if r:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.25):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        def wait_for(pred,timeout=8):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.15)
            return False
        try:
            drain(.5)
            assert wait_for(lambda: "W" in screen.display[0][:2]), \
                ("mock LSP never became ready\n"+text())
            # Jump to the end so both declarations (lines 0 and 5) scroll off the
            # top -- a jump (not incremental Ctrl-e) so the whole pane repaints.
            key("G",.5)
            drain(.5)
            # Both enclosing declarations pin at the top even though scrolled past.
            assert wait_for(lambda: "class Outer {" in text()), \
                ("the enclosing class should pin from LSP symbols\n"+text())
            assert "void inner()" in text(), \
                ("the enclosing method should also pin\n"+text())
            # A non-container line scrolled off is not pinned (anti-false-positive).
            assert "stmt 1;" not in text(), \
                ("a scrolled-off non-container line must not be pinned\n"+text())
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
    print(f"sticky scroll LSP fallback PTY passed: {cols}x{rows}")
