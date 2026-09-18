#!/usr/bin/env python3
""",ll requests textDocument/documentLink and lists results; Enter on a
file:// link opens it, Enter on a web link copies it instead of opening
a browser -- driven through a real PTY against a real (mock) language
server."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-doclinks-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-doclinks-cfg-") as cfg:
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
        (root/"other.txt").write_text("sibling content\n")
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
            key(",ll",.5)
            assert wait_for(lambda: "Open other.txt" in text()), \
                ("document links list should show the file link's tooltip\n"+text())
            key("j",.2)   # onto the web link entry (second one)
            key("\r",.3)  # "open" it -- should copy, not navigate
            assert "main.rs" in screen.display[-2], \
                ("a web link must not switch buffers\n"+text())
            assert wait_for(lambda: "Copied link" in text()), \
                ("a web link should be copied, not opened in a browser\n"+text())
            key(",ll",.5)  # re-request (still from main.rs, which has the LSP)
            assert wait_for(lambda: "Open other.txt" in text()), \
                ("document links list should reopen\n"+text())
            key("\r",.3)  # open the file link (first entry, cursor reset)
            assert wait_for(lambda: "other.txt" in screen.display[-2]), \
                ("opening a file link should switch to that buffer\n"+text())
            assert wait_for(lambda: "sibling content" in text()), \
                ("the opened buffer should show the real file's content\n"+text())
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
    print(f"Document links PTY passed: {cols}x{rows}")
