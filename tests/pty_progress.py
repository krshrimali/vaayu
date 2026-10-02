#!/usr/bin/env python3
"""Fidget-style progress stack: LSP $/progress tokens and :make jobs show
as rows in the bottom-right corner just above the status line (spinner,
title, dimmed source), turn into a ✓/✗ when they end, fade out after a
couple of seconds, and stay hidden with :set noprogress -- driven through
a real PTY at several terminal sizes."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
SPINNER="⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-progress-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-progress-cfg-") as cfg:
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
        def text():
            return "\n".join(screen.display)
        def wait_for(pred, timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():
                    return True
                drain(.05)
            return False
        # Status line is rows-2, message line rows-1: the stack's newest
        # row is the last content row, right-aligned.
        def stack_row(i=0):
            return screen.display[rows-3-i]
        def stack_rows():
            return [screen.display[r] for r in range(rows-3, rows-3-(rows-2)//2, -1)]
        try:
            drain(.3)
            assert wait_for(lambda: stack_row().rstrip().endswith("fixture")
                            and any(c in stack_row() for c in SPINNER)), \
                ("an active LSP progress token should show bottom-right with a spinner and its server\n"+text())
            assert "Indexing" in stack_row(), ("the token's title should show\n"+text())
            if cols>=100:
                assert "Indexing 50% — halfway" in stack_row(), \
                    ("title, percentage and message should show when there's room\n"+text())
            assert stack_row()[1:cols//3].strip()=="", \
                ("the stack belongs in the right part of the screen\n"+text())
            frame=stack_row()
            assert wait_for(lambda: stack_row()!=frame and stack_row().rstrip().endswith("fixture")), \
                ("the spinner should animate with no input\n"+text())
            key("K",.3)  # the mock ends the token on hover
            assert wait_for(lambda: "✓" in stack_row() and "fixture" in stack_row()), \
                ("an ended token should turn into a ✓\n"+text())
            assert wait_for(lambda: "fixture" not in stack_row(), timeout=4), \
                ("a finished task should fade out on its own\n"+text())
            key("\x1b")

            key(":make sleep 1.2\r",.1)
            assert wait_for(lambda: any("sleep 1.2" in r and r.rstrip().endswith("make") for r in stack_rows())), \
                ("a running :make should show its command with source 'make'\n"+text())
            assert wait_for(lambda: any("✓" in r and r.rstrip().endswith("make") for r in stack_rows()), timeout=4), \
                ("a finished :make should show ✓\n"+text())
            key("\x1b")
            assert wait_for(lambda: not any(r.rstrip().endswith("make") for r in stack_rows()), timeout=4), \
                ("the finished :make row should go away\n"+text())

            key(":set noprogress\r")
            key(":make sleep 0.8\r",.1)
            drain(.6)
            assert not any(r.rstrip().endswith("make") for r in stack_rows()), \
                (":set noprogress should hide the stack\n"+text())
            drain(.8)
            key("\x1b")
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
    print(f"Progress stack PTY passed: {cols}x{rows}")
