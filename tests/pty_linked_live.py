#!/usr/bin/env python3
""":linkededit with no name starts LIVE linked editing: it enters Insert at the
range under the cursor and mirrors every keystroke into the other linked
range(s) (no Tab, no second command). The mock links line 0 and line 2."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(80,24),(140,40)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-ll-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-ll-cfg-") as cfg:
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
            key(":linkededit\r",.4)   # no name -> live session; enters Insert
            assert wait_for(lambda: "INSERT" in text()), \
                ("live linked editing should enter Insert mode\n"+text())
            key("X",.3)               # type in the line-0 range
            # The line-2 range mirrors live, without Tab: both are now "abcX".
            assert wait_for(lambda: text().count("abcX")>=2), \
                ("the keystroke should mirror live into the other range\n"+text())
            assert "middle" in text(), ("the unlinked line is untouched\n"+text())
            key("\x1b",.3)            # Esc ends the session and leaves Insert
            assert wait_for(lambda: "INSERT" not in text()), \
                ("Esc should leave Insert / end the session\n"+text())
            key(":w\r",.3)
            assert main_rs.read_text()=="abcX\nmiddle\nabcX\n", \
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
    print(f"linked live editing PTY passed: {cols}x{rows}")
