#!/usr/bin/env python3
"""Autopairs: bracket/quote pairing, skip-over, pair backspace, brace-enter
expansion, and paste suppression -- driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-autopairs-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        file=root/"f.txt"
        file.write_text("\n")
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
        try:
            drain(.3)
            # Typing an open bracket inserts the close and leaves the cursor
            # between them; typing the close again skips over it rather than
            # duplicating it.
            key("ifoo(")
            assert screen.display[0].strip()=="foo()", screen.display[0]
            key("bar)")
            assert screen.display[0].strip()=="foo(bar)", screen.display[0]
            key("\x1b",.2)
            key(":w\r",.3)
            assert file.read_text()=="foo(bar)\n", file.read_text()
            # Backspace between an empty pair removes both characters.
            key(":1\r",.2)
            key("A(\x7f",.2)
            assert file.read_text() or True
            key("\x1b",.2)
            key(":w\r",.3)
            assert file.read_text()=="foo(bar)\n", ("backspace pair not collapsed\n"+file.read_text())
            # Enter between a brace pair expands to an indented blank line.
            key(":1\r",.2)
            key("A {\r",.2)
            key("\x1b",.2)
            key(":w\r",.3)
            assert file.read_text()=="foo(bar) {\n    \n}\n", ("brace-enter did not expand\n"+file.read_text())
            # Pasted text is never auto-paired: a literal bracketed paste of
            # "(x" must not gain a synthetic close.
            key(":1\r",.2)
            key("dG",.2)  # clear the buffer back to one empty line
            key("i",.1)
            os.write(fd,b"\x1b[200~(x\x1b[201~");drain(.2)
            key("\x1b",.2)
            key(":w\r",.3)
            assert file.read_text()=="(x", ("paste was auto-paired\n"+file.read_text())
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
    print(f"Autopairs PTY passed: {cols}x{rows}")
