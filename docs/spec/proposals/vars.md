# Part IX — Proposed: user-defined `when` variables

*[LiNix v7](../../SPEC.md) — the map is there; this is one part of it.*

**Status: BUILT 2026-07-20 (fourth session), position 4. Migrated into Part II as II.6b (fifth
session); this part is now the design rationale and open-question register, not the target
state.** Owner ruled the full programmable model (IX.6): typed values (W2), a line-file provider,
an external-executable provider, and an embedded Rhai provider (`vars.linix`) with a clock/shell/
files/env/network standard library. Providers are chosen by filename with a `[vars] source`
selector; a plan freezes its resolved variables. The canonical description now lives in **II.6b**,
with V.51–V.54 for the why. **The register is now closed: W1–W14 are all built** (W5, W11 and
the W8/W13 messaging landed 2026-07-20, fifth and sixth sessions). Decisions are numbered
`W1…W14` in IX.7; the fourth-, fifth- and sixth-session entries in Part VII are the build
record.

**How far customization goes here is an open discussion, not a settled question (owner, 2026-07-19).**
IX.5 draws a narrow boundary and IX.7 recommends holding it in several places. **Read those as
one position in a live argument, not as the decision** — the scope itself is what is being
decided. See IX.6.

## IX.1 The problem, and the rule it argues with

`when` today takes five keys — `os`, `arch`, `host`, `hostname`, `family` (II.2) — and all five
are **detected facts**. So the only way to say "this machine is my travel box" is to say
`when host == thinkpad`, in every file that cares. Buy a new laptop and you edit all of them,
and you have to remember which of nine hostname tests meant "travel" and which meant "this
specific machine, for a reason".

**The hostname is not the intent. It is a proxy for the intent, repeated until it rots.** A
variable lets the intent be named once and bound to machines in one place.

This argues with II.1: **facts about this machine are detected, never configured.** The rule
exists so a fleet does not need hand-maintained state on every box forever, and it is a good
rule. **The proposal below does not break it, and that is the load-bearing claim of this part:**
a variable is not a new fact, it is a **name for a condition over existing facts**. The `vars`
file is committed, shared, and identical on every machine; each machine derives its own values
from what LiNix already detected. Nothing is typed per box.

The place that claim gets tested is W7 — variables that genuinely cannot be derived.

## IX.2 The file

**`vars`**, alongside `active`, `priority` and `schedules` in the repo (II.1). A line file, not
TOML — because it needs `when`, and `when` is a line-file construct. `preferences.toml` is the
wrong home for the same reason.

```
modules/            your lists
profiles/           your choices
active              which profiles are on
priority            which backends, in order
vars                your own names for conditions      ← new
schedules           when LiNix runs itself
```

A new statement form, legal **only** in `vars` — the same way `schedule:` is legal only in
`schedules`:

```
NAME = VALUE
```

Value is verbatim to end of line, trimmed, exactly like a block-form option value (II.2). And
`when` works here as it works everywhere (II.2: *one rule, everywhere*):

```
role = desktop                      # the default, always present
gpu  = none

when host in [thinkpad, x220] {
  role = travel
}

when hostname == render-01 {
  role = workstation
  gpu  = nvidia
}
```

Then, anywhere `when` is legal:

```
when $role == travel {
  apt:mosh
  apt:tlp
}
```

## IX.3 Every variable is always defined

**A variable must have a top-level, unconditional definition. A `when` block overrides it; it
may not introduce a name.** Referencing a name that `vars` never defines at top level is an
error naming the file.

This is the rule that makes the rest work. Without it, `role` defined only inside
`when host == thinkpad` is *undefined* on every other machine, and `when $role == travel` on the
desktop has no answer — leaving a choice between "undefined is an error" (which breaks every
machine that is not the laptop) and "undefined is silently false" (which turns a typo into a
block that never fires and never complains). **Requiring a default deletes the question instead
of answering it.**

So: `$role` is always defined, everywhere, on every machine. A typo'd name is always an error.
Neither one depends on which box you are sitting at.

**Two `when` blocks that both match and set the same variable to different values = ERROR**,
naming both lines — II.7 rule 5, unchanged. Top-level-then-override is not a contradiction
because the default is not a claim about this machine; two matching blocks are.

## IX.4 The sigil, and why there is one

`$role` in a condition, bare `role =` in `vars`. **The `$` is not decoration — it separates two
namespaces that must never merge.**

Without it, `when role == travel` and `when os == linux` are the same syntax over different
things, and the day LiNix detects one more fact — `distro`, say, or `init` — every user who
named a variable `distro` has a file that silently changes meaning. **A detected-fact namespace
that can never grow is a worse cost than one character.** With the sigil, LiNix can add facts
forever and no user file is affected.

Two consequences, both intended:

- **A variable can never shadow a detected fact.** `$os` is not `os`; defining `os = …` in
  `vars` is legal and useless, and probably deserves a warning rather than an error.
- **Reading a condition tells you where the answer comes from.** `$role` is something you
  decided; `family` is something the machine reported. That distinction is worth seeing.

## IX.5 What the narrow version is not

**This section is a proposal for where to draw the line, and the line is not agreed** — IX.6
holds the open question. What follows is the narrow end of the range, written out so there is
something concrete to argue with.

**Not a template language.** Variables are legal **in `when` conditions only** — not in package
names, not in hook commands, not in `link:` targets, not in option values.

`link:~/.config/$role/init.lua` is the obvious next request and the answer is no, for now. The
moment a variable can appear in a path or a command, LiNix owns string interpolation, escaping,
quoting, and the question of what `$role` means inside a value that II.2 defines as *verbatim* —
and a package manager that has grown a templating language has stopped being a package manager.
The narrow version is useful on its own; the wide version is a project. See W9.

**Not computed.** No `$a = $b-heavy`, no arithmetic, no conditionals in values. See W10.

**Not per-machine input.** The `vars` file is committed and identical everywhere. See W7 for
the case this does not cover.

## IX.6 RULED (owner, 2026-07-20): a real language, with the clock and the shell

**The open question below is answered, and the answer is past the far end of the range the
section drew.** It is recorded here in full because everything in IX.1–IX.5 was written assuming
position 1, and **most of it is now superseded** — read this before believing anything above it.

### What was ruled

**A variable's value may come from a program, and that program may read the clock, run a shell
command, and reach the network.** Position 4 in the table below, deliberately, with eyes open.

### The consequence that forced a second ruling, and how it is closed

A value that can move between two commands breaks the one promise the plan makes. `linix plan`
resolves `$x` at 11:59:58 and shows nothing to do; the `sync` you confirm at 12:00:01 resolves it
again, gets a different answer, and removes forty packages **the preview never showed**. That is
not a bug to be fixed later — it is what "the value moved" means, and it makes `plan` a lie.

**So: variables are resolved exactly once per invocation, and a plan carries its resolved
variables with it.** The `sync` that executes a plan uses *the plan's* values, not freshly
computed ones. **The preview and the action agree by construction rather than by luck** — which
is the property II.8 rests on, and the only condition under which the clock is admissible at all.

### One contract, two providers

**A vars provider produces `name → value` pairs. That is the whole interface**, and it is the
reason this is one feature rather than two:

| provider | what it is | costs |
|---|---|---|
| **embedded (default)** | a script LiNix runs itself, in a language it ships | nothing to install; **runs identically on every machine in a fleet**, which is the entire argument for having a default at all |
| **external** | any executable — `vars.py`, `vars.sh`, `vars.js` — run by LiNix, printing `name=value` or JSON on stdout | only works where that interpreter is installed, so a fleet inherits a dependency |

**Both satisfy the same interface and neither is a special case of the other in the code.** The
resolver knows about providers; it does not know about Rhai or about Python. *(Language choice
for the embedded provider is an implementation decision, not a spec ruling — recorded where it
lands, not here.)*

**The simple line-file of IX.2 remains, and is not a third provider.** It is the same contract
with a trivial implementation: a file of `name = value` lines with `when` blocks produces
name → value pairs like everything else. What it is *not* is a second resolution path.

### What this supersedes above, explicitly

**IX.5 is dead as written.** "Not computed" is reversed outright. "Not per-machine input" is
reversed the moment a provider can run `hostname` or read a file. **What survives IX.5 is the
narrow claim about `when` conditions**, and even that is now a separate question (W9) rather
than a consequence of the scope.

**The W register's recommendations are void, not adjusted.** IX.7 says plainly that every
recommendation in it "assumes position 1, and every one of them changes if the answer is 2 or
higher." The answer is 4. **W1–W14 must each be re-asked against this ruling before any of them
is implemented** — carrying a position-1 recommendation into a position-4 feature is exactly the
stale-✅ failure this document exists to stop. The ones that visibly change shape:

- **W2 (types)** — "strings only" was cheap when values were typed by hand. A provider returning
  JSON has types already, and throwing them away at the boundary is a choice that needs making.
- **W4 (when `vars` loads)** — now *the* load-bearing entry. Providers may shell out, so
  resolution has a side-effecting phase, and the once-per-invocation rule above lives here.
- **W7 (the undetectable variable)** — **answered for free.** A provider can read anything, so
  there is no escape hatch to design. This is the entry that had no recommendation.
- **W10 (variables referencing variables)** — inside one script this is just a local, and the
  question shrinks to whether one *provider* can see another's output.
- **W13 (the guard)** — sharpens badly. A variable that moves on its own can now deactivate a
  profile with no edit to any file, so **`plan` must show the variable as the cause**, and the
  guard is protecting against a change nobody typed.

### The open discussion this replaces

Kept for the argument, which is still the right argument even though the answer went the other
way.



**Recorded at the owner's instruction, 2026-07-19.** Everything above proposes the narrow end
of a range. **The range itself has not been chosen**, and the choice is not a detail that falls
out of implementation — it decides what LiNix is.

The positions, roughly, from narrow to wide:

| | what a user can express | cost |
|---|---|---|
| **1. Named conditions** *(IX.1–IX.5)* | `$role == travel` in `when`, and nowhere else | almost none; the feature is a lookup table over detected facts |
| **2. Values in declarations** | `link:~/.config/$role/init.lua`, `@version=$pinned` | LiNix owns interpolation, escaping, and what `$` means inside a value II.2 defines as *verbatim* |
| **3. Derived values** | `tier = $role-heavy`, string ops, defaults | ordering, cycles, and a small expression language |
| **4. A configuration language** | conditionals in values, functions, imported var files | LiNix is now Nix, with worse ergonomics and none of the guarantees |

**Nothing forces these to be taken in order, and nothing forces stopping.** That is the danger:
each step is individually reasonable and argued for by a real use case, and the sum is a
different product. Equally, stopping at 1 may simply relocate the problem — a user who cannot
write `$role` in a path will write two nearly identical `link:` lines under two `when` arms, and
duplication is the thing this part exists to remove.

**Several register entries are the same argument in local form and should not be ruled on
piecemeal:** W9 (interpolation outside `when`) is the 1→2 boundary. W10 (variables referencing
variables) is 2→3. W2 (types) and W3 (bare `$flag`) are the shape of the expression language, if
there is to be one. W7 (the undetectable variable) sits outside the range — it is a question
about *where values come from*, not how far they reach, and it can be answered independently.

**Deciding the extent first makes the rest cheap; deciding it last means rewriting the parser
and the resolver at whichever step the boundary actually lands.** The recommendation in each
entry assumes position 1, and every one of them changes if the answer is 2 or higher — so
**they are conditional recommendations, not a consistent set to be adopted line by line.**

The open question, stated plainly, so it can be answered in one sitting: **is `vars` a lookup
table, or the beginning of a language?** No date, no owner, no deadline attached — but no code
should be written against Part IX until it is answered, because position 1 and position 3 do not
share a parser.


---

**Decisions: W1–W14.** They live in [the decision register](../decisions.md), with a status on
each — this part states the shape, the register states what is still unanswered.
