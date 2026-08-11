# Part I — Principles

*[Shall v7](../SPEC.md) — the map is there; this is one part of it.*

**P1. Every imperative command is a shortcut for editing a file and syncing.** Nothing can
be done only imperatively. If a command can make a change that no file could have made,
that command is a bug.

**P2. There is no legacy.** There are no existing users and we do not want legacy. No
migration path, no converter, no compatibility markers, no deprecation warnings, no
old-format readers. Every "legacy" branch in the codebase is dead weight for nobody. Delete
on sight.

**P3. Fail loud, never silent.** Every bug in this codebase is the same bug: something
didn't work and said nothing. When the choice is between a wrong answer and a visible
error, take the error.

**P4. A fact lives in one place.** A fact stored twice is a fact that will disagree with
itself. Compute, don't copy.

**P5. A default without a reason cannot be safely changed.** If you add a number, add the
reason. If you can't state the reason, don't add the number.

**P6. A comment states a constraint the code can't show. Nothing else.** Not what the line
does — the line does that. Not where it came from — git does that. Not that it's good —
that's the reader's call.

**P7. Shall is not Linux-first, whatever the name says (owner ruling, 2026-07-23).** Windows
and macOS are not ports and not a later phase. **A feature designed for one system is not
finished until the other two have an equivalent or a stated, written reason there can be
none** — and "the Linux tool has no counterpart" is a reason only after someone looked. The
name is a pun, not a scope.

This is a design rule, not an aspiration, and it has teeth in three places:

- **A new statement or backend arrives with its adapters, or arrives with the gap named in
  this document.** `setting:` shipped speaking `gsettings` and nothing else, and the Windows
  registry — the one store on any platform that answers a read-before-write query cleanly —
  was never even filed. That is the failure this rule exists to stop repeating.
- **A refusal beats a pretence.** A statement with no adapter on this host errors and names
  what is missing; it never reports success (X.4's `SettingStore::None`).
- **The competition is Linux-only.** Nix, decman, metapac and the rest stop at the Linux
  boundary. Being the tool that declares a Windows machine as readily as a Debian one is not
  a courtesy to Windows users; it is the only ground where Shall is alone.

**P8. Shall does the thing. It does not hand you the thing to do (owner ruling, 2026-07-23).**
Output whose next step is the user retyping it is not a feature — a command that prints lines
to paste into a module has done the easy half and left the half that fails. Where Shall knows
what should happen, it happens: it edits the declaration (`install`, `adopt`, `teleport`) or it
performs the repair. **Two things this does not license**, both already rules here: it must not
rewrite your files unasked (II.16), and it must not act without the plan being visible first
(`plan`, `--dry-run`, the guard). The correct shape is *ask, then do* — never *inform, then
leave*.

---

