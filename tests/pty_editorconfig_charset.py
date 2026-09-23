#!/usr/bin/env python3
"""`.editorconfig` `charset`: a file under `charset = utf-16le` is re-encoded to
UTF-16LE (BOM + 2-byte code units) on `:w`, overriding its detected UTF-8
encoding. Verified end-to-end via a PTY + disk bytes."""
import fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(80,14),(120,24)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-ecc-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        (root/".editorconfig").write_text("root = true\n[*]\ncharset = utf-16le\n")
        f=root/"a.txt"; f.write_bytes(b"hi\n")  # plain UTF-8 on load
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
            drain(.5)
            os.write(fd,b"A!\x1b"); drain(.3)   # modify so save writes
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
        # UTF-16LE BOM, then 'h' as 0x68 0x00.
        assert data[:2]==b"\xff\xfe", f"UTF-16LE BOM expected: {data!r}"
        assert b"h\x00i\x00" in data, f"UTF-16LE code units expected: {data!r}"
    print(f"editorconfig charset PTY passed: {cols}x{rows}")
