#!/usr/bin/env python3
"""A diagnostic's own column range gets underlined (pyte's per-cell
`.underscore`), its message shows as virtual text on the cursor's line
with a "[source(code)]" label, and the ",ld"/:diagnostics list shows a
relatedInformation entry as its own separately-jumpable row right after
its parent -- driven through a real PTY against a real (mock) language
server, inspecting pyte's screen state directly rather than just
internal editor state."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-diagrender-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-diagrender-cfg-") as cfg:
        root=pathlib.Path(proj)
        cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        log=root/"lsp.jsonl"
        config = (
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            '[lsp.fixture]\n'
            f'cmd = ["python3", "{mock_lsp}", "{log}", "--diag-on-change"]\n'
            'filetypes = ["rust"]\n'
        )
        (cfg/"vaayu/config.toml").write_text(config)
        # didOpen's diagnostic sits at line 0 chars 0-3 ("one"); appending
        # at end-of-line (not inserting inside it) keeps that range intact
        # so the underline check below still targets the right columns
        # after the edit that triggers didChange.
        main_rs=root/"main.rs"; main_rs.write_text("one\ntwo\nthree\n")
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
        def wait_for(predicate, timeout=10):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if predicate():
                    return True
                drain(.15)
            return False
        try:
            drain(.3)
            assert wait_for(lambda: "W" in screen.display[0][:2]), \
                ("mock LSP client never became ready\n"+text())
            # Dirty the buffer (append at end-of-line, not inside the
            # diagnostic's own range) to trigger didChange -> the mock's
            # richer error diagnostic (source/code/relatedInformation).
            key("A!\x1b",.5)
            assert wait_for(lambda: "E" in screen.display[0][:2]), \
                ("gutter should switch to the new error's marker\n"+text())
            # The content row (row 0 of the buffer pane) is screen row 0
            # here (no tabline/statusline above it in this layout). Find
            # exactly where "one!" starts rather than guessing the
            # gutter's width.
            row = screen.buffer[0]
            row_text = "".join(row[x].data for x in range(cols))
            content_start = row_text.index("one!")
            underlined = [row[content_start + c].underscore for c in range(3)]
            assert all(underlined), \
                (f"columns 0-2 (\"one\") should be underlined, got {underlined}\n"+text())
            not_underlined = row[content_start + 3].underscore
            assert not not_underlined, \
                ("column 3 (just past the diagnostic's range) should not "
                 "be underlined\n"+text())
            # At 40 columns the message itself can be truncated past the
            # label -- the label's presence is what's being checked here.
            assert wait_for(lambda: "[eslint(no-unused-vars)]" in text()), \
                ("the diagnostic's virtual text should show its source/code "
                 "label alongside the message\n"+text())
            key(",ld",.5)
            assert wait_for(lambda: "[eslint(no-unused-vars)]" in text()), \
                ("the diagnostics list should show the same source/code label\n"+text())
            assert wait_for(lambda: "declared here" in text()), \
                ("relatedInformation should show as its own entry\n"+text())
            key("j\r",.4)  # onto the related entry, then jump to it
            assert wait_for(lambda: "2:1" in text() or ":2:" in text() or "two" in screen.display[0]), \
                ("opening the related-information entry should jump to its "
                 "own location (line 1, 0-indexed)\n"+text())
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
    print(f"Diagnostic rendering PTY passed: {cols}x{rows}")
