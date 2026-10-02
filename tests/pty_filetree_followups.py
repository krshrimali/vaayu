#!/usr/bin/env python3
"""File tree follow-ups, driven through a real PTY: O cycles the sort
(header shows it, order changes), v opens a floating preview that follows
the cursor, gs/gu stage/unstage, U lists the trash and Enter restores an
item to its original directory, and expanded dirs + bookmarks survive a
restart (shada)."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, subprocess, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())

def git(root,*args):
    return subprocess.run(["git","-C",str(root),*args],capture_output=True,text=True)

class Session:
    def __init__(self,root,cfg,cols,rows,args):
        self.cols,self.rows=cols,rows
        self.pid,self.fd=pty.fork()
        if self.pid==0:
            os.chdir(root)
            os.environ.update(TERM="xterm-256color",XDG_CONFIG_HOME=str(cfg))
            os.execv(binary,[binary,*args])
        fcntl.ioctl(self.fd,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
        self.screen=pyte.Screen(cols,rows);self.stream=pyte.Stream(self.screen)
        self.decoder=codecs.getincrementaldecoder("utf-8")("replace")
        self.drain(.4)
    def drain(self,seconds=.2):
        end=time.monotonic()+seconds
        while time.monotonic()<end:
            ready,_,_=select.select([self.fd],[],[],min(.02,max(0,end-time.monotonic())))
            if ready:
                try:data=os.read(self.fd,65536)
                except OSError:break
                if not data:break
                self.stream.feed(self.decoder.decode(data))
    def key(self,s,seconds=.2):os.write(self.fd,s.encode());self.drain(seconds)
    def text(self):return "\n".join(self.screen.display)
    def tree(self):
        tw=min(32,max(self.cols//2,self.cols-20))
        return [row[:tw] for row in self.screen.display]
    def wait_for(self,pred,what,seconds=3):
        end=time.monotonic()+seconds
        while time.monotonic()<end:
            if pred():return
            self.drain(.05)
        raise AssertionError(what+"\n"+self.text())
    def quit(self):
        self.key(":qa!\r")
        end=time.monotonic()+3
        while time.monotonic()<end:
            done,status=os.waitpid(self.pid,os.WNOHANG)
            if done:
                assert os.waitstatus_to_exitcode(status)==0;self.pid=None;break
            self.drain(.05)
        assert self.pid is None,"Editor failed to exit"
    def close(self):
        if self.pid:
            os.kill(self.pid,signal.SIGKILL);os.waitpid(self.pid,0)
        os.close(self.fd)

def order(s,names):
    rows=s.tree()
    pos=[]
    for n in names:
        hit=[i for i,r in enumerate(rows) if n in r]
        assert hit,(n+" not in tree\n"+"\n".join(rows))
        pos.append(hit[0])
    return pos

for cols,rows in [(60,16),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix="vaayu-tree-followups-proj-") as proj, \
         tempfile.TemporaryDirectory(prefix="vaayu-tree-followups-cfg-") as cfg:
        root=pathlib.Path(proj);cfg=pathlib.Path(cfg)
        (cfg/"vaayu").mkdir(parents=True)
        (cfg/"vaayu/config.toml").write_text(
            'jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\ntree_icons=false\n')
        (root/"pkg").mkdir()
        (root/"pkg/inner.txt").write_text("inner\n")
        (root/"alpha.txt").write_text("a"*10+"\n")
        (root/"beta.md").write_text("PREVIEW-MARKER line one\nline two\n")
        (root/"gamma.rs").write_text("g"*500+"\n")
        has_git=git(root,"init","-q").returncode==0
        s=Session(root,cfg,cols,rows,[str(root/"alpha.txt")])
        try:
            s.key(",ft",.4)
            s.wait_for(lambda:any("gamma.rs" in r for r in s.tree()),"tree should list files")
            a,b,g=order(s,["alpha.txt","beta.md","gamma.rs"])
            assert a<b<g,"name order by default"
            # O: name -> type (md, rs, txt).
            s.key("O",.4)
            assert "Tree sorted by type" in s.text(),s.text()
            a,b,g=order(s,["alpha.txt","beta.md","gamma.rs"])
            assert b<g<a,("type order\n"+s.text())
            s.key("OO",.4)  # -> mtime -> size
            assert "Tree sorted by size" in s.text(),s.text()
            a,b,g=order(s,["alpha.txt","beta.md","gamma.rs"])
            assert g<b<a,("size order (largest first)\n"+s.text())
            p,=order(s,["pkg"])
            assert p<g,"directories stay first"
            s.key(":treesort name\r",.4)
            assert "Tree sorted by name" in s.text(),s.text()

            # v: floating preview follows the cursor.
            s.key("gg",.2)  # pkg
            s.key("v",.4)
            assert "inner.txt" in s.text()
            s.key("jj",.4)  # beta.md
            s.wait_for(lambda:"PREVIEW-MARKER" in s.text(),"preview should follow to beta.md")
            if cols>=100:
                line=[r for r in s.screen.display if "PREVIEW-MARKER" in r][0]
                assert line.index("PREVIEW-MARKER")>=32,("float sits beside the tree\n"+s.text())
            s.key("\x1b",.3)
            assert "PREVIEW-MARKER" not in s.text(),"Esc closes the preview"
            s.key("j",.3)
            assert "PREVIEW-MARKER" not in s.text() and "gggg" not in s.text(),"preview mode stays off"

            # gs / gu on beta.md.
            if has_git:
                s.key("k",.2)
                s.key("gs",.5)
                assert git(root,"diff","--cached","--name-only").stdout.strip()=="beta.md",s.text()
                s.key("gu",.5)
                assert git(root,"diff","--cached","--name-only").stdout.strip()=="",s.text()

            # Trash pkg/inner.txt, then restore it with U + Enter.
            s.key(":treefind\r",.2)
            s.key("gg",.2)
            s.key("l",.3)   # expand pkg
            s.key("j",.2)   # inner.txt
            s.key("tt",.4)
            assert not (root/"pkg/inner.txt").exists()
            s.key("U",.4)
            s.wait_for(lambda:"pkg/inner.txt" in s.text(),"trash list shows the origin")
            s.key("\r",.5)
            assert (root/"pkg/inner.txt").read_text()=="inner\n","Enter restores the file"
            assert "Restored pkg/inner.txt" in s.text(),s.text()

            # Bookmark pkg; it stays expanded; quit and relaunch.
            s.key(":tree\r",.3)
            s.key(",ft",.4)
            s.key("gg",.2)
            s.key("m",.2)
            s.quit()
        finally:
            s.close()
        s=Session(root,cfg,cols,rows,[str(root/"alpha.txt")])
        try:
            s.key(",ft",.4)
            s.wait_for(lambda:any("inner.txt" in r for r in s.tree()),"pkg stays expanded after a restart")
            assert any("pkg" in r and "★" in r for r in s.tree()),("bookmark survives\n"+s.text())
            s.quit()
        finally:
            s.close()
    print(f"File tree follow-ups PTY passed: {cols}x{rows}")
