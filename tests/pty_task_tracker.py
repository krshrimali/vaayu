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
    tours = root / ".tours"
    tours.mkdir()
    (tours / "intro.tour").write_text(
        '{"title":"Keymap tour","steps":['
        '{"file":"sample.txt","line":1,"description":"FIRST_KEYMAP_STEP"},'
        '{"file":"sample.txt","line":1,"description":"SECOND_KEYMAP_STEP"}]}'
    )
    store = data / "vaayu" / "tasks"
    store.mkdir(parents=True)
    for date, marker in zip(dates, markers):
        priority = {"OLDER_TASK_MARKER": 3, "TODAY_TASK_MARKER": 1, "FUTURE_TASK_MARKER": 5}[marker]
        (store / f"{date}.toml").write_text(
            f'''version = 1\ndate = "{date}"\n\n[[entries]]\nid = 1\ncreated_at = "2026-10-05T10:00:00Z"\nkind = "task"\nstatus = "open"\npriority = {priority}\ntext = "{marker}"\n'''
        )
        if marker == "OLDER_TASK_MARKER":
            with (store / f"{date}.toml").open("a") as f:
                f.write(
                    '\n[[entries]]\nid = 2\ncreated_at = "2026-10-05T09:00:00Z"\n'
                    'kind = "task"\nstatus = "done"\ntext = "OLD_DONE_TASK"\n'
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
        old_path = store / f"{dates[0]}.toml"
        today_path = store / f"{today}.toml"
        assert 'text = "OLDER_TASK_MARKER"' not in old_path.read_text(), "overdue open task should move out of its old day"
        assert 'text = "OLDER_TASK_MARKER"' in today_path.read_text(), "overdue task should appear in today's document"
        assert today_path.read_text().count('text = "OLDER_TASK_MARKER"') == 1, "rollover must not duplicate tasks"
        assert 'text = "OLD_DONE_TASK"' in old_path.read_text(), "completed tasks must remain on their original date"

        child.send("\x1b")
        time.sleep(.2)
        child.send(",")
        time.sleep(.12)
        child.send("t")
        time.sleep(.12)
        child.send("d")  # today's editable task document
        child.expect_exact("TODAY_TASK_MARKER")
        assert "FUTURE_TASK_MARKER" not in child.before
        assert "OLDER_TASK_MARKER" in child.before

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
        time.sleep(.15)
        child.send("ggf1r7jj0oFIRST_NOTE_LINE")
        child.send("\r")
        child.send("SECOND_NOTE_LINE")
        child.send("\x1b")
        time.sleep(.15)
        child.send(":wq\r")
        child.expect_exact("Task saved")
        assert "TASK_FROM_DRAFT" in child.before, "after saving, show the day's editable task view"
        assert "TASK_FROM_DRAFT" in today_path.read_text()
        assert "priority = 7" in today_path.read_text(), today_path.read_text()
        assert 'notes = """\nFIRST_NOTE_LINE\nSECOND_NOTE_LINE' in today_path.read_text(), today_path.read_text()

        child.send(",")
        time.sleep(.12)
        child.send("t")
        time.sleep(.12)
        child.send("D")  # mark the task block at the cursor as done
        child.expect_exact("Task marked done")
        assert 'status = "done"' in today_path.read_text()

        future_date = today + datetime.timedelta(days=7)
        child.send(f":taskadd {future_date.isoformat()} 14:30 --priority=9 INLINE_FUTURE_TASK\r")
        child.expect_exact("Added task")
        future_path = store / f"{future_date}.toml"
        assert future_path.exists(), "future task should be persisted under its scheduled date"
        assert "INLINE_FUTURE_TASK" in future_path.read_text()
        assert "priority = 9" in future_path.read_text()

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
        for marker in ["TASK_FROM_DRAFT", "FUTURE_TASK_MARKER", "OLDER_TASK_MARKER", "TODAY_TASK_MARKER"]:
            child.expect_exact(marker)
        child.send("\x11")  # send the task list to quickfix
        child.send("D")
        child.expect_exact("Task marked done")
        assert 'status = "done"' in future_path.read_text(), "D should complete the selected task from the list view"

        # Tabs and tours have their own leader prefixes, separate from tasks.
        child.send("\x1b")
        time.sleep(.15)
        child.send(",")
        time.sleep(.1)
        child.send("un")  # create a new tab
        child.send(":tabs\r")
        child.expect_exact("2 (current)")
        child.send("\x1b")
        time.sleep(.15)
        child.send(",")
        time.sleep(.1)
        child.send("u[")  # switch to the previous tab
        time.sleep(.2)
        child.send(",")
        time.sleep(.1)
        child.send("uq")  # close the active tab
        child.send(":tabs\r")
        child.expect_exact("1 (current)")
        child.send("\x1b")
        time.sleep(.15)
        child.send(",")
        time.sleep(.1)
        child.send("vs")
        child.expect_exact("Tours — Enter starts one")
        child.send("\r")
        child.expect_exact("FIRST_KEYMAP_STEP")
        child.send(",")
        time.sleep(.1)
        child.send("vn")
        child.expect_exact("SECOND_KEYMAP_STEP")
        child.send(",")
        time.sleep(.1)
        child.send("vp")
        child.expect_exact("FIRST_KEYMAP_STEP")
        child.send(",")
        time.sleep(.1)
        child.send("ve")
        child.expect_exact("Tour ended")
        child.send(":qa!\r")
        child.expect(pexpect.EOF)
    finally:
        if child.isalive():
            child.terminate(force=True)
print("Task tracker PTY passed: task view/list/add/note, date navigation, yesterday and week keymaps")
