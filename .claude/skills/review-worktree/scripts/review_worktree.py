#!/usr/bin/env python3
"""
This file handles the bookkeeping of the review-worktree skill: it creates the git worktree an agent writes code in,
and keeps the record that says what the worktree was made for. vaayu's `:worktrees` reads that record.

    <main>/code-reviews/
        fix-parser/          <- `new`: git worktree add -b fix-parser ... (the whole repository)
        fix-parser.json      <- `new` writes it, `update` keeps it current, `list` / `show` read it

Example:

    review_worktree.py new --slug fix-parser --title "Fix the parser crash on empty input" \\
        --description "Empty input made parse() index past the end. ..." --request "fix the crash in parse"

prints the worktree path (`<main>/code-reviews/fix-parser`). After the code is written:

    review_worktree.py update --slug fix-parser --status ready-for-review --focus src/parser.rs \\
        --test-command "cargo test parser"

Only the Python standard library is used, and git always runs without a shell.
"""
import argparse
import datetime
import json
import os
import re
import subprocess
import sys
from pathlib import Path

RECORDS_DIR = "code-reviews"
EXCLUDE_LINE = "/code-reviews/"
# A slug becomes a folder and a branch name, so it is kept to a safe, short alphabet (no `/`, no `..`).
SLUG_PATTERN = re.compile(r"^[a-z0-9][a-z0-9._-]{0,47}$")
STATUSES = ("in-progress", "ready-for-review", "revising", "merged", "abandoned")


def _git(*args, cwd):
    """Helper function to run git without a shell and return its stripped stdout."""
    result = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        raise SystemExit(f"git {' '.join(args)} failed: {result.stderr.strip() or result.stdout.strip()}")
    return result.stdout.strip()


# Needed so every command finds the same `code-reviews/` folder, whether it runs from the main checkout, a
# subdirectory, or inside one of the review worktrees. Re-used by every sub-command.
def main_root(start):
    # Main logic: the common git dir is `<main>/.git` for every worktree of a non-bare repository.
    common = Path(_git("rev-parse", "--path-format=absolute", "--git-common-dir", cwd=start))
    if common.name != ".git":
        raise SystemExit(
            f"Expected a non-bare repository (common git dir `{common}`). Please run this inside a normal checkout."
        )
    return common.parent


def _now():
    return datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def _record_path(root, slug):
    return root / RECORDS_DIR / f"{slug}.json"


def _write_record(path, record):
    """Helper function to write the record atomically, so vaayu never reads half a file."""
    tmp = path.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(record, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    os.replace(tmp, path)


def _read_record(path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        raise SystemExit(f"No record at {path}. Please create the worktree with `new` first.") from None


# Needed to make `code-reviews/` invisible to the main checkout (git status, rg, vaayu's file picker) without a
# tracked .gitignore change. Re-used only by `new`, which is idempotent about it.
def ensure_excluded(root):
    exclude = Path(_git("rev-parse", "--path-format=absolute", "--git-common-dir", cwd=root)) / "info" / "exclude"
    exclude.parent.mkdir(parents=True, exist_ok=True)
    lines = exclude.read_text(encoding="utf-8").splitlines() if exclude.exists() else []
    if EXCLUDE_LINE not in lines:
        with exclude.open("a", encoding="utf-8") as f:
            if lines and lines[-1] != "":
                f.write("\n")
            f.write(EXCLUDE_LINE + "\n")


def _branch_exists(root, name):
    result = subprocess.run(
        ["git", "show-ref", "--verify", "--quiet", f"refs/heads/{name}"], cwd=root, check=False
    )
    return result.returncode == 0


def cmd_new(args):
    # Validation check: the slug names a folder and a branch.
    if not SLUG_PATTERN.match(args.slug):
        raise SystemExit(
            f"Expected a slug of lowercase letters, digits, `.`, `_` or `-` (at most 48), but got {args.slug!r}."
            " Please pick a short kebab-case name such as `fix-parser-crash`."
        )
    root = main_root(Path.cwd())
    slug = args.slug
    # In case the slug (folder, record or branch) is taken, add `-2`, `-3`, ... rather than reuse someone's worktree.
    n = 1
    while (
        (root / RECORDS_DIR / slug).exists() or _record_path(root, slug).exists() or _branch_exists(root, slug)
    ):
        n += 1
        slug = f"{args.slug}-{n}"
    base = _git("rev-parse", "--verify", f"{args.base_ref}^{{commit}}", cwd=root)
    base_ref = args.base_ref
    if base_ref == "HEAD":
        # Record the branch name the user is on, which reads better than `HEAD` in vaayu.
        branch_name = _git("rev-parse", "--abbrev-ref", "HEAD", cwd=root)
        base_ref = branch_name if branch_name != "HEAD" else base[:10]
    (root / RECORDS_DIR).mkdir(exist_ok=True)
    ensure_excluded(root)
    worktree = root / RECORDS_DIR / slug
    _git("worktree", "add", "-q", "-b", slug, str(worktree), base, cwd=root)
    now = _now()
    record = {
        "version": 1,
        "slug": slug,
        "title": args.title,
        "description": args.description,
        "request": args.request,
        "worktree": f"{RECORDS_DIR}/{slug}",
        "branch": slug,
        "base": base,
        "base_ref": base_ref,
        "status": "in-progress",
        "created_at": now,
        "updated_at": now,
        "agent": args.agent,
        "notes": "",
        "test_command": args.test_command,
        "focus": args.focus,
        "revisions": [],
    }
    _write_record(_record_path(root, slug), record)
    print(worktree)


def _slug_from_cwd(root):
    """Helper function to infer the slug when the command runs inside a review worktree."""
    here = Path.cwd().resolve()
    reviews = (root / RECORDS_DIR).resolve()
    try:
        return here.relative_to(reviews).parts[0]
    except (ValueError, IndexError):
        raise SystemExit("Please pass --slug (not running inside code-reviews/<slug>/).") from None


def cmd_update(args):
    root = main_root(Path.cwd())
    slug = args.slug or _slug_from_cwd(root)
    path = _record_path(root, slug)
    record = _read_record(path)
    for field in ("title", "description", "notes", "test_command", "status"):
        value = getattr(args, field)
        if value is not None:
            record[field] = value
    if args.focus:
        record["focus"] = args.focus
    if args.revision:
        record.setdefault("revisions", []).append({"at": _now(), "summary": args.revision})
    record["updated_at"] = _now()
    _write_record(path, record)
    print(path)


def cmd_show(args):
    root = main_root(Path.cwd())
    slug = args.slug or _slug_from_cwd(root)
    record = _read_record(_record_path(root, slug))
    record["worktree_path"] = str(root / record.get("worktree", f"{RECORDS_DIR}/{slug}"))
    print(json.dumps(record, indent=2, ensure_ascii=False))


def cmd_list(_args):
    root = main_root(Path.cwd())
    folder = root / RECORDS_DIR
    for path in sorted(folder.glob("*.json")) if folder.is_dir() else []:
        try:
            record = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError) as error:
            print(f"{path.name}: unreadable ({error})")
            continue
        exists = (root / record.get("worktree", "")).is_dir()
        print(
            f"{record.get('slug', path.stem):<28} {record.get('status', ''):<17}"
            f" {record.get('title', '')}{'' if exists else '  (worktree missing)'}"
        )


def build_parser():
    parser = argparse.ArgumentParser(description="Worktrees for code an agent writes, with a record vaayu reads.")
    sub = parser.add_subparsers(dest="command", required=True)

    new = sub.add_parser("new", help="create code-reviews/<slug>/ (a git worktree) and its record; prints its path")
    new.add_argument("--slug", required=True)
    new.add_argument("--title", required=True, help="one line: what the change does")
    new.add_argument("--description", required=True, help="what it is for and what changes, a few sentences")
    new.add_argument("--request", default="", help="the user's ask, close to verbatim")
    new.add_argument("--base-ref", default="HEAD", help="commit or branch to start from (default: HEAD)")
    new.add_argument("--agent", default="claude")
    new.add_argument("--test-command", default="")
    new.add_argument("--focus", nargs="*", default=[], help="files to review first, relative to the repository")
    new.set_defaults(func=cmd_new)

    update = sub.add_parser("update", help="change fields of a record; always bumps updated_at")
    update.add_argument("--slug")
    update.add_argument("--title")
    update.add_argument("--description")
    update.add_argument("--notes")
    update.add_argument("--test-command")
    update.add_argument("--status", choices=STATUSES)
    update.add_argument("--focus", nargs="*")
    update.add_argument("--revision", help="append a revision note, e.g. 'Addressed #1 and #2; kept #3 (why)'")
    update.set_defaults(func=cmd_update)

    show = sub.add_parser("show", help="print a record, with the worktree's absolute path")
    show.add_argument("--slug")
    show.set_defaults(func=cmd_show)

    listing = sub.add_parser("list", help="list every record of this repository")
    listing.set_defaults(func=cmd_list)
    return parser


if __name__ == "__main__":
    parsed = build_parser().parse_args()
    parsed.func(parsed)
    sys.exit(0)
