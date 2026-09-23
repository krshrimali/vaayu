#!/usr/bin/env python3
"""`.editorconfig` per-file overrides: with `insert_final_newline = true` and
`trim_trailing_whitespace = true` for the edited file, `:w` adds a final newline
and strips trailing whitespace even when the global config leaves them off.
Verified end-to-end via a PTY + disk bytes."""
import fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(80,14),(120,24)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-ecx-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        # Global config leaves the on-save hooks OFF.
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nnumber=false\n'
            'trim_trailing_whitespace=false\ninsert_final_newline=false\n')
        (root/".editorconfig").write_text(
            "root = true\n[*]\ntrim_trailing_whitespace = true\ninsert_final_newline = true\n")
        # No final newline; a trailing-space line.
        f=root/"a.txt"; f.write_bytes(b"hello   \nworld")
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
            # Edit line 2 (not line 1's trailing spaces) so the buffer is dirty
            # while line 1 keeps its trailing whitespace for the trim hook.
            os.write(fd,b"jA!\x1b"); drain(.3)  # -> line 2 becomes "world!"
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
        assert data.endswith(b"\n"), f"final newline added: {data!r}"
        assert b"hello   \n" not in data and b"hello\n" in data, \
            f"trailing whitespace trimmed: {data!r}"
    print(f"editorconfig extras PTY passed: {cols}x{rows}")
