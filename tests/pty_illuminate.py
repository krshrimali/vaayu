#!/usr/bin/env python3
"""illuminate: with a language server attached, letting the cursor rest fires
CursorHold and auto-requests documentHighlight; the returned occurrences get a
blue background. The mock highlights line 0 and line 2, so a blue cell in row 2
(which carries no diagnostic) proves illuminate ran automatically — no manual
command. Driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-illum-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-illum-cfg-") as cfg:
        root=pathlib.Path(proj); cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        log=root/"lsp.jsonl"
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            'updatetime_ms=50\n'
            '[lsp.fixture]\n'
            f'cmd = ["python3", "{mock_lsp}", "{log}"]\n'
            'filetypes = ["rust"]\n')
        main_rs=root/"main.rs"
        main_rs.write_text("let value = 100;\nsecond line here\nreturn code now;\nlast\n")
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
        def text():return "\n".join(screen.display)
        # The mock highlights line 0 chars [4,10) and line 2 chars [0,6);
        # painted cells carry a non-default background (pyte doesn't name the
        # exact DarkBlue reliably, so test "not default" like the sibling test).
        def line0_lit():
            return any(screen.buffer[0][x].bg!="default" for x in range(4,10))
        def line2_lit():
            return any(screen.buffer[2][x].bg!="default" for x in range(0,6))
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
            # Move the cursor onto a symbol and let it rest; illuminate should
            # auto-request highlights and paint both occurrences.
            key("gg",.2); key("l",.15)   # move -> re-arm the hold timer
            assert wait_for(line2_lit), \
                ("illuminate should highlight the second occurrence (row 2)\n"+text())
            assert line0_lit(), ("first occurrence (row 0) should also be lit\n"+text())
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
    print(f"illuminate PTY passed: {cols}x{rows}")
