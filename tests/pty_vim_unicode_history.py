#!/usr/bin/env python3
"""Unicode block edits, display columns, repeat and history at three terminal sizes."""
import codecs, fcntl, os, pathlib, pty, re, select, signal, struct, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
cases = [('\tabc\n\tdef\nlast end\n', '\x16jAQ\x1b', '\tQabc\n\tQdef\nlast end\n', (1, 1)),
 ('\tabc\n\tdef\nlast end\n', 'l\x16jAQ\x1b', '\taQbc\n\tdQef\nlast end\n', (1, 2)),
 ('\tabc\n\tdef\nlast end\n', 'l\x16jlAQ\x1b', '\tabQc\n\tdeQf\nlast end\n', (1, 2)),
 ('a界bc\n界abc\nxy\n', 'l\x16jlIQ\x1b', 'aQ界bc\n Q界abc\nxy\n', (1, 2)),
 ('a界bc\n界abc\nxy\n', 'l\x16jAQ\x1b', 'a界Qbc\n界aQbc\nxy\n', (1, 1)),
 ('a界bc\n界abc\nxy\n', 'l\x16jld', 'abc\n bc\nxy\n', (1, 2)),
 ('a界bc\n界abc\nxy\n', 'l\x16jlcQ\x1b', 'aQbc\n Qbc\nxy\n', (1, 2)),
 ('ab界c\nx界yz\n0123456\n', '\x16jld', ' c\nyz\n0123456\n', (1, 1)),
 ('ab界c\nx界yz\n0123456\n', '\x16jlyp', 'aab b界c\nxx界界yz\n0123456\n', (1, 2)),
 ('ab界c\nx界yz\n0123456\n', '2l\x16jlIQ\x1b', 'abQ界c\nx Q界yz\n0123456\n', (1, 3)),
 ('ab界c\nx界yz\n0123456\n', '2l\x16jAQ\x1b', 'ab界Qc\nx界yQz\n0123456\n', (1, 2)),
 ('ab界c\nx界yz\n0123456\n', '2l\x16jld', 'abc\nx z\n0123456\n', (1, 3)),
 ('ab界c\nx界yz\n0123456\n', '2l\x16jlcQ\x1b', 'abQc\nx Qz\n0123456\n', (1, 3)),
 ('\tabc\n\tdef\nlast end\n', 'x$u\x12', 'abc\n\tdef\nlast end\n', (1, 1)),
 ('\tabc\n\tdef\nlast end\n', 'x$uj', '\tabc\n\tdef\nlast end\n', (2, 1)),
 ('\tabc\n\tdef\nlast end\n', 'x$u\x12j', 'abc\n\tdef\nlast end\n', (2, 1)),
 ('\tabc\n\tdef\nlast end\n', 'vlcQ\x1bdk.', 'Q\tdef\nlast end\n', (1, 1)),
 ('\tabc\n\tdef\nlast end\n', 'lvlcQ\x1bdk.', '\tQ\n\tdef\nlast end\n', (1, 2)),
 ('\tabc\n\tdef\nlast end\n', '2lvlcQ\x1bdk.', '\taQ\tdef\nlast end\n', (1, 3)),
 ('\tabc\n\tdef\nlast end\n', 'j$vlcQ\x1b2dd.', '\tabc\n\tdeQast end\n', (2, 4)),
 ('a界bc\n界abc\nxy\n', 'j$\x16jIQ\x1b', 'a界bc\n界Qabc\nxyQ\n', (2, 2)),
 ('a界bc\n界abc\nxy\n', 'j$\x16jAQ\x1b', 'a界bc\n界abcQ\nxyQ\n', (2, 2)),
 ('a\tbcd\n12\tx\nend\n', 'j$\x16jIQ\x1b', 'a\tbcd\n12Q\tx\nendQ\n', (2, 3)),
 ('a\tbcd\n12\tx\nend\n', '$\x16jIQ\x1b', 'a\tbQcd\n12\txQ\nend\n', (1, 4)),
 ('éabc\nxxéyz\nlast\n', 'j$\x16jIQ\x1b', 'éabc\nxxéyQz\nlastQ\n', (2, 6)),
 ('éabc\nxxéyz\nlast\n', 'j$\x16jAQ\x1b', 'éabc\nxxéyzQ\nlastQ\n', (2, 6)),
 ('a界bc\n界abc\nxy\n', 'dwu\x12', '界bc\n界abc\nxy\n', (1, 1)),
 ('a界bc\n界abc\nxy\n', 'ldwu\x12', 'abc\n界abc\nxy\n', (1, 2)),
 ('éabc\nxxéyz\nlast\n', 'dwu\x12', '\nxxéyz\nlast\n', (1, 1)),
 ('éabc\nxxéyz\nlast\n', 'jdwu\x12', 'éabc\n\nlast\n', (2, 1)),
 ('\n  first word\n\nlast line\n', 'dwu\x12', '  first word\n\nlast line\n', (1, 1)),
 ('abcdef\n', 'x$u\x12', 'bcdef\n', (1, 1)),
 ('ab界c\nx界yz\n0123456\n', '$x$u\x12j', 'ab界\nx界yz\n0123456\n', (2, 2)),
 ('a\tbcd\n12\tx\nend\n', 'j$\x16jlcQ\x1b', 'a\tbcd\n12Q \nendQ\n', (2, 3)),
 ('a\tbcd\n12\tx\nend\n', '\x16jlAQ\x1b', 'a\tQbcd\n12\tQ\tx\nend\n', (1, 1)),
 ('\n  first word\n\nlast line\n', '$vlcQ\x1bdk.', 'Q\nlast line\n', (1, 1)),
 ('abc\nlonger\n', '$vcQ\x1bj0.', 'abQ\nQ\n', (2, 1)),
 ('\nabc\n', '\x16jA\x1b', ' \nabc\n', (1, 1)),
 ('abcdef\n', 'xxx2u', 'bcdef\n', (1, 1)),
 ('abcdef\n', 'xxx3u2\x12', 'cdef\n', (1, 1), 2),
 ('ab界c\nx界yz\n', '\x16jl~', 'AB界c\nX界yz\n', (1, 1)),
 ('a\tbcd\n12\tx\n', '\x16jlA\x1b', 'a\tbcd\n12\t\tx\n', (1, 1)),
 ('abcd\n\tx\n', '2l\x16jlI\x1b', 'abcd\n\tx\n', (1, 2))]
for cols,rows in [(40,12),(100,24),(180,50)]:
    with tempfile.TemporaryDirectory(prefix='vaayu-unicode-history-') as tmp:
        root=pathlib.Path(tmp);cfg=root/'config/vaayu';cfg.mkdir(parents=True)
        (cfg/'config.toml').write_text('jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            'swap_0_and_caret=false\nautopairs=false\nsmartindent=false\nwatch=false\n')
        target=root/'sample.txt';target.write_text(cases[0][0])
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
            for initial,command,expected,cursor,*undo_count in cases:
                target.write_text(initial);key(':e!\r');
                for part_index,part in enumerate(('gg0'+command).split('\x1b')):
                    if part: key(part)
                    if part_index<command.count('\x1b'): key('\x1b')
                ruler=re.search(r'(\d+):(\d+)\s*$',screen.display[rows-2])
                assert ruler and (int(ruler[1]),int(ruler[2]))==cursor, (command,screen.display)
                if command=='\x16jA\x1b':
                    assert '[+]' in screen.display[rows-2], 'Block A padding must remain undoable, as in Vim'
                key(':w\r');assert target.read_text()==expected, (command,repr(target.read_text()),repr(expected))

                steps=undo_count[0] if undo_count else (2 if command.endswith('.') else 1)
                key('u'*steps);key(':w\r')
                assert target.read_text()==initial, ('undo',command,repr(target.read_text()))
            key(':qa!\r')
            end=time.monotonic()+3
            while time.monotonic()<end:
                done,status=os.waitpid(pid,os.WNOHANG)
                if done:
                    assert os.waitstatus_to_exitcode(status)==0
                    pid=None;break
                drain(.02)
            assert pid is None, 'Editor failed to exit'
        finally:
            if pid:
                try:os.kill(pid,signal.SIGKILL)
                except ProcessLookupError:pass
                os.waitpid(pid,0)
            os.close(fd)
    print(f'Unicode and history boundaries passed: {cols}x{rows}')
