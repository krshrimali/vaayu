#!/usr/bin/env python3
"""Exercise task tracker keymaps, all-date listing, and timestamped adds in a PTY."""
import datetime, os, pathlib, sys, tempfile, time
import pexpect

binary = str(pathlib.Path(sys.argv[1]).resolve())
today = datetime.date.today()
dates = [today - datetime.timedelta(days=30), today, today + datetime.timedelta(days=4)]
markers = ["OLDER_TASK_MARKER", "TODAY_TASK_MARKER", "FUTURE_TASK_MARKER"]
with tempfile.TemporaryDirectory(prefix="vaayu-tasktracker-") as tmp:
    base = pathlib.Path(tmp)
    root = base / "project"
    data = base / "data"
    config = base / "config" / "vaayu"
    root.mkdir(); config.mkdir(parents=True)
    (config / "config.toml").write_text("jk_escape=false\nclipboard_unnamedplus=false\nnumber=false\n")
    (root / "sample.txt").write_text("task tracker PTY\n")
    store = data / "vaayu" / "tasks"
    store.mkdir(parents=True)
    for date, marker in zip(dates, markers):
        (store / f"{date}.toml").write_text(
            f'''version = 1\ndate = "{date}"\n\n[[entries]]\nid = 1\ncreated_at = "2026-10-05T10:00:00Z"\nkind = "task"\nstatus = "open"\ntext = "{marker}"\n'''
        )
    env = os.environ | {"TERM": "xterm-256color", "XDG_CONFIG_HOME": str(base / "config"), "XDG_DATA_HOME": str(data)}
    child = pexpect.spawn(binary, [str(root / "sample.txt")], cwd=str(root), env=env,
                          dimensions=(30, 100), encoding="utf-8", timeout=5)
    try:
        time.sleep(.3)
        child.send(",")
        time.sleep(.12)
        child.send("t")
        time.sleep(.12)
        child.send("L")  # all saved task dates, including the future
        for marker in markers:
            child.expect_exact(marker)

        child.send("\x1b")
        time.sleep(.2)
        child.send(",")
        time.sleep(.12)
        child.send("t")
        time.sleep(.12)
        child.send("d")  # today's editable task document
        child.expect_exact("TODAY_TASK_MARKER")
        assert "FUTURE_TASK_MARKER" not in child.before

        child.send("\x1b")
        time.sleep(.2)
        child.send(",")
        time.sleep(.12)
        child.send("t")
        time.sleep(.12)
        child.send("l")  # opens an empty Insert-mode task draft
        child.expect_exact("Task draft")
        child.send("TASK_FROM_DRAFT")
        child.send("\x1b")
        time.sleep(.2)
        child.send(":wq\r")
        child.expect_exact("Task saved")
        assert "TASK_FROM_DRAFT" in child.before, "after saving, show the day's editable task view"
        today_path = store / f"{today}.toml"
        assert "TASK_FROM_DRAFT" in today_path.read_text()

        child.send(",")
        time.sleep(.12)
        child.send("t")
        time.sleep(.12)
        child.send("D")  # mark the task block at the cursor as done
        child.expect_exact("Task marked done")
        assert 'status = "done"' in today_path.read_text()

        future_date = today + datetime.timedelta(days=7)
        child.send(f":taskadd {future_date.isoformat()} 14:30 INLINE_FUTURE_TASK\r")
        child.expect_exact("Added task")
        future_path = store / f"{future_date}.toml"
        assert future_path.exists(), "future task should be persisted under its scheduled date"
        assert "INLINE_FUTURE_TASK" in future_path.read_text()

        child.send(",")
        time.sleep(.12)
        child.send("t")
        time.sleep(.12)
        child.send("N")  # activity-note prompt
        child.expect_exact("tasknote ")
        child.send(f"{today.isoformat()} ACTIVITY_NOTE_VIA_KEYMAP\r")
        child.expect_exact("Added note")
        assert "ACTIVITY_NOTE_VIA_KEYMAP" in today_path.read_text()

        child.send(",")
        time.sleep(.12)
        child.send("t")
        time.sleep(.12)
        child.send("O")  # date prompt
        child.expect_exact("on ")
        child.send(f"{dates[2].isoformat()}\r")
        child.expect_exact("FUTURE_TASK_MARKER")

        child.send("\x1b")
        time.sleep(.2)
        child.send(",")
        time.sleep(.12)
        child.send("t")
        time.sleep(.12)
        child.send("y")  # yesterday's task log
        yesterday = today - datetime.timedelta(days=1)
        child.expect_exact(f"Tasks for {yesterday}")

        child.send(",")
        time.sleep(.12)
        child.send("t")
        time.sleep(.12)
        child.send("w")  # Monday–Sunday view
        child.expect_exact("Tasks —")
        child.expect_exact("ACTIVITY_NOTE_VIA_KEYMAP")

        child.send(":tasklist\r")
        child.expect_exact("INLINE_FUTURE_TASK")
        child.send(":qa!\r")
        child.expect(pexpect.EOF)
    finally:
        if child.isalive():
            child.terminate(force=True)
print("Task tracker PTY passed: task view/list/add/note, date navigation, yesterday and week keymaps")
