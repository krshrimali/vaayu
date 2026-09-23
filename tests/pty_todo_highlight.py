#!/usr/bin/env python3
"""`:set todohighlight` recolors TODO/FIXME/etc. keywords inside comments
(TODO -> yellow, FIXME -> red) while leaving the same word in code alone.
PTY-driven against the binary (tree-sitter comment detection)."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
CONTENT="// TODO polish\nlet TODO = 1;\n// FIXME later\nfn main() {}\n"
YELLOW="ffff00"; RED="ff0000"
for cols,rows in [(80,14),(120,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-todo-") as tmp:
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
        # Color of the first char of the word starting at column `c0` on row `y`.
        def fg_at(y,c0):return screen.buffer[y][c0].fg
        def find_col(y,word):
            row="".join(screen.buffer[y][x].data for x in range(cols))
            return row.find(word)
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        try:
            drain(.5)
            key(":set todohighlight\r",.5)
            # The comment TODO (row 0) turns yellow.
            tcol=find_col(0,"TODO")
            assert wait_for(lambda: tcol>=0 and fg_at(0,tcol)==YELLOW), \
                ("comment TODO should be yellow\n"+text()+f"\nfg={fg_at(0,tcol)}")
            # The comment FIXME (row 2) turns red.
            fcol=find_col(2,"FIXME")
            assert fcol>=0 and fg_at(2,fcol)==RED, \
                ("comment FIXME should be red\n"+text()+f"\nfg={fg_at(2,fcol)}")
            # The `TODO` identifier in code (row 1) is NOT yellow.
            ccol=find_col(1,"TODO")
            assert ccol>=0 and fg_at(1,ccol)!=YELLOW, \
                ("code TODO must not be highlighted\n"+text()+f"\nfg={fg_at(1,ccol)}")
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
    print(f"todo-highlight PTY passed: {cols}x{rows}")
