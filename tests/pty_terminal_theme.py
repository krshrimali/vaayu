#!/usr/bin/env python3
"""Validate focused and unfocused floating-terminal borders in every palette."""
import codecs, fcntl, json, os, pathlib, pty, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
artifact=os.environ.get('VAAYU_AUDIT_ROOT')
palettes={
    'default':('00ffff','7f7f7f','262626'),
    'mono':('eeeeee','585858','303030'),
    'warm':('c85a5a','826e5a','3c2d23'),
    'cool':('5aa0d2','5a6e82','232d3c'),
    'gruvbox':('8ec07c','928374','3c3836'),
    'gruvbox-light':('427b58','928374','ebdbb2'),
    'flexoki':('3aa99f','878580','1c1b1a'),
    'flexoki-light':('24837b','6f6e69','f2f0e5'),
    'tokyonight':('7dcfff','565f89','292e42'),
    'tokyonight-day':('007197','848cb5','c4c8da'),
}
observations=[];failures=[]
with tempfile.TemporaryDirectory(prefix='vaayu-terminal-theme-') as tmp:
    root=pathlib.Path(tmp);cfg=root/'config/vaayu';cfg.mkdir(parents=True)
    (cfg/'config.toml').write_text('jk_escape=false\nnumber=false\nprogress=false\n')
    (root/'sample.txt').write_text('editor buffer\n')
    pid,fd=pty.fork()
    if pid==0:
        os.chdir(root)
        os.environ.pop('NO_COLOR',None)
        os.environ.update(TERM='xterm-256color',XDG_CONFIG_HOME=str(root/'config'),
                          XDG_DATA_HOME=str(root/'data'),SHELL='/bin/sh')
        os.execv(binary,[binary,str(root/'sample.txt')])
    cols,rows=100,30
    fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',rows,cols,0,0))
    screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder('utf-8')('replace')
    def drain(seconds=.18):
        end=time.monotonic()+seconds
        while time.monotonic()<end:
            ready,_,_=select.select([fd],[],[],min(.02,max(0,end-time.monotonic())))
            if ready:
                try:data=os.read(fd,65536)
                except OSError:break
                if not data:break
                stream.feed(decoder.decode(data))
    def key(value,seconds=.18):os.write(fd,value.encode());drain(seconds)
    def check(name,focused,fg,bg):
        corners=[(x,y) for y,row in enumerate(screen.display) for x,ch in enumerate(row) if ch=='╭']
        assert len(corners)==1, (name,'terminal frame missing',screen.display)
        x,y=corners[0]
        right=screen.display[y].rfind('╮')
        bottom=next(row for row in range(y+1,rows) if screen.buffer[row][x].data=='╰')
        cells=[screen.buffer[y][x],screen.buffer[y][x+1],screen.buffer[y+1][x],
               screen.buffer[y][right],screen.buffer[bottom][x],screen.buffer[bottom][right],
               screen.buffer[bottom][x+1],screen.buffer[y+1][right]]
        actual=[(c.data,c.fg,c.bg) for c in cells]
        ok=all(c.fg==fg and c.bg==bg for c in cells)
        observations.append({'theme':name,'focused':focused,'expected':[fg,bg],
                             'actual':actual,'passes':ok})
        if not ok:failures.append((name,focused,actual,fg,bg))
        else:
            assert all(c.fg==fg for c in cells)
            assert all(c.bg==bg for c in cells)
    try:
        drain(.3)
        for name,(accent,muted,bg) in palettes.items():
            key(':colorscheme '+name+'\r')
            key('\x1c',.3)
            check(name,True,accent,bg)
            key('\x17')
            check(name,False,muted,bg)
            key('\x1c')
        key(':qa!\r',.05)
    finally:
        try:os.kill(pid,signal.SIGKILL)
        except ProcessLookupError:pass
        os.waitpid(pid,0);os.close(fd)
suffix='baseline' if 'baseline' in binary else 'fixed'
if artifact:
    pathlib.Path(artifact,'terminal-theme-'+suffix+'.json').write_text(json.dumps(observations,indent=2)+'\n')
print(json.dumps({'observations':len(observations),'failures':failures}))
assert not failures, failures
