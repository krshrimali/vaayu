#!/usr/bin/env python3
"""Conceal: `:set conceal` with a `[[conceal_rules]]` hiding `**` shows
`**bold**` as `bold` on every line EXCEPT the one the cursor is on (which stays
revealed). Moving the cursor reveals/conceals lines accordingly. PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(80,24),(120,40)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-cc-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nnumber=false\n'
            '[[conceal_rules]]\npattern = "\\\\*\\\\*"\ncchar = ""\n')
        f=root/"a.md"; f.write_text("**bold**\n**more**\n")
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
                r,_,_=select.select([fd],[],[],.05)
                if r:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.3):os.write(fd,s.encode());drain(seconds)
        def line(y):return screen.display[y].strip()
        def wait_for(pred,timeout=4.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.1)
            return False
        try:
            drain(.5)
            assert line(0)=="**bold**" and line(1)=="**more**", \
                ("markers visible before conceal\n"+repr(screen.display[:2]))
            key(":set conceal\r",.4)
            # Cursor on line 0 -> revealed; line 1 -> concealed.
            assert wait_for(lambda: line(1)=="more"), \
                ("off-cursor line should hide the ** markers\n"+repr(screen.display[:2]))
            assert line(0)=="**bold**", \
                ("the cursor line stays revealed\n"+repr(screen.display[:2]))
            key("j",.4)  # move cursor to line 1
            assert wait_for(lambda: line(1)=="**more**"), \
                ("the new cursor line reveals its markers\n"+repr(screen.display[:2]))
            assert line(0)=="bold", \
                ("the line the cursor left is now concealed\n"+repr(screen.display[:2]))
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
    print(f"conceal PTY passed: {cols}x{rows}")
