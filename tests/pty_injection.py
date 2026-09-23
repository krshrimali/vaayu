#!/usr/bin/env python3
"""Tree-sitter injection: a ```rust fenced code block in a Markdown file gets
rust syntax highlighting (the `fn` keyword turns cyan), while prose stays plain.
PTY-driven against the binary."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
CONTENT="prose line\n```rust\nfn demo() {}\n```\nmore prose\n"
CYAN="00ffff"
for cols,rows in [(80,14),(120,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-inj-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"doc.md"; f.write_text(CONTENT)
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(f)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.4):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                r,_,_=select.select([fd],[],[],.05)
                if r:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def text():return "\n".join(screen.display)
        def find_row(word):
            for y in range(rows):
                if word in "".join(screen.buffer[y][x].data for x in range(cols)):
                    return y
            return None
        def row_has_cyan(y):
            return y is not None and any(screen.buffer[y][x].fg==CYAN for x in range(cols))
        def wait_for(pred,timeout=4.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.1)
            return False
        try:
            drain(.6)
            # The `fn demo()` line inside the rust fence is cyan-highlighted.
            assert wait_for(lambda: row_has_cyan(find_row("fn demo"))), \
                ("the rust fence should be syntax-highlighted (cyan fn)\n"+text())
            # The prose line is not.
            assert not row_has_cyan(find_row("prose line")), \
                ("markdown prose stays plain\n"+text())
            os.write(fd,b":qa!\r")
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
    print(f"injection PTY passed: {cols}x{rows}")
