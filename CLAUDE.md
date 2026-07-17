# Working in this repo

`docs/SPEC.md` is the plan and the source of truth. Read it before changing behaviour, and
record what you did in it after — Part III stays the plan, Part VII says how far it got (P4).
Every rule in Part II has a matching entry in Part V explaining the bug it exists to prevent;
**do not change a Part II rule without reading its V entry first.**

## Comments (P6)

**A comment states a constraint the code can't show. Nothing else.**

- Not *what* the line does — the line does that.
- Not *where it came from* — git does that.
- Not *that it's good*, or which spec paragraph blessed it — that's narration, and V.42
  bans it. A comment that cites `V.n` to explain a design is usually narration; a comment
  that says "this must run before the snapshot, or the rollback has nothing to revert to"
  is a constraint. Prefer the second.

If you can express the constraint in a name or a type instead of a comment, do that.

## NO LEGACY

This is a rewrite, not a migration. There are **no compatibility shims, no old-format
readers, no dual code paths kept "just in case".** When a thing is replaced, the old thing is
deleted in the same change — including the config keys, the docs, and the tests that named
it. A green test suite over the old model is not progress; it means the old model still runs.

## Changes

- One parser for `backend:name` (the grammar in `config/grammar/`). Anything that splits on
  `:` and trusts the prefix is a bug.
- Every path that removes calls the guard (`app/sync/guard.rs`); every install/change path
  calls the `[guard]` gate. A guard on one command is a guard on nothing.
- Prefer deleting to fixing. Two of everything is how this repo got into trouble; when you
  find a second implementation of something, the task is to remove one, not to reconcile them.

## Verify

`cargo build --all-targets` → `cargo test` → `cargo clippy --all-targets`. Report honestly:
unverified is not done, and a skipped step is a said-so, not a done.
