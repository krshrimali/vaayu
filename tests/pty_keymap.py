#!/usr/bin/env python3
"""User key remaps ([[keymap]]): a single-key normal remap (Y->y$), a leader
remap (,w -> :w), and an insert remap (<C-l> -> text). Driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(60,14),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-keymap-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\nleader=","\n'
            '[[keymap]]\nmode="n"\nlhs="Y"\nrhs="y$"\n'
            '[[keymap]]\nmode="n"\nlhs="<leader>w"\nrhs=":w<CR>"\n'
            '[[keymap]]\nmode="i"\nlhs="<C-l>"\nrhs="INSERTED"\n'
        )
        f=root/"f.txt"; f.write_text("hello world\n")
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(f)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.25):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                ready,_,_=select.select([fd],[],[],min(.02,max(0,end-time.monotonic())))
                if ready:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.2):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        try:
            drain(.4)
            # Y (remapped to y$) yanks to EOL; $ then p pastes it after the line.
            key("Y",.2); key("$",.2); key("p",.3)
            assert "worldhello" in text(), ("Y->y$ remap + paste failed\n"+text())
            # Leader remap ,w saves the (now modified) buffer.
            key(",w",.3)
            assert "worldhello" in f.read_text(), ("leader ,w -> :w did not save\n"+f.read_text())
            # Insert remap <C-l> inserts text.
            key("o",.2)          # open a new line in insert mode
            key("\x0c",.2)       # Ctrl-L -> "INSERTED"
            key("\x1b",.2)
            assert "INSERTED" in text(), ("insert <C-l> remap failed\n"+text())
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
    print(f"keymap PTY passed: {cols}x{rows}")
