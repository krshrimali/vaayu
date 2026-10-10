#!/usr/bin/env python3
"""Rapid Ctrl-W focus changes between the editor, file tree and outline."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
mock_lsp=str((pathlib.Path(__file__).parent/"mock_lsp.py").resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-outline-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-outline-cfg-") as cfg:
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
        main_rs=root/"main.rs"; main_rs.write_text("fn main() {\n    body();\n}\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(cfg),XDG_DATA_HOME=str(root/"data"))
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
        def right_text():
            half = cols // 2
            return "\n".join(row[half:] for row in screen.display)
        try:
            def wait_for(predicate, timeout=10):
                end=time.monotonic()+timeout
                while time.monotonic()<end:
                    if predicate():
                        return True
                    drain(.15)
                return False
            drain(.3)
            # Wait for the mock server's published diagnostic (a gutter
            # 'W' on line 1) as a readiness proxy, the same way
            # regression.rs's mock-LSP tests wait for diagnostics before
            # issuing a request -- otherwise ,lO can race a client that
            # hasn't finished initializing yet and get "no server" instead
            # of ever retrying.
            assert wait_for(lambda: "W" in screen.display[0][:2]), \
                ("mock LSP client never became ready\n"+"\n".join(screen.display))
            key(",lo",.6)
            assert wait_for(lambda: "symbol" in right_text()), \
                ("outline sidebar never showed the symbol\n"+right_text())
            assert "fn" in right_text(), ("outline sidebar missing the symbol kind\n"+right_text())
            def divider():
                return next(row.index('│') for row in screen.display[:rows-2] if '│' in row)
            def chord(direction,gap,held):
                suffix=chr(ord(direction)&31) if held else direction
                if gap:
                    os.write(fd,b'\x17');drain(gap);key(suffix,.04)
                else:key('\x17'+suffix,.04)
            for panel in ['outline','tree']:
                if panel=='tree':
                    key(',lo');key(',e',.3)
                border=divider()
                toward_editor='h' if panel=='outline' else 'l'
                toward_panel='l' if panel=='outline' else 'h'
                for held in [False,True]:
                    for gap in [0,.001,.005,.02,.1]:
                        chord(toward_editor,gap,held)
                        assert (screen.cursor.x<border)==(panel=='outline'), (panel,held,gap,'editor',text())
                        chord(toward_panel,gap,held)
                        assert (screen.cursor.x>border)==(panel=='outline'), (panel,held,gap,'sidebar',text())
                key(('\x17'+toward_editor+'\x17'+toward_panel)*8,.2)
                assert (screen.cursor.x>border)==(panel=='outline'), ('queued chord burst',panel,text())
            key(":qa\r")
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
    print(f"Sidebar navigation PTY passed: {cols}x{rows}")
