#!/usr/bin/env python3
"""`:foldsyntax` folds function/class bodies via tree-sitter: each multi-line
function collapses to a foldtext row; `zR` opens them. PTY-driven."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
CONTENT=("fn alpha() {\n"
         "    let a = 1;\n"
         "    let b = 2;\n"
         "}\n"
         "fn beta() {\n"
         "    let c = 3;\n"
         "}\n")
for cols,rows in [(80,14),(120,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-folds-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"a.rs"; f.write_text(CONTENT)
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
        def key(s,seconds=.4):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        def wait_for(pred,timeout=4.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.6)
            assert "let a = 1" in text()
            key(":foldsyntax\r",.6)
            # Both function bodies collapse; two foldtext markers appear.
            assert wait_for(lambda: text().count("⋯")>=2), \
                ("foldsyntax should fold both functions\n"+text())
            assert "let a = 1" not in text() and "let c = 3" not in text(), \
                ("function bodies should be hidden\n"+text())
            assert "fn alpha" in text() and "fn beta" in text(), \
                ("function headers stay visible\n"+text())
            key("zR",.4)
            assert wait_for(lambda: "let a = 1" in text() and "⋯" not in text()), \
                ("zR should reopen every fold\n"+text())
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
    print(f"foldsyntax PTY passed: {cols}x{rows}")
