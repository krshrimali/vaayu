#!/usr/bin/env python3
"""Semantic-token incremental (`full/delta`) updates: after the initial full
request the editor holds the server's resultId, and a later edit triggers a
`semanticTokens/full/delta` request whose edits are spliced into the token
stream. The mock's delta appends a keyword token on line 3, so line 3 turns
cyan only after the edit. Also verifies the delta method was actually sent."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(80,24),(140,40)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-sd-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-sd-cfg-") as cfg:
        root=pathlib.Path(proj); cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        log=root/"lsp.jsonl"
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            '[lsp.fixture]\n'
            f'cmd = ["python3", "{mock_lsp}", "{log}", "--semantic"]\n'
            'filetypes = ["rust"]\n')
        main_rs=root/"main.rs"; main_rs.write_text("abc = 1;\nmore\nrdonly\nkw3 x\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(cfg))
            os.execv(binary,[binary,str(main_rs)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.25):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                ready,_,_=select.select([fd],[],[],min(.02,max(0,end-time.monotonic())))
                if ready:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.25):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        def row_cyan(y):return any(screen.buffer[y][x].fg=="00ffff" for x in range(cols))
        def wait_for(pred,timeout=5.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.3)
            assert wait_for(lambda: "W" in screen.display[0][:2]), \
                ("mock LSP never became ready\n"+text())
            key(":set semantictokens\r",.5)   # initial full request
            assert wait_for(lambda: row_cyan(0)), \
                ("the full request colors line 0's keyword cyan\n"+text())
            assert not row_cyan(3), ("line 3 has no token before the delta\n"+text())
            # Edit (append on line 2) -> bumps the revision -> a delta request.
            key("2GA z\x1b",.5)
            assert wait_for(lambda: row_cyan(3)), \
                ("the delta appends a keyword token on line 3 (turns cyan)\n"+text())
            # The incremental request was actually used.
            assert wait_for(lambda: "semanticTokens/full/delta" in log.read_text()), \
                ("a full/delta request should have been sent\n"+log.read_text())
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
    print(f"semantic delta PTY passed: {cols}x{rows}")
