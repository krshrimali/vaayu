#!/usr/bin/env python3
"""`:colorscheme` swaps the syntax palette live: a keyword is cyan under the
default scheme and a different color under another. Driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-colo-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"code.rs"; f.write_text("fn main() {}\n")
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
        def row0_fgs():
            return [screen.buffer[0][x].fg for x in range(cols)]
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.6)  # tree-sitter parse
            # Default: the `fn` keyword is cyan.
            assert wait_for(lambda: "00ffff" in row0_fgs()), \
                ("keyword should be cyan under the default scheme\n"+"\n".join(screen.display))
            key(":colorscheme warm\r",.4)
            # After switching, no cell is cyan any more (keyword recolored).
            assert wait_for(lambda: "00ffff" not in row0_fgs()), \
                (":colorscheme should recolor the keyword live\n"+"\n".join(screen.display))
            key(":colorscheme default\r",.4)
            assert wait_for(lambda: "00ffff" in row0_fgs()), \
                ("switching back restores cyan\n"+"\n".join(screen.display))
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
    print(f"colorscheme PTY passed: {cols}x{rows}")
