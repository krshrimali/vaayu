#!/usr/bin/env python3
"""A snippet completion with a numbered-stop transform mirror
(`${1:name} => ${1/(.*)/[$1]/}`): editing the stop and tabbing out syncs the
mirror with the transformed text. Driven through a real (mock) LSP over a PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(80,24),(140,40)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-xform-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-xform-cfg-") as cfg:
        root=pathlib.Path(proj); cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        log=root/"lsp.jsonl"
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            '[lsp.fixture]\n'
            f'cmd = ["python3", "{mock_lsp}", "{log}"]\n'
            'filetypes = ["rust"]\n')
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
        def key(s,seconds=.2):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        def wait_for(pred,timeout=10):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.15)
            return False
        try:
            drain(.3)
            assert wait_for(lambda: "W" in screen.display[0][:2]), \
                ("mock LSP never became ready\n"+text())
            key("i",.2)
            key("xfo",.5)  # fuzzy-matches filterText "XFORM"
            assert wait_for(lambda: "xform" in text()), \
                ("completion popup should offer the xform snippet\n"+text())
            key("\r",.4)   # accept -> "name => [name]"
            assert wait_for(lambda: "name => [name]" in screen.display[0]), \
                ("the snippet expands with the transform applied to the default\n"+text())
            key("id",.3)   # type over the selected stop 1 ("name" -> "id")
            assert wait_for(lambda: "id => [name]" in screen.display[0]), \
                ("typing replaces the stop but the mirror is not yet synced\n"+text())
            key("\t",.4)   # Tab: sync stop 1 into its transform mirror
            assert wait_for(lambda: "id => [id]" in screen.display[0]), \
                ("tabbing out syncs the mirror with the transformed new text\n"+text())
            key("\x1b",.2)
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
    print(f"snippet transform PTY passed: {cols}x{rows}")
