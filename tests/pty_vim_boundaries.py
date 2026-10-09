#!/usr/bin/env python3
"""Interactive Vim-verified editing boundaries, cursors and undo at three terminal sizes."""
import codecs, fcntl, os, pathlib, pty, re, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
blank='a\n\n  b c\nD e f\n'
words='one two three\nfour five\nsix seven\n'
block='aaa bbb ccc ddd\nfirst second third\n  x y z\n'
cases=[
    (blank,'wsQ\x1b','a\nQ\n  b c\nD e f\n',(2,1)),
    (blank,'$dw','\n\n  b c\nD e f\n',(1,1)),
    (blank,'jdw','a\n  b c\nD e f\n',(2,3)),
    (blank,'w2dw','a\nc\nD e f\n',(2,1)),
    (blank,'2de','D e f\n',(1,1)),
    (words,'jcbQ\x1b','one two Q\nfour five\nsix seven\n',(1,9)),
    (words,'j$vlcQ\x1b','one two three\nfour fivQsix seven\n',(2,9)),
    (words,'$\x16jld','one two t\nfour five\nsix seven\n',(1,9)),
    (block,'$\x16jld','aaa bbb ccc dd\nfirst second t\n  x y z\n',(1,14)),
    (block,'j$\x16jlcQ\x1b','aaa bbb ccc ddd\nfirst sQ\n  x y zQ\n',(2,8)),
    (block,'$y2wp','aaa bbb ccc dddd\nfirst \nfirst second third\n  x y z\n',(1,16)),
    ('only one line\n','dd"1p','\nonly one line\n',(2,1)),
    ('only one line\n','ddP','only one line\n\n',(1,1)),
    ('only one line\n','3J','only one line\n',(1,1)),
    ('only one line\n','2dd','only one line\n',(1,1)),
    (words,'wdk',words,(1,5)),
    (blank,'jd$',blank,(2,1)),
    (blank,'jciwQ\x1b','a\nQ\n  b c\nD e f\n',(2,1)),
    (blank,'jcawQ\x1b','a\nQ c\nD e f\n',(2,1)),
    (words,'$cwQ\x1b','one two threQ\nfour five\nsix seven\n',(1,13)),
    ('only one line\n','$vlcQ\x1b','only one linQ\n',(1,13)),
    ('only one line\n','$vlyp','only one linee\n',(1,14)),
    (blank,'3J','a b c\nD e f\n',(1,2)),
    ('\talpha beta\n  c d\n\tlast end\n','j2x','\talpha beta\n  c\n\tlast end\n',(2,3)),
    ('\talpha beta\n  c d\n\tlast end\n','\x16jlyp','\t\taalpha beta\n  c   c dd\n\tlast end\n',(1,2)),
    (words,'c0Q\x1b','Qone two three\nfour five\nsix seven\n',(1,1)),
    ('\talpha beta\n  c d\n\tlast end\n','jdaw','\talpha beta\n  c\n\tlast end\n',(2,3)),
    ('Q\nZ\na\n界abc\n','\x16jy2jp','Q\nZ\naQ\n Z界abc\n',(3,2)),
    ('\tabc\n\tdef\n','l\x16jlcQ\x1b','\tQc\n\tQf\n',(1,2)),
    ('abc\n','xdk.','c\n',(1,1)),
]
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix='vaayu-vim-boundaries-') as tmp:
        root=pathlib.Path(tmp);cfg=root/'config/vaayu';cfg.mkdir(parents=True)
        (cfg/'config.toml').write_text('jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            'swap_0_and_caret=false\nautopairs=false\nsmartindent=false\nwatch=false\n')
        target=root/'sample.txt';target.write_text(words)
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root);os.environ.update(TERM='xterm-256color',XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'))
            os.execv(binary,[binary,str(target)])
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder('utf-8')('replace')
        def drain(seconds=.08):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                ready,_,_=select.select([fd],[],[],min(.02,max(0,end-time.monotonic())))
                if ready:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(value,seconds=.08):os.write(fd,value.encode());drain(seconds)
        try:
            drain(.3)
            for initial,command,expected,cursor in cases:
                target.write_text(initial);key(':e!\r');key('gg0'+command)
                ruler=re.search(r'(\d+):(\d+)\s*$',screen.display[rows-2])
                assert ruler and (int(ruler[1]),int(ruler[2]))==cursor, (command,screen.display)
                key(':w\r');assert target.read_text()==expected, (command,repr(target.read_text()),repr(expected))
                key('uu' if command.startswith(('dd','x')) else 'u');key(':w\r')
                assert target.read_text()==initial, ('undo',command,repr(target.read_text()))
            key(':qa!\r')
        finally:
            try:os.kill(pid,signal.SIGKILL)
            except ProcessLookupError:pass
            os.waitpid(pid,0);os.close(fd)
    print(f'Vim editing boundaries passed: {cols}x{rows}')
