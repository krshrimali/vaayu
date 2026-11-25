#!/usr/bin/env python3
"""Right-click menus, double-click selection and clickable breadcrumbs,
driven by real SGR mouse sequences over a PTY with the default config
(breadcrumbs on): double-click a word, right-click it, choose Cut from the
menu with the mouse; Esc closes a menu without touching the buffer; the
file name's breadcrumb lists the top-level functions and jumps to one."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())

def sgr(button, col, row, release=False):
    # 0-based cell -> 1-based xterm SGR (1006) report.
    return f"\x1b[<{button};{col+1};{row+1}{'m' if release else 'M'}"

CONTENT=("fn alpha() {\n"
         "    let hello = world;\n"
         "}\n"
         "fn beta() {}\n")

for cols,rows in [(80,24),(140,40)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-menu-") as tmp:
        root=pathlib.Path(tmp)
        (root/"config/vaayu").mkdir(parents=True)
        (root/"config/vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
        )
        file=root/"f.rs"
        file.write_text(CONTENT)
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
        def send(s,seconds=.15):os.write(fd,s.encode());drain(seconds)
        def text():return "\n".join(screen.display)
        def find(label):
            for y,line in enumerate(screen.display):
                x=line.find(label)
                if x>=0:return x,y
            return None
        def wait_for(pred,timeout=3.0):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.05)
            return False
        def save_and_read():
            send(":w\r",.3)
            return file.read_text()
        try:
            drain(.4)
            # Row 0 is the breadcrumb bar; the text starts on row 1.
            assert wait_for(lambda: "f.rs" in screen.display[0]), screen.display[0]
            word=find("world")
            assert word, text()
            x,y=word[0]+1,word[1]
            # Double-click selects the word.
            for _ in range(2):
                send(sgr(0,x,y),.03);send(sgr(0,x,y,True),.03)
            drain(.2)
            assert "VISUAL" in text(), text()
            # Right-click it: a menu over the text, the selection kept.
            send(sgr(2,x,y),.05);send(sgr(2,x,y,True),.3)
            assert wait_for(lambda: find("Copy GitHub Permalink") and find("Select All")), text()
            assert "VISUAL" in text(), text()
            cut=find("Cut ") or find("Cut")
            send(sgr(0,cut[0],cut[1]),.05);send(sgr(0,cut[0],cut[1],True),.3)
            assert wait_for(lambda: not find("Select All")), "menu closed after a choice\n"+text()
            saved=save_and_read()
            assert saved.splitlines()[1]=="    let hello = ;", saved
            # Esc closes a menu; the key never reaches the buffer.
            send(sgr(2,6,2),.05);send(sgr(2,6,2,True),.3)
            assert wait_for(lambda: find("Select All")), text()
            send("\x1b",.3)
            assert wait_for(lambda: not find("Select All")), text()
            assert save_and_read()==saved
            # The file name's breadcrumb lists the top-level functions.
            name=find("f.rs")
            assert name and name[1]==0, text()
            send(sgr(0,name[0],0),.05);send(sgr(0,name[0],0,True),.3)
            assert wait_for(lambda: find("fn alpha") and find("fn beta") and find("fn beta")[1]>0), text()
            send("G",.1);send("\r",.3)   # last entry, Enter
            send("x",.2)
            assert save_and_read().splitlines()[3]=="n beta() {}", file.read_text()
            send(":qa!\r")
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
    print(f"Context menu PTY passed: {cols}x{rows}")
