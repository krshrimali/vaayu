#!/usr/bin/env python3
"""Tree-sitter textobjects: dif clears a function body, daf deletes the whole
function, on a real Rust file through a PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-tobj-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"code.rs"; f.write_text("fn foo() {\n    let x = MARKERBODY;\n}\nstruct Keep {}\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(f)])
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
        try:
            drain(.5)  # tree-sitter parse
            key("2G",.2)              # into the function body
            key("dif",.3)            # clear the body
            assert "MARKERBODY" not in text(), ("dif should clear the body\n"+text())
            assert "fn foo" in text(), ("dif should keep the signature\n"+text())
            key("u",.3)              # undo
            assert "MARKERBODY" in text(), ("undo should restore the body\n"+text())
            key("2G",.2); key("daf",.3)  # delete the whole function
            assert "fn foo" not in text() and "MARKERBODY" not in text(), \
                ("daf should delete the function\n"+text())
            assert "struct Keep" in text(), ("other items remain\n"+text())
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
    print(f"textobjects PTY passed: {cols}x{rows}")
