#!/usr/bin/env python3
"""Key-to-screen latency of `vaayu --web` in a real browser, beside the same
scripted keys in the terminal frontend.

For the browser: `frame` is key event to the frame applied to the grid and
`paint` to the animation frame that draws it, which includes waiting for the
next vsync. For the terminal: key written to the first output byte,
the same proxy bench/latency.py uses. Neither includes the display's scanout.

Usage: bench/web_latency.py target/release/vaayu --out bench/web-latency.json
"""
import argparse, json, os, pathlib, pty, select, shutil, statistics, struct, subprocess, tempfile, time, fcntl, termios
from playwright.sync_api import sync_playwright

parser = argparse.ArgumentParser()
parser.add_argument("binary")
parser.add_argument("--browsers", default="chromium,firefox")
parser.add_argument("--out")
parser.add_argument("--width", type=int, default=1600)
parser.add_argument("--height", type=int, default=1000)
args = parser.parse_args()
binary = str(pathlib.Path(args.binary).resolve())
SOURCE = pathlib.Path(__file__).resolve().parent.parent / "src/render.rs"

SCRIPT = [
    ({"key": "j"}, 200), ({"key": "k"}, 200), ({"key": "w"}, 100), ({"key": "b"}, 100),
    ({"key": "f", "ctrl": True}, 30), ({"key": "b", "ctrl": True}, 30),
]
TERMINAL_KEYS = {"f+ctrl": "\x06", "b+ctrl": "\x02"}
TYPING = "the quick brown fox "

MEASURE = """async ([keys, n]) => {
  const input = document.getElementById('input');
  const frame = [], paint = [];
  const yieldOnce = () => new Promise((resolve) => {
    const channel = new MessageChannel();
    channel.port1.onmessage = resolve;
    channel.port2.postMessage(0);
  });
  const summary = (s) => {
    s.sort((a, b) => a - b);
    return { n: s.length, p50: s[s.length >> 1], p95: s[Math.floor(s.length * 0.95)], max: s[s.length - 1] };
  };
  for (let i = 0; i < n; i++) {
    const before = __vaayu.stats.frames, started = performance.now();
    const k = keys[i % keys.length];
    input.dispatchEvent(new KeyboardEvent('keydown', { key: k.key, ctrlKey: !!k.ctrl, bubbles: true, cancelable: true }));
    while (__vaayu.stats.frames === before && performance.now() - started < 500) await yieldOnce();
    frame.push(performance.now() - started);
    await new Promise((resolve) => requestAnimationFrame(resolve));
    paint.push(performance.now() - started);
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  return { frame: summary(frame), paint: summary(paint) };
}"""

def summary(samples):
    samples = sorted(samples)
    pick = lambda q: samples[min(len(samples) - 1, int(len(samples) * q))]
    return {"n": len(samples), "p50": round(pick(0.5), 3), "p95": round(pick(0.95), 3), "max": round(samples[-1], 3)}


def prepare(tmp):
    project = tmp / "proj"
    project.mkdir()
    shutil.copy(SOURCE, project / "render.rs")
    (tmp / "cfg/vaayu").mkdir(parents=True)
    (tmp / "cfg/vaayu/config.toml").write_text("jk_escape=false\nprogress=false\nclipboard_unnamedplus=false\n")
    return project, dict(os.environ, XDG_CONFIG_HOME=str(tmp / "cfg"), XDG_DATA_HOME=str(tmp / "data"),
                         HOME=str(tmp), SHELL="/bin/sh", TERM="xterm-256color")


def web_run(pw, name):
    with tempfile.TemporaryDirectory(prefix="vaayu-web-bench-") as directory:
        tmp = pathlib.Path(directory)
        project, env = prepare(tmp)
        process = subprocess.Popen([binary, "--web", "--no-open", "--port", "0", "render.rs"], cwd=project, env=env,
                                   stdout=subprocess.PIPE, text=True)
        url = process.stdout.readline().strip()
        browser = getattr(pw, name).launch()
        page = browser.new_page(viewport={"width": args.width, "height": args.height})
        page.goto(url)
        time.sleep(1.5)
        cols, rows = page.evaluate("[__vaayu.grid.cols, __vaayu.grid.rows]")
        result = {"grid": [cols, rows]}

        # Timed inside the page: polling from automation would add its own
        # scheduling delay (13 ms in Chromium) to everything it measures.
        groups = {}
        for key, count in SCRIPT:
            groups[key["key"] + ("+ctrl" if key.get("ctrl") else "")] = page.evaluate(MEASURE, [[key], count])
        page.keyboard.press("i")
        typing = [{"key": " " if c == " " else c} for c in TYPING]
        groups["typing"] = page.evaluate(MEASURE, [typing, 200])
        page.keyboard.press("Escape")
        result["operations"] = groups
        # A full repaint of the whole grid, however it was caused.
        cost = page.evaluate("""async () => {
            const samples = [];
            for (let i = 0; i < 60; i++) {
                __vaayu.grid.dirty.fill(1);
                const before = __vaayu.stats.paintMs;
                window.dispatchEvent(new Event('vaayu-repaint'));
                await new Promise(r => requestAnimationFrame(r));
                await new Promise(r => requestAnimationFrame(r));
                samples.push(__vaayu.stats.paintMs - before);
            }
            return samples;
        }""")
        result["full_repaint_ms"] = summary(cost)
        result["bytes_per_frame"] = round(page.evaluate("__vaayu.stats.bytes / __vaayu.stats.frames"))
        browser.close()
        process.kill()
        process.wait()
        return result


def terminal_run(cols, rows):
    with tempfile.TemporaryDirectory(prefix="vaayu-term-bench-") as directory:
        tmp = pathlib.Path(directory)
        project, env = prepare(tmp)
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(project)
            os.environ.clear()
            os.environ.update(env)
            os.execv(binary, [binary, "render.rs"])
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

        def drain(seconds):
            end = time.monotonic() + seconds
            while time.monotonic() < end:
                ready, _, _ = select.select([fd], [], [], 0.002)
                if ready:
                    os.read(fd, 1 << 20)

        def press(key):
            os.write(fd, key.encode())
            started = time.perf_counter()
            ready, _, _ = select.select([fd], [], [], 1)
            elapsed = (time.perf_counter() - started) * 1000
            if ready:
                os.read(fd, 1 << 20)
            drain(0.004)
            return elapsed if ready else None

        drain(1)
        groups = {}
        for key, count in SCRIPT:
            label = key["key"] + ("+ctrl" if key.get("ctrl") else "")
            samples = [s for s in (press(TERMINAL_KEYS.get(label, label)) for _ in range(count)) if s]
            groups[label] = summary(samples)
        os.write(fd, b"i")
        drain(0.05)
        groups["typing"] = summary([s for s in (press(c) for c in TYPING * 10) if s])
        os.kill(pid, 9)
        os.waitpid(pid, 0)
        os.close(fd)
        return groups


results = {"binary": binary, "browsers": {}}
with sync_playwright() as pw:
    for name in args.browsers.split(","):
        results["browsers"][name] = web_run(pw, name)
cols, rows = next(iter(results["browsers"].values()))["grid"]
results["terminal"] = terminal_run(cols, rows)
print(json.dumps(results, indent=1))
if args.out:
    pathlib.Path(args.out).write_text(json.dumps(results, indent=1) + "\n")
