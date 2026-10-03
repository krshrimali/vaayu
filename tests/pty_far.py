#!/usr/bin/env python3
"""Reviewed project-wide replace (`,sr` / `:far`): typing the search,
replacement and file-glob fields lists every match grouped by file as the
line it becomes (plus a -/+ preview where there is room); a whole file can
be switched off; R writes only the selected matches; U reverts the apply;
the screen keeps its state across close/reopen. Driven through a real PTY
at several sizes."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
A="let oldname = 1;\nfn f() -> i32 { oldname }\n"
B="oldname\n"
MD="oldname in notes\n"
for cols,rows in [(70,16),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-far-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        proj=root/"proj"; (proj/"src").mkdir(parents=True)
        a=proj/"src/a.rs"; a.write_text(A)
        b=proj/"src/b.rs"; b.write_text(B)
        md=proj/"notes.md"; md.write_text(MD)
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(proj)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"),HOME=str(root))
            os.execv(binary,[binary,str(a)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.3):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                ready,_,_=select.select([fd],[],[],min(.02,max(0,end-time.monotonic())))
                if ready:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.3):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        def wait_for(pred,what,seconds=3):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                drain(.05)
                if pred(text()):return
            raise AssertionError(what+"\n"+text())
        def wait_disk(path,want,what):
            end=time.monotonic()+3
            while time.monotonic()<end:
                if path.read_text()==want:return
                drain(.05)
            raise AssertionError(what+": "+repr(path.read_text())+"\n"+text())
        try:
            drain(.8)
            assert "oldname" in text(), ("initial content\n"+text())
            # 1. Open the screen and fill in the three fields.
            key(",sr",.4)
            wait_for(lambda t:"Search & replace" in t and "▸Search:" in t,"replace screen should open on the search field")
            key("oldname",.2); key("\t",.1); key("newname",.2); key("\t",.1); key("*.rs",.2); key("\r",.1)
            wait_for(lambda t:"3 match(es) in 2 file(s)" in t and "3 selected" in t,"should list the .rs matches only")
            t=text()
            assert "notes.md" not in t, ("the glob must exclude notes.md\n"+t)
            assert "[x] src/a.rs  (2/2)" in t and "[x] src/b.rs  (1/1)" in t, ("file rows\n"+t)
            assert "let newname = 1;" in t, ("rows preview the replaced line\n"+t)
            if rows>=24:
                assert "- let oldname = 1;" in t and "+ let newname = 1;" in t, ("-/+ preview\n"+t)
            # 2. Switch b.rs off: jump to its header and toggle.
            key("n",.2); key(" ",.3)
            wait_for(lambda t:"[ ] src/b.rs  (0/1)" in t and "2 selected" in t,"b.rs should be deselected")
            # 3. Apply: only a.rs changes; nothing was written to the others.
            key("R",.5)
            wait_disk(a,A.replace("oldname","newname"),"a.rs should be rewritten")
            assert b.read_text()==B and md.read_text()==MD, "deselected / filtered files must be untouched"
            wait_for(lambda t:"Replaced 2 match(es) in 1 file(s)" in t,"apply should be reported")
            wait_for(lambda t:"1 match(es) in 1 file(s)" in t,"the rescan should show what is left")
            # 4. U reverts the apply on disk.
            key("U",.5)
            wait_disk(a,A,"U should restore a.rs")
            wait_for(lambda t:"Undid the replace in 1 file(s)" in t,"undo should be reported")
            # 5. Close, then reopen: the fields come back.
            key("q",.3)
            wait_for(lambda t:"Search & replace" not in t and "let oldname = 1;" in t,"q should return to the buffer")
            key(":far\r",.5)
            wait_for(lambda t:"oldname" in t and "newname" in t and "*.rs" in t and "match(es)" in t,"reopen keeps the fields")
            key("\x1b",.2); key("\x1b",.2); key("q",.3)
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
    print(f"far PTY passed: {cols}x{rows}")
