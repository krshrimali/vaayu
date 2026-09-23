#!/usr/bin/env python3
"""Large-file mode: a file over `large_file_kb` opens and is navigable without
tree-sitter/spell/todo scans. Smoke-tests that the editor renders it and quits
cleanly (no hang), driven via a PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-lf-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        # 50 KiB threshold; the file below is ~500 KiB -> large-file mode.
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nnumber=false\nrainbow=true\nlarge_file_kb=50\n')
        f=root/"big.rs"; f.write_text("fn f() { let x = (1 + 2); }\n"*20000)
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
        def wait_for(pred,timeout=5.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.1)
            return False
        try:
            drain(.6)
            # The file content renders.
            assert wait_for(lambda: "fn f()" in text()), ("large file should render\n"+text())
            # Navigation works (jump to end, then back).
            key("G",.4)
            key("gg",.4)
            assert "fn f()" in text()
            key(":q!\r")
            end=time.monotonic()+4
            while time.monotonic()<end:
                done,status=os.waitpid(pid,os.WNOHANG)
                if done:
                    assert os.waitstatus_to_exitcode(status)==0;pid=None;break
                drain(.05)
            assert pid is None,"Editor failed to exit (possible hang on a large file)"
        finally:
            if pid:
                os.kill(pid,signal.SIGKILL);os.waitpid(pid,0)
            os.close(fd)
    print(f"large-file PTY passed: {cols}x{rows}")
