#!/usr/bin/env python3
"""Multiple cursors: Ctrl-N adds a cursor at the next occurrence of the word
under the cursor (status line counts them, secondaries are drawn reversed);
a change typed once lands at every cursor and undoes as one step; ,mj stacks
cursors down a column, and Insert-mode typing at line ends draws each
secondary past its line's end; Visual-block Ctrl-N makes a column of
cursors; Esc collapses them. Driven through a real PTY at several sizes."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
SRC="foo bar foo\nfoo baz\nnone\n"
for cols,rows in [(60,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-mc-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text('jk_escape=false\nnumber=false\n')
        proj=root/"proj"; proj.mkdir()
        f=proj/"a.txt"; f.write_text(SRC)
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(proj)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(root/"config"),HOME=str(root))
            os.execv(binary,[binary,str(f)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder("utf-8")("replace")
        def drain(seconds=.3):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                ready,_,_=select.select([fd],[],[],min(.02,max(0,end-time.monotonic())))
                if ready:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(s,seconds=.3):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        def wait_for(pred,what,seconds=3):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                drain(.05)
                if pred(text()):return
            raise AssertionError(what+"\n"+text())
        def row_of(prefix):
            # The text starts after the (sign) gutter.
            for y,line in enumerate(screen.display):
                if line.lstrip().startswith(prefix):return y
            raise AssertionError("no row starting with "+repr(prefix)+"\n"+text())
        def gutter(y):return len(screen.display[y])-len(screen.display[y].lstrip())
        def reversed_cols(y):
            # Reversed cells on a text row, as text columns.
            return [x-gutter(y) for x in range(cols) if screen.buffer[y][x].reverse]
        def wait_disk(want,what):
            end=time.monotonic()+3
            while time.monotonic()<end:
                if f.read_text()==want:return
                drain(.05)
            raise AssertionError(what+": "+repr(f.read_text())+"\n"+text())
        try:
            drain(.8)
            wait_for(lambda t:"foo bar foo" in t,"initial content")
            # 1. Ctrl-N twice: three cursors on the three `foo`s.
            key("\x0e",.2); key("\x0e",.3)
            wait_for(lambda t:"3 cursors" in t,"status line should count the cursors")
            y0=row_of("foo bar foo"); y1=row_of("foo baz")
            # The primary (last added, on line 2) is the terminal cursor; the
            # other two are drawn reversed.
            assert reversed_cols(y0)==[0,8], ("secondaries on line 1\n"+text()+"\n"+str(reversed_cols(y0)))
            assert (screen.cursor.y,screen.cursor.x-gutter(y1))==(y1,0), ("primary on line 2: "+str((screen.cursor.y,screen.cursor.x)))
            # 2. One change, every cursor; one undo step.
            key("ciwqux",.3); key("\x1b",.3)
            wait_for(lambda t:"qux bar qux" in t and "qux baz" in t,"ciw should change every occurrence")
            key("\x1b",.3)
            wait_for(lambda t:"cursors" not in t.splitlines()[-2] and "multiple cursors cleared" in t,"Esc in Normal collapses")
            assert reversed_cols(row_of("qux bar qux"))==[], ("no secondaries left\n"+text())
            key("u",.3)
            wait_for(lambda t:"foo bar foo" in t and "foo baz" in t,"a single u undoes the whole change")
            # 3. ,mj down a column, then append at each line's end.
            key("gg0",.2); key(",mj",.2); key(",mj",.3)
            wait_for(lambda t:"3 cursors" in t,",mj should add cursors below")
            key("A!",.4)
            wait_for(lambda t:"foo bar foo!" in t and "foo baz!" in t and "none!" in t,"Insert typing at every cursor")
            y0=row_of("foo bar foo!"); y1=row_of("foo baz!")
            assert reversed_cols(y0)==[12] and reversed_cols(y1)==[8], ("secondaries past the line ends\n"+text()+str((reversed_cols(y0),reversed_cols(y1))))
            key("\x1b",.2); key("\x1b",.2)
            key(":w\r",.4)
            wait_disk("foo bar foo!\nfoo baz!\nnone!\n","the append should be saved")
            # 4. Visual-block Ctrl-N: a column of cursors; x deletes there.
            key("gg0l\x16jj",.3); key("\x0e",.3)
            wait_for(lambda t:"3 cursors" in t and "V-BLOCK" not in t,"block Ctrl-N should leave Visual with 3 cursors")
            key("x",.3)
            wait_for(lambda t:"fo bar foo!" in t and "fo baz!" in t and "nne!" in t,"x at every cursor")
            key("\x1b",.2)
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
    print(f"multicursor PTY passed: {cols}x{rows}")
