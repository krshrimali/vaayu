#!/usr/bin/env python3
"""vim-surround style add/delete/change, driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-surround-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        file=root/"f.txt"
        file.write_text("say hello world\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(file)])
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
        def key(s,seconds=.15):os.write(fd,s.encode());drain(seconds)
        def save_and_read():
            key(":w\r",.3)
            return file.read_text()
        try:
            drain(.3)
            # ysiw" on "hello" (cursor starts on 'say') wraps the inner word.
            key("w")
            key("ysiw\"")
            assert save_and_read()=="say \"hello\" world\n", save_and_read()
            # cs"' changes the quotes to single quotes.
            key("cs\"'")
            assert save_and_read()=="say 'hello' world\n", save_and_read()
            # ds' removes the quotes entirely.
            key("ds'")
            assert save_and_read()=="say hello world\n", save_and_read()
            # Visual S wraps the selection: select "world" and wrap in ().
            key("$")
            key("v")
            for _ in range(4):
                key("h",.05)
            key("S(",.2)
            assert save_and_read()=="say hello ( world )\n", save_and_read()
            # yss* wraps the whole line.
            key("0",.1)
            key("yss*",.2)
            assert save_and_read()=="*say hello ( world )*\n", save_and_read()
            key(":qa\r")
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
    print(f"Surround PTY passed: {cols}x{rows}")
