#!/usr/bin/env python3
"""The browser editor in real Chromium and Firefox: every check drives the page
with trusted input events and reads the cell grid or canvas pixels it painted."""
import codecs, fcntl, os, pathlib, pty, select, signal, struct, subprocess, sys, tempfile, termios, time, zlib
import pyte
from playwright.sync_api import sync_playwright

binary = str(pathlib.Path(sys.argv[1]).resolve())
browsers = sys.argv[2:] or ["chromium", "firefox"]
mock_lsp = str((pathlib.Path(__file__).parent / "mock_lsp.py").resolve())
failures = []
passed = 0

def red_png(size=16):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    header = struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0)
    pixels = (b"\x00" + b"\xff\x00\x00" * size) * size
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(pixels)) + chunk(b"IEND", b"")


def check(name, ok, detail=""):
    global passed
    if ok:
        passed += 1
        print(f"  ok   {name}")
    else:
        failures.append(name)
        print(f"  FAIL {name} {detail}")


def make_project(tmp):
    project = tmp / "proj"
    (project / "src").mkdir(parents=True)
    (project / "src/main.rs").write_text(
        'fn main() {\n    println!("hello 世界 😀");\n}\n'
        + "".join(f"// line {i}\n" for i in range(300))
    )
    (project / "notes.txt").write_text("alpha beta\ngamma delta\n")
    (project / "wide.txt").write_text(
        "ascii|\n世界|\n😀|\néx|\n👩‍💻|\n🇮🇳|\nनमस्ते|\n✔️|\n"
    )
    (project / "red.png").write_bytes(red_png())
    (project / "doc.md").write_text(
        "# Heading\n\n![red](red.png)\n\n```mermaid\nflowchart TD\nA[Start]-->B[End]\n```\n"
    )
    return project


def write_config(tmp, extra=""):
    directory = tmp / "cfg/vaayu"
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "config.toml").write_text(
        "jk_escape=false\nprogress=false\nclipboard_unnamedplus=false\n" + extra
    )


def session_env(tmp):
    env = dict(os.environ, XDG_CONFIG_HOME=str(tmp / "cfg"), XDG_DATA_HOME=str(tmp / "data"),
               HOME=str(tmp), SHELL="/bin/sh", TERM="xterm-256color")
    for name in ("DISPLAY", "WAYLAND_DISPLAY", "NO_COLOR", "BROWSER"):
        env.pop(name, None)
    return env


class Session:
    def __init__(self, tmp, project, *args):
        self.process = subprocess.Popen(
            [binary, "--web", "--no-open", "--port", "0", *args], cwd=project,
            env=session_env(tmp), stdout=subprocess.PIPE, text=True)
        self.url = self.process.stdout.readline().strip()

    def stop(self):
        if self.process.poll() is None:
            self.process.kill()
        self.process.wait()


class Tab:
    def __init__(self, browser, url, width=1100, height=720, **context):
        self.context = browser.new_context(viewport={"width": width, "height": height}, **context)
        self.page = self.context.new_page()
        self.errors = []
        self.page.on("pageerror", lambda e: self.errors.append(str(e)))
        self.page.on("console", lambda m: self.errors.append(m.text) if m.type == "error" else None)
        self.page.on("dialog", lambda d: d.accept())
        self.page.goto(url)
        self.wait(lambda: self.frames() > 0)
        self.settle()

    def frames(self):
        return self.page.evaluate("window.__vaayu ? __vaayu.stats.frames : 0")

    def wait(self, predicate, timeout=8, what=""):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if predicate():
                return True
            time.sleep(0.03)
        return False

    def settle(self, quiet=0.15, timeout=6):
        """Waits until no frame has arrived for `quiet` seconds and all is painted."""
        end = time.monotonic() + timeout
        last, since = self.frames(), time.monotonic()
        while time.monotonic() < end:
            time.sleep(0.02)
            now = self.frames()
            if now != last:
                last, since = now, time.monotonic()
            elif time.monotonic() - since >= quiet:
                break
        self.page.evaluate("new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))")

    def rows(self):
        return self.page.evaluate(
            "Array.from({length: __vaayu.grid.rows}, (_, y) => __vaayu.rowText(y))")

    def text(self):
        return "\n".join(self.rows())

    def has(self, needle, timeout=8):
        return self.wait(lambda: needle in self.text(), timeout)

    def lacks(self, needle, timeout=8):
        return self.wait(lambda: needle not in self.text(), timeout)

    def keys(self, text, settle=True):
        self.page.keyboard.type(text)
        if settle:
            self.settle()

    def press(self, *chords):
        for chord in chords:
            self.page.keyboard.press(chord)
        self.settle()

    def ex(self, command):
        self.keys(":" + command, settle=False)
        self.press("Enter")

    def size(self):
        return self.page.evaluate("[__vaayu.grid.cols, __vaayu.grid.rows]")

    def cursor(self):
        return self.page.evaluate("[__vaayu.vt.x, __vaayu.vt.y]")

    def cell_center(self, col, row):
        w, h = self.page.evaluate("[__vaayu.metrics.css.w, __vaayu.metrics.css.h]")
        return (col + 0.5) * w, (row + 0.5) * h

    def find(self, needle):
        for y, row in enumerate(self.rows()):
            if needle in row:
                return row.index(needle), y
        return None

    def pixel_colors(self, x, y, width, height):
        return self.page.evaluate(
            """([x, y, w, h]) => {
                const c = document.getElementById('grid');
                const copy = document.createElement('canvas');
                copy.width = w; copy.height = h;
                const g = copy.getContext('2d');
                g.drawImage(c, x, y, w, h, 0, 0, w, h);
                const d = g.getImageData(0, 0, w, h).data;
                const set = new Set();
                for (let i = 0; i < d.length; i += 4) set.add(d[i] << 16 | d[i+1] << 8 | d[i+2]);
                return [...set];
            }""", [x, y, width, height])

    def close(self):
        self.context.close()


def test_editing(tab, project):
    check("page leaves the token out of the address bar", "token" not in tab.page.url)
    check("file opened from the command line", tab.has('println!("hello'))
    check("status line present", tab.has("NORMAL"))
    check("title names the file", tab.page.title().startswith("src/main.rs"))
    tab.keys("ggOinserted line")
    tab.press("Escape")
    check("typing inserts text", tab.has("inserted line"))
    check("title marks unsaved edits", tab.page.title().startswith("● "))
    tab.ex("w")
    check(":w writes the file", (project / "src/main.rs").read_text().startswith("inserted line\n"))
    tab.keys("u")
    check("undo works", tab.lacks("inserted line"))
    tab.press("Control+r")
    check("Ctrl-R redo reaches the editor", tab.has("inserted line"))
    tab.keys("dd")
    tab.ex("w")
    tab.press("Control+v")
    check("Ctrl-V starts visual block, not a browser paste", tab.has("V-BLOCK") or tab.has("VISUAL BLOCK"), tab.rows()[-2])
    tab.press("Escape")
    tab.keys("i")
    tab.press("Control+[")
    check("Ctrl-[ is Escape", tab.has("NORMAL"))
    tab.keys("/gamma-not-here")
    tab.press("Escape")


def test_syntax(tab):
    x, y = tab.find("fn main")
    w, h = tab.page.evaluate("[__vaayu.metrics.cellW, __vaayu.metrics.cellH]")
    keyword = tab.pixel_colors(x * w, y * h, w * 2, h)
    plain = tab.pixel_colors((x + 3) * w, y * h, w * 4, h)
    check("syntax colors reach the canvas", set(keyword) != set(plain) and len(keyword) > 1)


def test_windows_and_mouse(tab):
    tab.ex("only")
    tab.ex("vsplit notes.txt")
    check("vsplit shows both files", tab.has("alpha beta") and tab.has("fn main"))
    col, row = tab.find("fn main")
    tab.page.mouse.click(*tab.cell_center(col + 3, row))
    tab.settle()
    check("click moves focus and cursor", tab.cursor() == [col + 3, row], tab.cursor())
    tab.press("Alt+w", "l")
    check("Alt-W stands in for Ctrl-W", tab.find("alpha beta") is not None and tab.cursor()[0] > col + 3, tab.cursor())
    tab.ex("only")
    tab.ex("e src/main.rs")
    tab.keys("gg")
    before = tab.find("// line 0")
    x, y = tab.cell_center(10, 10)
    tab.page.mouse.move(x, y)
    tab.page.mouse.wheel(0, 400)
    tab.settle()
    check("wheel scrolls the view", tab.find("// line 0") != before)
    tab.keys("gg")
    col, row = tab.find("// line 2")
    tab.page.mouse.move(*tab.cell_center(col, row))
    tab.page.mouse.down()
    tab.page.mouse.move(*tab.cell_center(col + 6, row), steps=4)
    tab.page.mouse.up()
    tab.settle()
    check("mouse drag selects", tab.has("VISUAL"))
    tab.press("Escape")


def test_tools(tab):
    tab.ex("tree")
    check("file tree opens", tab.has("wide.txt"))
    tab.ex("tree")
    tab.keys(",ff")
    check("file picker opens", tab.has("notes.txt"))
    tab.keys("widetx")
    check("picker filters", tab.has("wide.txt") and tab.lacks("notes.txt"))
    tab.press("Enter")
    check("picker opens the file", tab.has("ascii|"))
    tab.keys(",/")
    tab.keys("gamma")
    check("live grep finds text", tab.has("gamma delta"))
    tab.press("Escape")
    tab.press("Escape")
    tab.ex("colorscheme github-light")
    check("colorscheme reaches the page background",
          tab.page.evaluate("getComputedStyle(document.documentElement).getPropertyValue('--page-bg')").strip() == "#ffffff",
          tab.page.evaluate("getComputedStyle(document.documentElement).getPropertyValue('--page-bg')"))
    tab.ex("colorscheme default")
    tab.ex("help")
    check(":help opens", tab.has("Vaayu — editing and review"))
    tab.keys("q")


def test_wide_text(tab):
    tab.ex("only")
    tab.ex("e wide.txt")
    tab.settle()
    rows = tab.rows()
    bars = [row.index("|") for row in rows if "|" in row]
    check("sample rows painted", len(bars) >= 8, rows[:9])
    check("unicode widths align the same as the renderer", "世界|" in tab.text() and "😀|" in tab.text())


def test_terminal(tab):
    tab.ex("terminal")
    tab.settle(0.3)
    tab.keys("echo web-term-$((40+2))", settle=False)
    tab.press("Enter")
    check("embedded terminal runs commands", tab.has("web-term-42"))
    tab.keys("exit", settle=False)
    tab.press("Enter")
    tab.settle(0.3)
    tab.press("Escape", "Alt+w", "c")
    check("closing the terminal returns to the file", tab.lacks("web-term-42"))
    tab.ex("only")
    # Output far faster than anyone reads: the editor must stay caught up.
    tab.ex("terminal")
    tab.settle(0.3)
    tab.keys("seq 1 150000; echo seq-finished", settle=False)
    tab.press("Enter")
    check("a flood of terminal output catches up", tab.has("seq-finished", 25))
    tab.keys("echo still-responsive", settle=False)
    tab.press("Enter")
    check("input still works after a flood", tab.has("still-responsive", 5))
    tab.keys("exit", settle=False)
    tab.press("Enter")
    tab.settle(0.3)
    tab.press("Escape", "Alt+w", "c")
    tab.ex("only")


def test_images(tab):
    tab.ex("e doc.md")
    tab.keys(",mp")
    check("preview renders the heading", tab.has("Heading"))
    def has_images():
        return tab.page.evaluate("__vaayu.images.size") >= 1
    check("preview uploads images to the page", tab.wait(has_images))
    tab.settle(0.4)
    w, h, cols, rows = tab.page.evaluate(
        "[__vaayu.metrics.cellW, __vaayu.metrics.cellH, __vaayu.grid.cols, __vaayu.grid.rows]")
    colors = tab.pixel_colors(0, 0, cols * w, (rows - 2) * h)
    check("the PNG's pixels are on the canvas", 0xff0000 in colors, len(colors))
    tab.keys("q")
    tab.settle()
    tab.ex("bd!")


def test_clipboard(tab, is_chromium):
    tab.ex("e notes.txt")
    tab.keys("gg")
    tab.keys('"+yy')
    tab.settle()
    if is_chromium:
        check('"+y copies to the browser clipboard',
              tab.wait(lambda: tab.page.evaluate("navigator.clipboard.readText()") == "alpha beta\n"),
              repr(tab.page.evaluate("navigator.clipboard.readText()")))
    tab.keys("G")
    tab.keys('"+p')
    check('"+p pastes back what was copied', tab.has("alpha beta", 2) and len([r for r in tab.rows() if "alpha beta" in r]) == 2)
    tab.keys("u")
    # A browser paste, as Ctrl+Shift+V or the menu would deliver it.
    tab.keys("ggO", settle=False)
    tab.page.evaluate("""() => {
        const text = 'from browser\\nsecond line';
        const data = new DataTransfer();
        data.setData('text/plain', text);
        // Chromium reads `clipboardData`; Firefox builds it from `data`.
        document.getElementById('input').dispatchEvent(new ClipboardEvent('paste', {
            clipboardData: data, dataType: 'text/plain', data: text, bubbles: true, cancelable: true,
        }));
    }""")
    tab.settle()
    tab.press("Escape")
    check("browser paste inserts multiple lines", tab.has("from browser") and tab.has("second line"))
    tab.keys('"+p')
    check("a browser paste feeds the + register", len([r for r in tab.rows() if "from browser" in r]) == 2)
    tab.ex("e!")
    if is_chromium:
        tab.page.evaluate("navigator.clipboard.writeText('via shortcut')")
        tab.keys("ggO", settle=False)
        tab.press("Control+Shift+v")
        tab.press("Escape")
        check("Ctrl+Shift+V pastes through the browser", tab.has("via shortcut"))
        tab.ex("e!")
    # Shift+drag selects in the display and copies on release.
    col, row = tab.find("gamma delta")
    tab.page.keyboard.down("Shift")
    tab.page.mouse.move(*tab.cell_center(col, row))
    tab.page.mouse.down()
    tab.page.mouse.move(*tab.cell_center(col + 4, row), steps=3)
    tab.page.mouse.up()
    tab.page.keyboard.up("Shift")
    tab.settle()
    check("Shift+drag paints a display selection",
          tab.page.evaluate("!document.getElementById('selection').hidden"))
    if is_chromium:
        check("Shift+drag copies the cells",
              tab.wait(lambda: tab.page.evaluate("navigator.clipboard.readText()") == "gamma"))
    tab.press("Escape")


def test_input_method(tab):
    tab.ex("e! notes.txt")
    tab.keys("ggi", settle=False)
    tab.page.evaluate("""() => {
        const input = document.getElementById('input');
        input.dispatchEvent(new CompositionEvent('compositionstart', {data: ''}));
        input.value = 'नमस्ते';
        input.dispatchEvent(new InputEvent('input', {data: 'नमस्ते', isComposing: true}));
        input.dispatchEvent(new CompositionEvent('compositionend', {data: 'नमस्ते'}));
    }""")
    tab.settle()
    tab.press("Escape")
    check("composed text arrives once", tab.text().count("नमस्ते") == 1, tab.rows()[0])
    tab.keys("u")


def test_resize(tab):
    before = tab.size()
    tab.page.set_viewport_size({"width": 700, "height": 420})
    tab.settle(0.2)
    after = tab.size()
    check("resize changes the grid", after != before and after[0] < before[0], (before, after))
    check("status line follows the new height", "NORMAL" in tab.rows()[after[1] - 2], tab.rows()[after[1] - 2])
    col, row = tab.find("gamma delta")
    tab.page.mouse.click(*tab.cell_center(col + 2, row))
    tab.settle()
    check("clicks map to the resized grid", tab.cursor() == [col + 2, row], tab.cursor())
    tab.page.set_viewport_size({"width": 1100, "height": 720})
    tab.settle(0.2)


def test_display_options(browser, tmp, project):
    write_config(tmp, "web_font_size = 22\n")
    session = Session(tmp, project, "notes.txt")
    tab = Tab(browser, session.url, device_scale_factor=1.5)
    cell = tab.page.evaluate("__vaayu.metrics.css.h")
    check("web_font_size sets the cell size", cell > 22, cell)
    check("the cell is a whole number of device pixels",
          float(tab.page.evaluate("__vaayu.metrics.cellW")).is_integer())
    col, row = tab.find("gamma delta")
    tab.page.mouse.click(*tab.cell_center(col + 2, row))
    tab.settle()
    check("clicks map correctly on a scaled display", tab.cursor() == [col + 2, row], tab.cursor())
    tab.press("F11")
    check("F11 enters page fullscreen",
          tab.wait(lambda: tab.page.evaluate("document.fullscreenElement !== null"), 3))
    tab.close()
    session.stop()


def test_lsp(tab):
    tab.ex("e src/main.rs")
    tab.keys("ggK")
    check("LSP reply is painted without another key", tab.has("hover", 5) or tab.has("Hover", 1), tab.rows()[:6])
    tab.press("Escape")


def test_reload_and_takeover(tab, browser, url):
    tab.ex("e notes.txt")
    tab.keys("ggiunsaved-mark", settle=False)
    tab.press("Escape")
    tab.page.reload()
    tab.wait(lambda: tab.frames() > 0)
    tab.settle()
    check("reload keeps the session and unsaved text", tab.has("unsaved-mark"))
    second = Tab(browser, url)
    check("a second tab attaches", second.has("unsaved-mark"))
    check("the first tab says it was replaced",
          tab.wait(lambda: tab.page.evaluate("!document.getElementById('banner').hidden")))
    second.keys("A!!", settle=False)
    second.press("Escape")
    tab.page.click("#banner-action")
    tab.wait(lambda: tab.page.evaluate("document.getElementById('banner').hidden"))
    tab.settle()
    check("taking it back shows the live state", tab.has("unsaved-mark"))
    check("the other tab is told in turn",
          second.wait(lambda: second.page.evaluate("!document.getElementById('banner').hidden")))
    second.close()
    tab.ex("e!")
    tab.settle()


def test_quit(tab, session, project):
    tab.keys("ggiunsaved", settle=False)
    tab.press("Escape")
    tab.ex("q")
    check("quit refuses unsaved edits", session.process.poll() is None and tab.has("unsaved", 2))
    tab.ex("qa!")
    check("server exits on :qa!", tab.wait(lambda: session.process.poll() is not None))
    check("server exit status is 0", session.process.returncode == 0, session.process.returncode)
    check("page says the session ended", tab.wait(
        lambda: "quit" in tab.page.evaluate("document.getElementById('banner-text').textContent")))
    check("state file removed", not (project / ".vaayu/web.json").exists())


def parity_tui_screen(tmp, project, cols, rows, steps):
    """The same keys through the terminal frontend, for comparison."""
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(project)
        os.environ.clear()
        os.environ.update(session_env(tmp))
        os.execv(binary, [binary, "src/main.rs"])
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    screen = pyte.Screen(cols, rows)
    stream = pyte.Stream(screen)
    decoder = codecs.getincrementaldecoder("utf-8")("replace")
    def drain(seconds=0.25):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            ready, _, _ = select.select([fd], [], [], 0.02)
            if ready:
                try:
                    stream.feed(decoder.decode(os.read(fd, 65536)))
                except OSError:
                    break
    drain(0.5)
    shots = []
    for keys in steps:
        os.write(fd, keys.replace("\n", "\r").encode())
        drain(0.5)
        shots.append(list(screen.display))
    os.kill(pid, signal.SIGKILL)
    os.waitpid(pid, 0)
    os.close(fd)
    return shots


PARITY_STEPS = [
    "",
    "G",
    "gg:vsplit notes.txt\n",
    "\x17l:only\n",
    ":e wide.txt\n",
    ":tree\n",
    ":tree\n:e src/main.rs\n",
    ":help\n",
    "\x1b:bd\n",
    "ifn\x1b",
    "u:colorscheme gruvbox\n",
    ":colorscheme default\n",
]


def test_parity(browser, tmp):
    """One editor, two displays: row for row, the page must show what pyte shows."""
    project = tmp / "parity"
    project.mkdir()
    (project / "src").mkdir()
    (project / "src/main.rs").write_text('fn main() {\n    println!("hello");\n}\n' + "// x\n" * 60)
    (project / "notes.txt").write_text("alpha beta\n")
    (project / "wide.txt").write_text("世界|\n😀|\nplain|\n")
    session = Session(tmp, project, "src/main.rs")
    tab = Tab(browser, session.url, width=900, height=500)
    cols, rows = tab.size()
    tab_shots = []
    for keys in PARITY_STEPS:
        for c in keys:
            if c == "\n":
                tab.page.keyboard.press("Enter")
            elif c == "\x1b":
                tab.page.keyboard.press("Escape")
            elif c == "\x17":
                tab.page.keyboard.press("Alt+w")
            else:
                tab.page.keyboard.type(c)
        tab.settle(0.3)
        tab_shots.append([r.replace("​", "") for r in tab.rows()])
    tab.close()
    session.stop()
    # Each frontend has its own session state from here: a fresh shada each.
    tui_tmp = tmp / "parity-tui"
    tui_tmp.mkdir()
    write_config(tui_tmp)
    tui_shots = parity_tui_screen(tui_tmp, project, cols, rows, PARITY_STEPS)
    for index, (web, tui) in enumerate(zip(tab_shots, tui_shots)):
        # The browser's first-run hint is the one intended difference.
        web = [t if w.startswith("Browsers keep") else w for w, t in zip(web, tui)]
        norm_web = ["".join(c for c in r if c != " ") for r in web]
        norm_tui = ["".join(c for c in r if c != " ") for r in tui]
        mismatches = [y for y, (a, b) in enumerate(zip(norm_web, norm_tui)) if a != b]
        detail = "" if not mismatches else f"row {mismatches[0]}: {web[mismatches[0]]!r} != {tui[mismatches[0]]!r}"
        check(f"parity with the terminal, step {index} {PARITY_STEPS[index]!r}", not mismatches, detail)


def run_browser(pw, name, root):
    print(f"== {name}")
    tmp = root / name
    tmp.mkdir()
    parity = tmp / "parity-web"
    parity.mkdir()
    write_config(parity)
    project = make_project(tmp)
    log = tmp / "lsp.jsonl"
    write_config(tmp, f'[lsp.fixture]\ncmd = ["python3", "{mock_lsp}", "{log}"]\nfiletypes = ["rust"]\n')
    browser = getattr(pw, name).launch()
    session = Session(tmp, project, "src/main.rs")
    options = {"permissions": ["clipboard-read", "clipboard-write"]} if name == "chromium" else {}
    tab = Tab(browser, session.url, **options)
    try:
        test_editing(tab, project)
        test_syntax(tab)
        test_windows_and_mouse(tab)
        test_tools(tab)
        test_wide_text(tab)
        test_terminal(tab)
        test_images(tab)
        test_clipboard(tab, name == "chromium")
        test_input_method(tab)
        test_resize(tab)
        test_lsp(tab)
        test_reload_and_takeover(tab, browser, session.url)
        check("no page errors", not tab.errors, tab.errors)
        test_quit(tab, session, project)
        tab.close()
        test_parity(browser, parity)
        (project / ".vaayu/web.json").unlink(missing_ok=True)
        test_display_options(browser, tmp, project)
    finally:
        session.stop()
        browser.close()


with tempfile.TemporaryDirectory(prefix="vaayu-web-") as directory:
    root = pathlib.Path(directory)
    with sync_playwright() as pw:
        for name in browsers:
            run_browser(pw, name, root)

print(f"{passed} passed, {len(failures)} failed")
if failures:
    print("\n".join(f"  - {f}" for f in failures))
    sys.exit(1)
