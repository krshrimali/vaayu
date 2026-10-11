#!/usr/bin/env python3
"""Matching motions and block replacement through a PTY, using Vim reference data.

Set VAAYU_PENDING_FULL=1 to run every directly seedable reference case.
"""
import codecs, fcntl, json, os, pathlib, pty, re, select, signal, struct, sys, tempfile, termios, time
import pyte

binary = str(pathlib.Path(sys.argv[1]).resolve())
fixtures = json.loads((pathlib.Path(__file__).parent / 'fixtures/vim_pending.json').read_text())
# Registers with artificial width metadata are covered by Rust. The PTY
# creates registers through real yanks; width-one blocks need no metadata shim.
block = [c for c in fixtures['block'] if c['regtype'] in ['v', 'V', '\x161']]
cases = block + fixtures['percent'] if os.environ.get('VAAYU_PENDING_FULL') else block[::4] + fixtures['percent'][::3]
# This Vim build replays only deletion for Visual put. Vaayu replays the
# replacement; these explicit cases check that intended behavior in the UI.
for command, expected, cursor in [
    ('l\x16jl"aPj0.', 'aQRd\nQRRh\nQRkl\nlast\n', [2, 2]),
    ('l\x16jl"apj0.', 'aQRd\nQRRh\nQRkl\nlast\n', [2, 2]),
    ('l\x16jlPj0.', 'aQRd\nQRRh\nQRkl\nlast\n', [2, 2]),
    ('l\x16jlpj0.', 'aQRd\nbcRh\nfgkl\nlast\n', [2, 1]),
    ('l\x16jlPj02.', 'aQRd\nQRQRRh\nQRQRkl\nlast\n', [2, 4]),
]:
    cases.append(dict(initial='abcd\nefgh\nijkl\nlast\n', register='QR', regtype='v',
        command=command, expected=dict(text=expected, cursor=cursor), undo_steps=2))
for cols, rows in [(40, 12), (100, 24), (180, 50)]:
    with tempfile.TemporaryDirectory(prefix='vaayu-pending-vim-') as tmp:
        root = pathlib.Path(tmp)
        cfg = root / 'config/vaayu'; cfg.mkdir(parents=True)
        (cfg / 'config.toml').write_text('jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n'
            'swap_0_and_caret=false\nautopairs=false\nsmartindent=false\nwatch=false\n')
        target = root / 'sample.txt'; target.write_text('fixture\n')
        source = root / 'register.txt'
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(root)
            os.environ.update(TERM='xterm-256color', XDG_CONFIG_HOME=str(root / 'config'), XDG_DATA_HOME=str(root / 'data'))
            os.execv(binary, [binary, str(target)])
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', rows, cols, 0, 0))
        screen = pyte.Screen(cols, rows); stream = pyte.Stream(screen)
        decoder = codecs.getincrementaldecoder('utf-8')('replace')
        def drain(seconds=.06):
            end = time.monotonic() + seconds
            while time.monotonic() < end:
                ready, _, _ = select.select([fd], [], [], min(.02, max(0, end - time.monotonic())))
                if ready:
                    try: data = os.read(fd, 65536)
                    except OSError: break
                    if not data: break
                    stream.feed(decoder.decode(data))
        def key(value, seconds=.06):
            os.write(fd, value.encode()); drain(seconds)
        try:
            drain(.3)
            for index, case in enumerate(cases):
                if 'register' in case:
                    source.write_text(case['register'])
                    key(f':e! {source}\r')
                    kind = case['regtype']
                    key('gg0' + ('VG' if kind == 'V' else '\x16G' if kind.startswith('\x16') else 'vG$') + '"ay')
                target.write_text(case['initial'])
                key(f':e! {target}\r')
                command = 'gg0' + case['command']
                for part_index, part in enumerate(command.split('\x1b')):
                    if part: key(part)
                    if part_index < command.count('\x1b'): key('\x1b')
                ruler = re.search(r'(\d+):(\d+)\s*$', screen.display[rows - 2])
                assert ruler and [int(ruler[1]), int(ruler[2])] == case['expected']['cursor'], (index, command, screen.display)
                key(':w\r')
                assert target.read_text() == case['expected']['text'], (index, command, repr(target.read_text()), case['expected'])
                if case['expected']['text'] != case['initial']:
                    key('u' * case.get('undo_steps', 1)); key(':w\r')
                    assert target.read_text() == case['initial'], ('undo', index, command, repr(target.read_text()))
                    key('\x12' * case.get('undo_steps', 1)); key(':w\r')
                    assert target.read_text() == case['expected']['text'], ('redo', index, command)
            key(':qa!\r')
            end = time.monotonic() + 3
            while time.monotonic() < end:
                done, status = os.waitpid(pid, os.WNOHANG)
                if done:
                    assert os.waitstatus_to_exitcode(status) == 0
                    pid = None; break
                drain(.02)
            assert pid is None, 'Editor failed to exit'
        finally:
            if pid:
                try: os.kill(pid, signal.SIGKILL)
                except ProcessLookupError: pass
                os.waitpid(pid, 0)
            os.close(fd)
    print(f'Pending Vim PTY passed: {cols}x{rows}, {len(cases)} cases')
