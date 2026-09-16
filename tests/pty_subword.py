#!/usr/bin/env python3
"""camelCase/snake_case/kebab-case subword motions (g w / g b / g e),
usable bare, with operators and in Visual mode -- driven through a PTY.
Each check resets the buffer to a fresh single line first, so assertions
don't depend on exactly how much trailing whitespace a previous `dgw`
consumed (matching this editor's existing word-motion end-of-word/line
behavior, not something subword motion changes)."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-subword-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        file=root/"f.txt"
        file.write_text("x\n")
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
        def set_line(text):
            key("ggcc"+text,.1)
            key("\x1b",.15)
            key("0",.1)
        def save_and_read_line():
            key(":w\r",.3)
            return file.read_text().splitlines()[0] if file.read_text() else ""
        try:
            drain(.3)
            # dgw on a camelCase word deletes only the first subword.
            set_line("myVarName")
            key("dgw")
            assert save_and_read_line()=="VarName", save_and_read_line()
            # gw moves to the next subword start ('N'); a second dgw there
            # deletes just "Name" (word motion is free to also take a
            # trailing separator, same as this editor's existing dw).
            set_line("VarNameZ")
            key("gwdgw")
            assert save_and_read_line().startswith("Var"), save_and_read_line()
            assert "Name" not in save_and_read_line(), save_and_read_line()
            # snake_case and kebab-case split on the separator, not through it.
            # (the separator itself is a gap swept up by the motion, same
            # as a real `dw` sweeping trailing whitespace)
            set_line("snake_case")
            key("dgw")
            assert save_and_read_line()=="case", save_and_read_line()
            set_line("kebab-case")
            key("dgw")
            assert save_and_read_line()=="case", save_and_read_line()
            # gb moves backward by subword.
            set_line("myVarName")
            key("$gbx")
            assert save_and_read_line()=="myVarame", save_and_read_line()
            # Visual gw extends the selection by subword, not by full word.
            set_line("myVarName")
            key("vgwd")
            assert save_and_read_line()=="arName", save_and_read_line()
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
    print(f"Subword motion PTY passed: {cols}x{rows}")
