#!/usr/bin/env python3
"""New default language definitions (Vim, CSS, HTML, Solidity) get real
syntax highlighting, not just plain text -- driven through a real PTY,
inspecting pyte's per-cell fg attribute directly (the same technique
pty_highlight_colors.py already established) rather than only checking
the editor doesn't crash on these filetypes."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
FIXTURES = {
    "sample.vim": '" a comment\nlet x = 1\n',
    "sample.css": "/* c */\n.a { width: 1px; }\n",
    "sample.html": "<!-- c -->\n<div class=\"a\">text</div>\n",
    "sample.sol": "// c\ncontract Foo { uint x = 1; }\n",
}
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-newlangs-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        for name, content in FIXTURES.items():
            (root/name).write_text(content)
        for name in FIXTURES:
            f=root/name
            pid,fd=pty.fork()
            if pid==0:
                os.chdir(root)
                os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
                os.execv(binary,[binary,str(f)])
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
            def text():
                return "\n".join(screen.display)
            try:
                drain(.4)
                # Row 0's first line always has some highlighted token
                # (the comment marker/text) in every one of these
                # fixtures -- at least one cell must differ from the
                # terminal's own default foreground.
                row = screen.buffer[0]
                colored = any(row[x].fg not in ("default",) for x in range(cols))
                assert colored, \
                    (f"{name} should show real syntax highlighting, not "
                     f"plain text\n"+text())
                key(":qa!\r")
                end=time.monotonic()+3
                while time.monotonic()<end:
                    done,status=os.waitpid(pid,os.WNOHANG)
                    if done:
                        assert os.waitstatus_to_exitcode(status)==0;pid=None;break
                    drain(.05)
                assert pid is None,f"Editor failed to exit for {name}"
            finally:
                if pid:
                    os.kill(pid,signal.SIGKILL);os.waitpid(pid,0)
                os.close(fd)
    print(f"New language highlighting PTY passed: {cols}x{rows}")
