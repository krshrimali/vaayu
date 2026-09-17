#!/usr/bin/env python3
"""File tree copy: a directory copy that fails partway through (a nested
unreadable file) rolls back, leaving no partial destination directory
behind -- driven through a real PTY."""
import codecs, fcntl, os, pathlib, pty, select, signal, stat, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-filetree-cprb-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-filetree-cprb-cfg-") as cfg:
        root=pathlib.Path(proj)
        cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        (root/"dest").mkdir()
        (root/"src_dir").mkdir()
        opener=root/"opener.txt"; opener.write_text("open me\n")
        (root/"src_dir/ok.txt").write_text("fine\n")
        blocked=root/"src_dir/blocked.txt"; blocked.write_text("denied\n")
        blocked.chmod(0)  # unreadable -- fails the copy partway through
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(cfg))
            os.execv(binary,[binary,str(opener)])
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
        def right_text():
            half = cols // 2
            return "\n".join(row[half:] for row in screen.display)
        try:
            drain(.3)
            key(",ft",.3)
            # nodes sorted: dest/, src_dir/, opener.txt -- cursor starts on dest.
            assert "dest" in right_text() and "src_dir" in right_text(), \
                ("tree missing dest/ or src_dir/\n"+right_text())
            key("j",.2)   # onto src_dir/
            key("y",.2)   # copy it
            key("k",.2)   # back onto dest/
            key("p",.4)   # paste -- copies into dest/, blocked.txt fails it
            assert not (root/"dest"/"src_dir").exists(), \
                ("a failed directory copy should roll back, not leave "
                 "dest/src_dir behind\n"+right_text())
            assert (root/"src_dir"/"ok.txt").exists(), \
                "the original source directory must be untouched"
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
            blocked.chmod(stat.S_IRUSR | stat.S_IWUSR)
    print(f"File tree copy rollback PTY passed: {cols}x{rows}")
