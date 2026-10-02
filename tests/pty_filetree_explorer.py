#!/usr/bin/env python3
"""File tree as a neo-tree style explorer, driven through a real PTY:
`,e` opens it pinned to the left edge at a fixed width, focuses an open
one, and closes a focused one; leader keys still work from inside it;
`/` fuzzy-finds across the whole project (not just expanded dirs); a
mouse double-click opens a file; `?` shows the key reference; and `:q`
from the last editing pane quits instead of leaving the tree alone."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(100,24),(160,40)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-explorer-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-explorer-cfg-") as cfg:
        root=pathlib.Path(proj)
        cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\ntree_icons=false\n'
            'lsp.rust_analyzer.enabled=false\n'
        )
        for d in ["src/core","src/ui","docs"]:
            (root/d).mkdir(parents=True)
        (root/"src/core/engine.rs").write_text("ENGINE BODY\n")
        (root/"src/ui/view.rs").write_text("VIEW BODY\n")
        (root/"docs/guide.md").write_text("GUIDE BODY\n")
        readme=root/"README.md"; readme.write_text("README BODY\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(cfg))
            os.execv(binary,[binary,str(readme)])
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
        TW=32
        def tree():return "\n".join(r[:TW] for r in screen.display)
        def editor():return "\n".join(r[TW+1:] for r in screen.display)
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        def click(x,y):
            # SGR mouse press + release (1-based coordinates).
            os.write(fd,f"\x1b[<0;{x+1};{y+1}M\x1b[<0;{x+1};{y+1}m".encode());drain(.1)
        try:
            drain(.4)
            assert "README BODY" in text(), text()
            # ,e opens the tree pinned left at a fixed width, focused.
            key(",e",.4)
            assert wait_for(lambda:"README.md" in tree()), ("tree did not open on the left\n"+text())
            assert all(r[TW]=="│" for r in screen.display[1:rows-3]), \
                ("the sidebar's border should sit at column %d\n%s"%(TW,text()))
            assert "README BODY" in editor(), ("editor should stay beside the tree\n"+text())
            # / fuzzy-finds across the project: engine.rs lives in a
            # never-expanded directory.
            assert "engine.rs" not in tree()
            key("/",.3); key("engine",.5)
            assert wait_for(lambda:"engine.rs" in tree()), ("project-wide filter missed engine.rs\n"+text())
            assert "core" in tree() and "src" in tree(), ("hit should show its ancestors\n"+text())
            assert "guide.md" not in tree(), ("non-matching files must be hidden\n"+text())
            key("\r",.2)   # stop typing (cursor is on the best hit)
            key("\r",.4)   # open it
            assert wait_for(lambda:"ENGINE BODY" in editor()), ("Enter did not open the hit\n"+text())
            # ,e from the editor focuses the open tree; Esc there clears the filter.
            key(",e",.3)
            key("\x1b",.3)
            assert wait_for(lambda:"guide.md" in tree() or "docs" in tree()), \
                ("Esc should clear the filter back to the full tree\n"+text())
            # Leader keys work from the tree: ,ff opens the picker, and the
            # chosen file lands in the editing pane, not the tree.
            key(",ff",.4); key("view",.4); key("\r",.5)
            assert wait_for(lambda:"VIEW BODY" in editor()), ("picker from the tree misrouted\n"+text())
            assert "README.md" in tree(), ("tree should still be shown\n"+text())
            # ,e twice (focus, then close) removes it; the editor takes the width.
            key(",e",.3); key(",e",.4)
            assert wait_for(lambda:not any(r[:TW].strip().startswith("README.md") for r in screen.display)), \
                ("second ,e should close a focused tree\n"+text())
            assert "VIEW BODY" in text()[:cols], ("editor should span the screen again\n"+text())
            # Reopen; ? shows the key reference, any other key closes it.
            key(",e",.4)
            key("?",.3)
            assert "collapse all" in tree(), ("? should show the key reference\n"+text())
            key("x",.3)
            assert "collapse all" not in tree(), text()
            # Mouse: double-click README.md opens it.
            y=next(i for i,r in enumerate(screen.display) if "README.md" in r[:TW])
            x=screen.display[y].index("README.md")
            click(x,y); click(x,y); drain(.3)
            assert wait_for(lambda:"README BODY" in editor()), ("double-click did not open\n"+text())
            # :q from the only editing pane quits rather than leaving the tree.
            key(":q\r",.3)
            end=time.monotonic()+3
            while time.monotonic()<end:
                done,status=os.waitpid(pid,os.WNOHANG)
                if done:
                    assert os.waitstatus_to_exitcode(status)==0;pid=None;break
                drain(.05)
            assert pid is None,(":q next to just the tree should quit\n"+text())
        finally:
            if pid:
                os.kill(pid,signal.SIGKILL);os.waitpid(pid,0)
            os.close(fd)
    print(f"File tree explorer PTY passed: {cols}x{rows}")
