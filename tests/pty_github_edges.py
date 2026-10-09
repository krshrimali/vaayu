#!/usr/bin/env python3
"""Exercise invalid input, errors, and delayed GitHub completions through PTYs."""
import ast, codecs, fcntl, json, os, pathlib, pty, select, shutil, signal, struct, subprocess, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
artifact=os.environ.get('VAAYU_AUDIT_ROOT')
original=ast.parse((pathlib.Path(__file__).parent/'pty_github.py').read_text())
fake_source=next(ast.literal_eval(n.value) for n in original.body if isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='FAKE_GH' for t in n.targets))
gate=r'''
import time
if os.path.exists('fault') and a[:2] == ['pr','view'] and a[2] == '--json':
    sys.stderr.write(open('fault').read())
    sys.exit(1)
if ((os.path.exists('gate-checkout') and a[:2] == ['pr','checkout']) or
    (os.path.exists('gate-list') and a[:2] == ['pr','list'])):
    open('started','w').write(' '.join(a))
    deadline=time.monotonic()+8
    while not os.path.exists('release') and time.monotonic()<deadline:
        time.sleep(.01)
'''
fake_source=fake_source.replace('def out(v):',gate+'\ndef out(v):')
fake_source=fake_source.replace('print(json.dumps(v))', "print(json.dumps(v), flush=True)\n    open('finished','w').write('done')")
evidence=[]
def git(root,*args):
    return subprocess.run(['git','-C',str(root),*args],check=True,capture_output=True,text=True).stdout.strip()

for scenario in ['invalid-arguments','implicit-http-error','edits-during-checkout',
                 'superseded-checkout','command-during-request','visual-during-request']:
    with tempfile.TemporaryDirectory(prefix='vaayu-gh-edge-') as tmp:
        base=pathlib.Path(tmp); root=base/'project'; root.mkdir()
        cfg=base/'config/vaayu'; cfg.mkdir(parents=True)
        (cfg/'config.toml').write_text('jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\nwatch=false\nautoread=false\nprogress=false\n')
        bindir=base/'bin'; bindir.mkdir()
        for name in ['git','rg','sh','python3']:
            found=shutil.which(name)
            if found: (bindir/name).symlink_to(found)
        (bindir/'gh').write_text(fake_source); (bindir/'gh').chmod(0o755)
        git(root,'init','-q','-b','main');git(root,'config','user.email','audit@example.com');git(root,'config','user.name','Audit')
        (root/'.gitignore').write_text('gh.log\nfault\ngate-*\nstarted\nrelease\n')
        (root/'a.py').write_text('one\ntwo\nthree\n')
        git(root,'add','.');git(root,'commit','-qm','initial');git(root,'checkout','-qb','pr-5')
        (root/'a.py').write_text('one\nTWO\nchanged on pr branch\n')
        git(root,'commit','-qam','PR');git(root,'checkout','-q','main')
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(root)
            os.environ.update(TERM='xterm-256color',XDG_CONFIG_HOME=str(base/'config'),
                              XDG_DATA_HOME=str(base/'data'),PATH=str(bindir))
            os.execv(binary,[binary,str(root/'a.py')])
        cols,rows=120,28
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',rows,cols,0,0))
        screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder('utf-8')('replace')
        def drain(seconds=.15):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                ready,_,_=select.select([fd],[],[],min(.02,max(0,end-time.monotonic())))
                if ready:
                    try:data=os.read(fd,65536)
                    except OSError:break
                    if not data:break
                    stream.feed(decoder.decode(data))
        def key(value,seconds=.15):os.write(fd,value.encode());drain(seconds)
        def text():return '\n'.join(screen.display)
        def wait_for(pred,timeout=3):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if pred():return True
                drain(.03)
            return False
        def note(label,bug,expected,extra=None):
            evidence.append({'scenario':label,'bug':bug,'expected':expected,'actual_screen':text(),
                             'details':extra})
        try:
            drain(.3)
            if scenario=='invalid-arguments':
                for cmd in ['ghpr garbage','ghdiff garbage','ghthreads garbage','ghchecks garbage']:
                    key(':'+cmd+'\r')
                    assert wait_for(lambda:'Usage:' in text()), text()
                    assert not (root/'gh.log').exists(), 'Invalid arguments must not invoke gh'
                    note(cmd,'Usage:' not in text(),'Reject invalid PR number with a usage message',
                         'No gh command started')
                    key('\x1b')
            elif scenario=='implicit-http-error':
                (root/'fault').write_text('HTTP 502: upstream temporarily unavailable\n')
                key(':ghpr\r')
                assert wait_for(lambda:'HTTP 502' in text()), text()
                assert 'no pull request for the current branch' not in text()
                note(scenario,False,'Preserve HTTP 502 rather than claim that no PR exists')
            elif scenario in ['edits-during-checkout','superseded-checkout']:
                (root/'gate-checkout').touch()
                key(':ghcheckout 5\r')
                assert wait_for(lambda:(root/'started').exists())
                if scenario=='edits-during-checkout':
                    key('iUNSAVED_MARKER\x1b')
                    assert 'UNSAVED_MARKER' in text()
                else:
                    key(':ghprs\r')
                    assert wait_for(lambda:'checkout in progress' in text()), text()
                    assert not (root/'gh.log').read_text().count('pr list'), 'Checkout must retain its receiver'
                    key('\x1b')
                (root/'release').touch()
                assert wait_for(lambda:git(root,'branch','--show-current')=='pr-5')
                drain(.6)
                if scenario=='edits-during-checkout':
                    lost='UNSAVED_MARKER' not in text()
                    assert not lost, text()
                    assert 'kept unsaved edits' in text(), text()
                    note(scenario,lost,'Preserve edits made while network checkout is pending')
                    key('u')
                    assert 'UNSAVED_MARKER' not in text(), 'Undo must undo the insert'
                    key('\x12')
                    assert 'UNSAVED_MARKER' in text(), 'Redo must restore the preserved edit'
                    note('undo-after-checkout',False,
                         'Unsaved text remains recoverable through undo')
                else:
                    assert 'changed on pr branch' in text(), text()
                    note(scenario,False,
                         'Always reload buffers when a background checkout completes',
                         {'branch':git(root,'branch','--show-current'),'disk':(root/'a.py').read_text()})
            else:
                (root/'gate-list').touch()
                key(':ghprs\r')
                assert wait_for(lambda:(root/'started').exists())
                key(':set number' if scenario=='command-during-request' else 'vll')
                before=text()
                (root/'release').touch()
                assert wait_for(lambda:(root/'finished').exists()), 'GitHub fixture did not finish'
                drain(.25)  # Command mode shows the typed command instead of the message line.
                assert '#5  Fix parser' not in text(), text()
                assert (':set number' in text() and 'COMMAND' in text()) if scenario=='command-during-request' else 'VISUAL' in text(), text()
                note(scenario,False,'Defer results while command input or visual selection is active',{'before':before})
                key('\x11')
                assert '#5  Fix parser' in text(), 'Ctrl-Q should open deferred results'
            key(':qa!\r',.05)
        finally:
            try:os.kill(pid,signal.SIGKILL)
            except ProcessLookupError:pass
            os.waitpid(pid,0);os.close(fd)
if artifact:
    pathlib.Path(artifact,'github-edge-fixed.json').write_text(json.dumps(evidence,indent=2)+'\n')
print(json.dumps({'observations':len(evidence),'bugs':sum(e['bug'] for e in evidence)}))
