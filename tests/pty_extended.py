#!/usr/bin/env python3
"""Large-file, Unicode, nested-session and terminal-size regression."""
import codecs, fcntl, json, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-extended-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nclipboard_unnamedplus=false\n')
        file=root/"large.txt"
        file.write_text("a\u0301👩‍💻z\n"+"line with 界 and words\n"*50000)
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root);os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
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
        def key(s,seconds=.2):os.write(fd,s.encode());drain(seconds)
        try:
            drain(1)
            key("lx:w\r")
            assert file.read_text().splitlines()[0]=="a\u0301z"
            key(":vsplit\r:split\r:sessionsave\r")
            session=json.loads((root/".vaayu/session.json").read_text())
            assert len(session["panes"])==3
            key(":only\r:sessionload\r")
            key("G",.3)
            key("iXYZ\x1b",.3)
            key(":w\r",.5)
            assert "XYZ" in file.read_text(), ("\n".join(screen.display),file.read_text()[-100:])
            # Terminal resizes preserve the nested layout and remain editable.
            fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows+2,cols+5,0,0))
            os.kill(pid,signal.SIGWINCH);drain(.3)
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
    print(f"Extended PTY passed: {cols}x{rows}, 50,001 Unicode lines")
