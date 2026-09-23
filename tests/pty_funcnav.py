#!/usr/bin/env python3
"""`]f` / `[f` jump to the next / previous function definition (tree-sitter).
The cursor's reported line in the status ruler is used to verify the jump,
driven through a real PTY on a Rust file."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
import re
def cursor_line(screen):
    # The status ruler shows " <line>:<col> "; scan the bottom rows for the
    # last "N:M" token (the command/message line sits below the ruler).
    for row in reversed(screen.display):
        m=re.findall(r"(\d+):(\d+)", row)
        if m:
            return int(m[-1][0])
    return None
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-fnav-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        f=root/"code.rs"
        f.write_text("fn one() {\n    a();\n}\nfn two() {\n    b();\n}\nfn three() {\n    c();\n}\n")
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
        try:
            drain(.6)  # allow tree-sitter parse
            key("gg",.2)               # top of file (fn one, line 1)
            key("]f",.3)               # -> fn two (line 4)
            assert cursor_line(screen)==4, ("]f should land on fn two (line 4), got %s"%cursor_line(screen))
            key("]f",.3)               # -> fn three (line 7)
            assert cursor_line(screen)==7, ("]f should land on fn three (line 7), got %s"%cursor_line(screen))
            key("[f",.3)               # -> fn two (line 4)
            assert cursor_line(screen)==4, ("[f should land on fn two (line 4), got %s"%cursor_line(screen))
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
    print(f"funcnav PTY passed: {cols}x{rows}")
