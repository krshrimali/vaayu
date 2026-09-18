#!/usr/bin/env python3
"""Active LSP $/progress shows in the per-pane status line (recomputed
fresh every frame from live state), not just the message line -- so an
unrelated action's own message (e.g. a save confirmation) never hides
it, unlike the message-line-only surfacing this had before -- driven
through a real PTY against a real (mock) language server."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-lspprogsl-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-lspprogsl-cfg-") as cfg:
        root=pathlib.Path(proj)
        cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        log=root/"lsp.jsonl"
        config = (
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            '[lsp.fixture]\n'
            f'cmd = ["python3", "{mock_lsp}", "{log}", "--progress"]\n'
            'filetypes = ["rust"]\n'
        )
        (cfg/"vaayu/config.toml").write_text(config)
        main_rs=root/"main.rs"
        main_rs.write_text("fn main() {}\n")
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
        def status_line():
            return screen.display[-2]
        def message_line():
            return screen.display[-1]
        def wait_for(pred, timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():
                    return True
                drain(.05)
            return False
        try:
            drain(.3)
            assert wait_for(lambda: "Indexing" in status_line()), \
                ("progress should show in the status line\n"+status_line())
            # An unrelated action (save) sets its own message -- the
            # status-line progress indicator must survive that, unlike
            # the message line, which does get overwritten. Checks for a
            # quote mark (the save message's format is `"<name>"
            # written`) rather than "written" itself, since a long tmp
            # path can push "written" off the edge of a 40-col terminal.
            before = message_line()
            key(":w\r",.3)
            assert wait_for(lambda: message_line() != before and '"' in message_line()), \
                ("save should set its own message\n"+message_line())
            assert "Indexing" in status_line(), \
                ("progress should still show in the status line after an "
                 "unrelated message overwrote the message line\n"
                 f"status: {status_line()!r}\nmessage: {message_line()!r}")
            key("K",.3)  # hover -- the mock also sends progress "end" here
            assert wait_for(lambda: "Indexing" not in status_line()), \
                ("progress ending should remove the status-line indicator\n"+status_line())
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
    print(f"LSP progress status line PTY passed: {cols}x{rows}")
