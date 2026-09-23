#!/usr/bin/env python3
"""`.editorconfig` `end_of_line`: opening a file under an .editorconfig that
sets `end_of_line = crlf` makes a subsequent `:w` write CRLF endings, even
though the file was loaded as LF. Verified end-to-end via a PTY + disk bytes."""
import fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(80,14),(120,24)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-ec-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        (root/".editorconfig").write_text("root = true\n[*]\nend_of_line = crlf\n")
        f=root/"a.txt"; f.write_bytes(b"one\ntwo\n")  # loaded as LF
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"))
            os.execv(binary,[binary,str(f)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        def drain(seconds=.4):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                r,_,_=select.select([fd],[],[],.05)
                if r:
                    try:
                        if not os.read(fd,65536):break
                    except OSError:break
        try:
            drain(.6)
            # Make an edit (append a char) so the buffer is modified, then save.
            os.write(fd,b"oX\x1b"); drain(.3)   # open line below, type X, Esc
            os.write(fd,b":w\r"); drain(.5)
            os.write(fd,b":q!\r"); drain(.3)
            end=time.monotonic()+3
            while time.monotonic()<end:
                done,status=os.waitpid(pid,os.WNOHANG)
                if done:
                    assert os.waitstatus_to_exitcode(status)==0;pid=None;break
                time.sleep(.05)
            assert pid is None,"Editor failed to exit"
        finally:
            if pid:
                os.kill(pid,signal.SIGKILL);os.waitpid(pid,0)
            os.close(fd)
        data=f.read_bytes()
        assert b"\r\n" in data, f"expected CRLF endings on save, got {data!r}"
        assert b"one\r\n" in data and b"two\r\n" in data, f"lines use CRLF: {data!r}"
    print(f"editorconfig eol PTY passed: {cols}x{rows}")
