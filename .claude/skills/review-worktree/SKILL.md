---
name: review-worktree
description: Use whenever you are about to write, edit, refactor or generate code or tests in a git repository for Kush (shrimali), including multi-file features and one-line fixes, and whenever he pastes a review that starts with "# Code review from vaayu". Kush reviews Claude-written code in the vaayu editor, never straight in his checkout.
---

# Review worktree

Kush reviews code you write in **vaayu**, not in the chat. So every change goes
into its own git worktree under `code-reviews/<slug>/` at the repository's main
root, with a record `code-reviews/<slug>.json` saying what it is for. In vaayu,
`:worktrees` lists the records, Enter opens a review screen of the diff (changed
files with `+adds −dels`, unchanged code dimmed), he comments, and `:reviewcopy`
gives him a prompt to paste back to you.

```
<main>/code-reviews/
    fix-parser/        git worktree, the whole repo, branch fix-parser
    fix-parser.json    the record vaayu shows: title, description, base, status
```

Helper (stdlib Python, no shell): `~/.claude/skills/review-worktree/scripts/review_worktree.py`
(`new`, `update`, `show`, `list`; `--help` on each). Call it `RW` below.

## When this applies

- **Writing code in a git repo**: follow steps 1 to 4.
- **Already inside `code-reviews/<slug>/`** (cwd or the task names it): keep
  working there. Never nest a worktree inside a worktree.
- **A pasted vaayu review**: step 5.
- **Not a git repo, read-only questions, or Kush says "edit in place" / "in
  this checkout"**: work normally.

## 1. Create the worktree before the first edit

```bash
python3 ~/.claude/skills/review-worktree/scripts/review_worktree.py new \
  --slug fix-parser-crash \
  --title "Fix the parser crash on empty input" \
  --description "Empty input made parse() index past the end of the token list. Return an empty AST instead, and test it." \
  --request "<his ask, close to verbatim>"
```

It prints the worktree path (`WT`). It branches from `HEAD` of the main
checkout, records that commit as `base`, and adds `/code-reviews/` to
`.git/info/exclude` (never to a tracked `.gitignore`). The slug is short
kebab-case; a taken one gets `-2`.

If `git status --porcelain` in the main checkout shows uncommitted changes to
files the task depends on, ask whether to carry them over
(`git -C <main> diff HEAD | git -C "$WT" apply`) before writing code.

## 2. Work only inside `WT`

- Every Read/Edit/Write path is under `WT`; every command runs with
  `cd "$WT"` or `git -C "$WT"`. The main checkout stays untouched.
- `WT` has no build output yet. Before a first build in a big repo (Rust
  `target/`, `node_modules`), check `df -h "$WT"`: a second build tree can use
  gigabytes.
- Do not commit unless Kush asks. vaayu diffs the working tree against
  `base`, so uncommitted work is reviewed the same way. Never push.

## 3. Keep the record true

Rewrite the description as the change takes shape: what it is for, what
changed, and how. `RW update` bumps `updated_at` every time.

## 4. Hand off

```bash
RW update --slug fix-parser-crash --status ready-for-review \
  --focus src/parser.rs tests/parse.rs \
  --test-command "cargo test parser" \
  --notes "Behavior change: empty input now returns an empty AST instead of panicking."
```

Then tell him, in this order: the worktree path and branch; a two-line
summary; the tests you ran and their result; and how to review:
`:worktrees` in vaayu, Enter on `<slug>` (or `:reviewstart` inside the
worktree), `,rc` to comment, `:reviewcopy` to bring the comments back.

## 5. A pasted review (`# Code review from vaayu: ...`)

1. Its `Repository:` line is the worktree. Work there (steps 2 and 3 apply).
2. Each `## #N path:line` section is one request: the quoted code is what he
   looked at, the text after it is what he wants. Read the file as it is now.
3. Address every open comment. Keep each change to what the comment asks.
4. `RW update --status ready-for-review --revision "Addressed #1, #2; kept #3: <why>"`.
5. Reply per number: `#1: done, <what changed, path:line>`, `#3: kept as is
   because <reason>`. Tell him the review screen picks the edits up on its
   own; resolving a comment stays his call (`R` in `:comments`).

## Finishing

Merge, delete or clean up only when he asks:
`git -C <main> merge --no-ff <branch>`, then
`git -C <main> worktree remove code-reviews/<slug>` and
`RW update --slug <slug> --status merged` (or `abandoned`).

## Record fields

`title` (one line), `description` (what and why), `request`, `slug`,
`worktree` (relative to main), `branch`, `base` (full sha) and `base_ref`,
`status` (`in-progress`, `ready-for-review`, `revising`, `merged`,
`abandoned`), `focus` (files to read first), `test_command`, `notes`,
`revisions` (`{at, summary}`), `created_at`/`updated_at` (UTC). vaayu shows
title, description, status, notes and the last revision; `base` decides the
diff, so never edit it by hand.

## Common mistakes

| Mistake | Fix |
| --- | --- |
| Edited files in the main checkout | Move the edits into `WT` (`git -C <main> diff \| git -C "$WT" apply`, then `git -C <main> checkout -- <files>` only with his OK). |
| `git worktree add` by hand, no record | `:worktrees` shows "(no description)". Run `RW new` or write the record. |
| Record left at `in-progress` | Nothing tells him it is ready. `RW update --status ready-for-review`. |
| New worktree for a revision | Revisions go into the same worktree, so his open comments still line up. |
| Committed or pushed to "finish" | Only when asked. The review reads the working tree. |
