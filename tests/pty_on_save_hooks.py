#!/usr/bin/env python3
"""On-save hooks (config trim_trailing_whitespace + insert_final_newline) and
the autocommand bus, driven through a real PTY: editing then `:w` writes
trimmed, newline-terminated bytes to disk, and a [[autocmd]] BufWritePre Ex
command runs before the write."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-onsave-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            'trim_trailing_whitespace=true\ninsert_final_newline=true\n'
            '[[autocmd]]\nevent="BufWritePre"\npattern="*.rs"\ncommand="s/TODO/DONE/"\n'
        )
        # A .txt file: trailing spaces on line 1, and (after we edit) a new
        # last line with trailing tab and no final newline.
        f=root/"f.txt"; f.write_text("alpha   \nbeta\n")
        rsf=root/"code.rs"; rsf.write_text("TODO one\n")
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
        try:
            drain(.4)
            # Append a new last line WITH trailing whitespace and no final
            # newline, then save. Trim + final-newline must fix both.
            key("G$",.2)          # end of last line ("beta")
            key("o", .2)          # open a new line below (this file ends with \n)
            key("gamma\t  ", .2)  # type text with trailing tab+spaces
            key("\x1b", .2)       # back to Normal
            key(":w\r", .3)
            disk=f.read_text()
            assert disk=="alpha\nbeta\ngamma\n", repr(disk)
            # Now the autocmd: open the .rs file and save; BufWritePre s/TODO/DONE/
            key(":e code.rs\r", .4)
            key(":w\r", .3)
            rs_disk=rsf.read_text()
            assert rs_disk=="DONE one\n", repr(rs_disk)
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
    print(f"on-save hooks PTY passed: {cols}x{rows}")
