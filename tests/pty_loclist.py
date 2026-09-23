#!/usr/bin/env python3
""":ldiagnostics fills a location list (distinct from quickfix) from the current
buffer's diagnostics; :lopen reopens it and Enter jumps to the entry. Driven
through a real PTY with the mock language server (which publishes one warning)."""
import codecs, fcntl, os, pathlib, pty, re, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
def cursor_line(screen):
    for row in reversed(screen.display):
        m=re.findall(r"(\d+):(\d+)", row)
        if m:
            return int(m[-1][0])
    return None
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-loclist-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-loclist-cfg-") as cfg:
        root=pathlib.Path(proj); cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        log=root/"lsp.jsonl"
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            '[lsp.fixture]\n'
            f'cmd = ["python3", "{mock_lsp}", "{log}"]\n'
            'filetypes = ["rust"]\n')
        main_rs=root/"main.rs"; main_rs.write_text("one\ntwo\nthree\nfour\nfive\n")
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
        def wait_for(pred,timeout=4.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.3)
            assert wait_for(lambda: "W" in screen.display[0][:2]), \
                ("mock LSP never became ready\n"+text())
            key("4G",.2)  # move away from the diagnostic (line 1) so the jump shows
            key(":ldiagnostics\r",.4)
            assert wait_for(lambda: "Location list" in text()), \
                (":ldiagnostics should open the location list\n"+text())
            assert "fixture warning" in text(), ("the diagnostic entry is listed\n"+text())
            key("\r",.3)  # Enter jumps to the diagnostic (line 1)
            assert wait_for(lambda: cursor_line(screen)==1), \
                ("Enter should jump to the diagnostic's line\n"+text())
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
    print(f"loclist PTY passed: {cols}x{rows}")
