#!/usr/bin/env python3
""":linkededit renames every LSP linked-editing range at once (e.g. an open/
close tag pair). The mock links line 0 and line 2, so renaming updates both.
Driven through a real PTY with the mock language server."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-le-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-le-cfg-") as cfg:
        root=pathlib.Path(proj); cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        log=root/"lsp.jsonl"
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            '[lsp.fixture]\n'
            f'cmd = ["python3", "{mock_lsp}", "{log}"]\n'
            'filetypes = ["rust"]\n')
        main_rs=root/"main.rs"; main_rs.write_text("abc\nmiddle\nabc\n")
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
        def wait_for(pred,timeout=5.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.4)
            assert wait_for(lambda: "W" in screen.display[0][:2]), \
                ("mock LSP never became ready\n"+text())
            key(":linkededit XYZ\r",.5)
            # Both linked ranges (rows 0 and 2) become XYZ; middle is untouched.
            assert wait_for(lambda: text().count("XYZ")>=2), \
                ("both linked ranges should be renamed\n"+text())
            assert "middle" in text(), ("the unlinked line is untouched\n"+text())
            assert "abc" not in text(), ("original text should be replaced\n"+text())
            key(":w\r",.3)
            assert main_rs.read_text()=="XYZ\nmiddle\nXYZ\n", \
                ("on-disk content after save: %r"%main_rs.read_text())
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
    print(f"linkededit PTY passed: {cols}x{rows}")
