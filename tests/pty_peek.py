#!/usr/bin/env python3
"""Floating peek windows (goto-preview style): gpd / ,pr / ,pk open a
bordered float over the buffer from a real (mock) language server's
answer, j/k drive it, Enter jumps, Tab unfocuses, Esc / moving the
cursor closes it -- driven through a real PTY at several sizes."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-peek-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-peek-cfg-") as cfg:
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
        main_rs=root/"main.rs"
        main_rs.write_text("one_alpha\ntwo_beta\nthree_gamma\nfour_delta\ntarget_line\nsix_zeta\n")
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
        def float_rows():
            return [l for l in screen.display if "│" in l]
        try:
            drain(.3)
            assert wait_for(lambda: "W" in text()), ("mock LSP never became ready\n"+text())

            # gpd: definition (line 5, "target_line") previewed in a float
            # under the cursor, two lines of context above it.
            key("gpd")
            assert wait_for(lambda: "╭─ Definition" in text()), ("gpd should open a Definition float\n"+text())
            body="\n".join(float_rows())
            assert "   5 target_line" in body and "   3 three_gamma" in body, ("float should preview the target\n"+text())
            assert "╰─ q close" in text(), ("focused float shows its key hints\n"+text())
            # The float is modal: x must not reach the buffer.
            key("x",.2)
            key("\x1b",.2)
            assert "╭" not in text() and "╰" not in text(), ("Esc should close the float\n"+text())
            assert "one_alpha" in text(), ("keys sent to the float must not edit the buffer\n"+text())
            key("x",.2)
            assert "ne_alpha" in text() and "one_alpha" not in text(), ("cursor should not have moved\n"+text())
            key("u",.2)

            # ,pr: references listed above a preview of the selection.
            key(",pr")
            assert wait_for(lambda: "╭─ References — 3" in text()), ("',pr' should open a References float\n"+text())
            body="\n".join(float_rows())
            for l in ("main.rs:2  two_beta", "main.rs:4  four_delta", "main.rs:6  six_zeta"):
                assert l in body, (f"reference list should contain {l!r}\n"+text())
            assert "── 1/3" in text(), ("rule shows the selection index\n"+text())
            key("j",.2)
            assert "── 2/3" in text(), ("j selects the next reference\n"+text())
            key("\r",.3)
            assert "╭" not in text(), ("Enter should close the float\n"+text())
            key("x",.2)
            assert "our_delta" in text() and "four_delta" not in text(), ("Enter should jump to the second reference\n"+text())
            key("u",.2)
            key("gg",.2)

            # ,pk: hover text float; Tab unfocuses it, moving the cursor closes it.
            key(",pk")
            assert wait_for(lambda: "╭─ Hover" in text() and "fixture hover" in text()), ("',pk' should open a Hover float\n"+text())
            key("\t",.2)
            assert ",pf focus" in text(), ("Tab should unfocus the float\n"+text())
            key("j",.2)
            assert "╭" not in text(), ("moving the cursor closes an unfocused float\n"+text())

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
    print(f"Peek float PTY passed: {cols}x{rows}")
