#!/usr/bin/env python3
"""`:set semantictokens` overlays LSP semantic-token colors. The mock marks
line 0 chars 0-1 as a `keyword` (→ cyan), which tree-sitter wouldn't color
(it's an identifier), so cyan appears only once semantic tokens are enabled.
Driven through a real PTY with the mock (`--semantic`)."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-sem-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-sem-cfg-") as cfg:
        root=pathlib.Path(proj); cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        log=root/"lsp.jsonl"
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            '[lsp.fixture]\n'
            f'cmd = ["python3", "{mock_lsp}", "{log}", "--semantic"]\n'
            'filetypes = ["rust"]\n')
        main_rs=root/"main.rs"; main_rs.write_text("abc = 1;\nmore\n")
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
        def row0_has_cyan():
            return any(screen.buffer[0][x].fg=="00ffff" for x in range(cols))
        def row1_struck():
            return any(screen.buffer[1][x].strikethrough for x in range(cols))
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
            assert not row0_has_cyan(), ("no semantic color before enabling\n"+text())
            assert not row1_struck(), ("no strikethrough before enabling\n"+text())
            key(":set semantictokens\r",.5)
            assert wait_for(row0_has_cyan), \
                ("semantic keyword token should color the text cyan\n"+text())
            # The line-1 token carries the `deprecated` modifier -> struck through.
            assert wait_for(row1_struck), \
                ("a deprecated semantic token should be struck through\n"+text())
            key(":set nosemantic\r",.4)
            assert wait_for(lambda: not row0_has_cyan()), \
                ("disabling should remove the semantic color\n"+text())
            assert not row1_struck(), ("disabling removes the strikethrough\n"+text())
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
    print(f"semantic-tokens PTY passed: {cols}x{rows}")
