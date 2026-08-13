# Part V — Why

*[Shall v7](../SPEC.md) — the map is there; this is one part of it.*

> **Do not change a Part II rule without reading its entry here.** Each is the scar of a
> real bug.

**V.1 — Why `-g` died.** `Config::groups_dir` meant two things: the wish-list folder, and
the anchor for `locks.json` / `keep.txt` / `local.txt` / profiles. `-g` moved both, while
`registry.json` — the ownership record — never moved. So `plan -g /B` read /B's one package
against an ownership record claiming 579, called 578 of them drift, and purged the machine.
`-g` is gone because "which folder" stopped being a question anyone asks: files are storage,
modules are the unit, profiles choose.

**V.2 — Why profiles choose and modules hold.** It's the one sentence that explains the
whole system. The moment profiles hold things or modules make choices, it stops being true.
A module can never reference a profile (the layering rule) because otherwise "what does
`editors` contain?" has a different answer depending on what you activated — the library
cannot depend on the app.

**V.3 — Why a profile may still hold packages.** Decided knowingly against V.2's tidiness,
because `--into Work` is a real want. The cost is real: those packages are unshareable
forever, and you find out the day you want to share them.

**V.4 — Why `group:` and `include:` died.** `group:editors` pointing at a file was **already
a no-op** — the resolver seeded every `.txt` unconditionally, so the file was loaded before
you named it. It looked like opt-in and wasn't, which taught people a wrong model of how
Shall decides things. `include:` strictly superseded it.

**V.5 — Why conflicts are errors.** Files were read in filesystem order and first
declaration won. `a.txt: jq@1.6` vs `b.txt: jq@1.7` was decided by the disk. Sorting the
read order only makes the wrong answer deterministic.

**V.6 — Why `keep.txt` died.** It lived in the groups folder and ended in `.txt`, so the
resolver ate it: *"never remove firefox"* also silently meant *"install firefox"*. It was
held back by a hardcoded one-element denylist. **Separate by location, not by denylist** —
and `forget` gives people the thing they actually wanted, which was a way to make Shall let
go.

**V.7 — Why `absent:` is the one exception to "only removes what it manages".** Because you
named it. Everything else Shall touches, it owns. `absent:` is you reaching outside that,
deliberately, by name. It stays a line rather than a file because a file can't be turned off
per profile, can't be shared, and puts Shall's bookkeeping back in a folder you author.

**V.7b — Why a name no line can hold is protected, and why the escape hatch does not open
it.** `winget list` answers for Add/Remove-Programs entries with pseudo-IDs like
`ARP\Machine\X64\Android Studio`. A package name is one word (II.2), so no module line can
hold that: `adopt` cannot take it, nothing can declare it, and it is therefore **unmanaged
forever** — which made it a standing `purge-undeclared` candidate that `shall adopt` could
never clear. The documented safe sequence, adopt-then-purge, proposed deleting Android Studio.

Removing what you could never have been asked to keep is the inverse of "Shall only removes
what it manages" (V.34), so it is a protection rather than a warning. It is checked **before**
`unprotected_packages`, which is otherwise absolute (V.35): that hatch means *"I manage this
one myself"*, and you cannot manage what you cannot write down — there is nothing for it to
release. Asked through the one grammar, not a second copy of the naming rule.

*Found by the live Windows sweep, where `adopt` wrote those IDs into `modules/adopted.txt` and
every later command — `rollback` included — died parsing the file Shall had just generated.*

**V.7c — Why silence is not a no, and what it costs to say so.** *(Owner ruling,
2026-07-22.)* Every read in this codebase went through `run_output`, which hands back a
failed command's empty output as an ordinary empty answer — deliberately, because a
non-zero exit from `pacman -Ss` or `dnf search` usually just means the query matched
nothing. So a search that could not run and a package that does not exist arrived at the
resolver as the same thing: `false`.

**Three container images hit it for three different reasons in one session.** Fedora,
because dnf5 changed its output format and the parser read dnf4's. Alpine, because
`--no-cache` left no index to search. The `tools` image, because it deletes
`/var/lib/apt/lists` to stay small. Every time, a bare `jq` walked past the system manager
that had it, fell through the whole priority list to cargo, matched a **library** crate
named `jq`, and failed at install — and had that crate shipped a binary, Shall would have
installed the wrong package and **frozen the wrong manager into the lock**, where it would
stay after the index was fixed. The parser fixes removed the day's instances; a dropped
network reproduces the shape on any real machine.

**A hard stop was the wrong answer**, because one flaky manager would then fail a sync that
has nothing to do with it. **The lock is the thing to withhold, not the install.** So the
name still falls through, and what changes is what gets remembered: a pick made past a
silent manager is never recorded, so the next sync re-asks and moves the package once the
index is back (II.7b). The cost is one extra probe per affected name per sync, which is what
the owner ruled acceptable — *"it's just about efficiency."*

**What counts as silence has to be conservative in the other direction**, or the lock never
gets written. A non-zero exit alone is an ordinary empty result for pacman, dnf and brew, so
the signal is a non-zero exit **with a complaint on stderr**: `search_output` in the executor,
used by every backend's `search` and nothing else. A manager this machine does not have, and
one with no search facility at all, still count as a plain no — those are settled facts, and
re-asking would get the same answer forever.

**One gap survives, knowingly.** `apt-cache search` with an empty index exits zero and says
nothing, which is indistinguishable from a real miss. There is no generic signal left to read
there; it needs a per-manager index-health check, which is a different feature.

**V.8 — Why blocks use `{ }` and not `( )` or `end`.** `( )` is already the grouping operator
in profile math — same character, two meanings, the trap we removed from `include:`. `end` is
clumsy. "Pick your own delimiter" means nobody can read anyone else's files.

**V.9 — Why block values are verbatim and `#` doesn't comment inside them.** Fail loud. If
`#` commented there, `after_install = curl -H "X: #tag"` silently truncates and runs the
wrong command. The other way, `version = 1.6 # my pin` gives a version the parser visibly
rejects. **You reached for the block form precisely because you needed a value the short
form couldn't hold. Verbatim is what you asked for.**

**V.10 — Why no quotes.** `"` needs `\"` needs `\\` needs a newline rule. The block form
makes the problem stop existing rather than giving it a rule.

**V.11 — Why the extension is cosmetic.** Nothing is active unless a profile names it, so
`use editors` against a misnamed file says *"no module named `editors`"* with a list. **The
reference is the safety net**, not the extension.

**V.12 — Why adopt takes manual-only.** Not because 579 is a big number. **Declaring a
dependency breaks dependency management.** Put `libgpm2` in a module and you've declared it,
so Shall keeps it forever; remove vim and it stays, because apt says "orphan" and your file
says "I want this" and the file wins. Monday's bug was claiming ownership of a set that was
never Shall's.

**V.13 — What "estimate" means.** apt records that something was **explicitly requested** —
not **who** requested it. Canonical's installer marked ~90 packages manual at image-build
time; they are indistinguishable from the `apt install vim` you typed. There is no field for
"a human, on purpose." **(measured)**

**V.14 — Why the priority order.** Most of the current 10-backend order is **meaningless** —
apt, pacman and dnf never coexist. The order that decides something is **system manager vs
language manager**: if both apt and cargo have `ripgrep`, the **system one wins**, because
your distro maintains it and updates it with everything else. Language managers are for what
your distro doesn't carry. That also explains pip last: it installs into your system Python
and can break it. *(uv and pipx being absent from the order is simply a bug.)*

**V.15 — Why `priority` also means "enabled".** One list, one question: *which package
managers does this setup use, and in what order.* It replaces four settings for one fact
(`backend_priority`, `enabled_backends`, `hostname_backends`, `default_backend`) of which
only two merge today. An explicit `snap:foo` failing when snap isn't listed is a feature: it
catches typos and makes your backend set declared rather than inherited.

**V.16 — Why unpinned names get locked, per machine.** Shall *probes* — "does apt have
ripgrep?" So `ripgrep` lands on cargo today, Ubuntu adds it tomorrow, and the same unchanged
line resolves to apt: Shall uninstalls from cargo and installs from apt because a repo you
don't control changed. **The unpinned name is the question; the lock is the answer.**

**The answer is per machine, and the lock is not a demand.** `locks/` travels with the config,
but *which manager has ripgrep* is a fact about a host, so one shared file would have the
Ubuntu and Fedora boxes overwriting each other's answer on every sync — churn in a tracked
file and a merge conflict every time. One file per host (II.6) settles that. And a lock naming
a manager this machine does not have is re-asked, not obeyed: it exists to stop an unedited
line quietly changing meaning, which is a different thing from insisting on a manager that
isn't here. Insisting is what a pin is for, and a pin is written on the line (II.7b).

**Where this came from:** a config that resolved `jq` to apt on one box and then moved to a box
without apt. The lock was honoured, apt was asked, and the run went wrong in a way no wording
of the lock rule could fix — because the lock was answering a question about the wrong machine.
The fix is not a better fallback inside the lock; it is that the line says what it will accept
and the lock only ever records what happened here.

**V.47 — Why a `repo:` line names its backend.** *(Decided 2026-07-17.)* A repository belongs
to exactly one package manager — a PPA is apt's, a COPR is dnf's, and `add-apt-repository`
run against dnf is a system command that fails, or worse, half-succeeds. A bare `repo:SPEC`
would make Shall guess which backend, and the honest ways to guess are all wrong: a
prefix→backend table (`ppa:`→apt) is a second copy of a fact each backend already owns and grows
with every ecosystem (P4); "the one system backend in `priority`" fails at run time on the
machine where the guess is wrong, which is the machine you least want a repo command
misfiring on. So the backend is named, exactly as a package line names one: `repo:apt:ppa:...`.
It is refused when the backend is not in `priority` (V.15), and a bare `repo:` is a parse
error that says so — caught in the file, not at the command. **The repo and the package it
serves already sit together in a module (II.16); naming the backend once more is the cost of
never running the wrong tool.**

**V.17 — Why regex is live by default.** "Give me all the fonts, including ones that don't
exist yet" is real. Mandatory locking turns a living pattern into a frozen list and defeats
the point of writing a pattern. **The lock file is the switch** — that's how every lockfile
already works.

**V.18 — Why regex matches names, not meaning.** `photo*` finds `photocollage`,
`photoprint`, `photoqt` — and misses `gimp`, `darktable`, `krita`, `rawtherapee`,
`shotwell`, `digikam`, `inkscape`: every actual photo editor. Real prefix *families* are the
good use (`texlive-*`, `fonts-*`). Debian's own answer to a family is a **metapackage** —
someone's judgement rather than a naming coincidence — and better where one exists.

**V.19 — Why `max_removals = 20` works and `max_installs` has no default.** **20 is more
than a person removes on purpose** — calibrated against human behaviour, so a plan removing
50 is wrong at any scale on any machine. **Installs have no equivalent ceiling: the biggest
install you'll ever do is the correct one** (a fresh machine). So `max_installs` exists but
defaults to unset — the number is yours, for your reason. *(Rejected: screen height — the
same command would behave differently on different machines. Rejected: a ratio — a fresh
machine's ratio is undefined.)*

**V.20 — Why the ratio catches Monday and a count doesn't.** On Alpine, `adopt` correctly
took 14 packages and a mis-scoped `prune` scheduled all 14 for removal — **under the count
limit, none protected, all things you'd cry about**. The count misses it on small machines.
**Manage 3, delete 576 → you have made a mistake, on every machine, always.**

**V.21 — Why `purge-undeclared` is a command and not a mode.** **Sync is then never
dangerous** — not "safe by default", but safe permanently. No setting anyone can flip,
inherit, or copy from a dotfiles repo makes a routine sync delete something it didn't
install.

**V.22 — Why `-y` cannot skip a refusal.** Every CI job and every script passes `-y`, and an
unattended run cannot notice a machine being dismantled. **`-y` means "don't ask me". It has
never meant "ignore your safety rails", and every place it currently does is a bug.**

**V.23 — Why `confirm_destructive` died.** In a declarative system, **deleting a line is the
confirmation.** You said what you wanted; asking whether you meant it is asking twice. And
the setting named after removals gated a module-file overwrite (not a removal) while missing
both `prune` and `sync`.

**V.24 — Why the plan always leads with counts.** **A warning that only fires sometimes is a
mechanism that can be miscalibrated. A summary that's always there can't be.**

**V.25 — Why the 16 protections became 5.** **Eleven of them were never protections — they
were declarations wearing a protection costume.** "Don't remove this, it's leased" →
`@expires=`. "…you installed it imperatively" → it's in the `imperative` module like
everything else. "…it's held" → `@hold`. "Do remove this, it's bloatware" → `absent:`. Each
existed because there was **no way to say the thing directly**, so someone bolted an
exception onto the removal path instead. `protect_imperative` is the clearest: it exists
*purely* to stop drift-pruning deleting `shall install`-ed packages, because they lived in
`local.txt`, which `-g` could move out from under the registry. **Someone met Monday's bug,
understood the symptom exactly, and patched it with a flag.** Not one behaviour was deleted;
they moved to where they were always trying to be.

**V.26 — Why protection is a refusal, not a declaration.** Everything else is a statement of
intent ("I want this"). Protection is "I will not do that, and there is no flag." It doesn't
care whether the package is managed, declared, adopted, or predates Shall. That's why it
lives in preferences and not in a module — and why deleting a declared `apt:python3` line
makes Shall refuse until you unprotect it.

**V.27 — Why hooks are lines despite the supply chain.** `use` is **already** a trust
decision: a `repo:` line in someone's module means they can ship you any package with any
script in it. Hooks make that road shorter, not different in kind. **The lock is the
approval** — because you approve a script once and they edit it three months later, which is
how most npm incidents actually worked: the malicious version was never the one anyone
reviewed. **Hash everything, including your own scripts**, because "did I write this?" has
no clean answer once you've cloned your own repo onto a second machine — and the friction
that catches you editing `setup.sh` is the same friction that catches a teammate's `git
pull`.

**V.28 — Why schedules got their own file.** `active` answers exactly one question: *what is
this machine set to right now?* A schedule is written once and forgotten — a fact, not a
switch. An active-list for schedules would invent a state that needn't exist ("defined but
off"), so you'd check two files for one fact. And the separate file means a cron job can't
arrive via `use` at all. **Door left open, deliberately unbuilt:** "sync nightly when I'm in
Work" — a `schedule:` line can live in a module and be selected by a profile; the grammar
already allows it.

**V.29 — Why `@requires` survives.** **(verified, `planner.rs:407-426`)** `spec.requires`
becomes a real `graph.add_edge` — install **ordering**. A module is a *set* and says nothing
about order. `@requires` is the one thing modules can't say. It matters only for things
outside a package manager (a `.deb` from a URL, a GitHub binary) — things with **no one to
ask**. apt's own dependencies are ordered for free at `planner.rs:427`.

**V.30 — Why git is the history.** **Shall commits only on a successful sync, so every
commit is a state your machine actually reached** — not one you asked for. `git log` is
where your machine has been; `git diff` and `shall plan` are the same question; rollback can
never take you somewhere that never worked. And the registry needs no history, because
declaration + convergence reproduces it.

**V.31 — Why no commit algebra.** Set math works on profiles because they're choices you're
making *now*. Commits are moments that already happened, and "the union of March and today"
isn't a machine anyone asked for. Git covers what's real. **Intersect of commits does not
exist in git and no use case was found** — twenty years of git not having it is evidence.

**V.32 — Why lock signing died.** **Signing one file in a folder of unsigned files protects
nothing.** Anyone who can edit `locks.json` can edit your modules — they'd change `apt:jq`
to `apt:evil` and no signature would notice. It guards one door in a building with no walls.
Ours was `sha256(key + "|" + text)` — a construction cryptographers warn against — compared
with `==`, which leaks timing. And **appearance is worse than nothing, because you stop
looking.** `git commit -S` signs everything, with real crypto, verified by a tool that's been
attacked for twenty years.

**V.32b — Why the check reads git's verdict and does not compute one.** Shall runs `git log
--pretty=%G?` and carries the letter it gets back. It does not decide what a key is worth,
because that is the twenty-year-old tool's job and re-deciding it is how the previous signing
scheme ended up with `sha256(key + "|" + text)`. The same reasoning splits `Good` from
`Unverified`: git distinguishes a signature it trusts from one made by an untrusted, expired or
revoked key, and folding the second into the first would restore exactly the appearance-without-
protection V.32 is about. **And why the refusal is off by default:** a rule that fires on every
rollback in a repo nobody signs is a rule that gets turned off, at which point the signed case
is unprotected too.

**V.33 — Why `clone` died.** It copied **the installed set, not the intent** — you got a
machine with the same packages and no idea why. `git clone && shall sync` gives the intent,
the history, the pins, and the ability to change it afterwards.

**V.34 — Why `prune` and `orphans` died.** sync removes drift by definition, so `prune` is
sync with the install half amputated. "Prune" meant four unrelated things; deleting the
command leaves exactly one meaning ("delete old history") for the first time. `orphans`
shows what sync would remove, which is `plan` — and its message named two commands and
described neither.

**V.35 — Why `--backend` is refused on removals.** A scoped removal is Monday's exact shape:
**you narrow what Shall looks at without narrowing what it owns**, so everything outside the
scope looks like drift.

**V.36 — Why `clean` survives.** It's apt's housekeeping, not Shall's drift, and only apt
knows about it. It goes through the guard because `autoremove` is a mass removal Shall
didn't plan and has famously eaten desktop environments. It stays explicit because automatic
cleanup is a surprise removal.

**V.37 — Why suspensions survive.** Nearly deleted — "I want this and I don't want this"
smells like a contradiction with a timer. The case that saves it: **"take the game away
until the weekend."** People genuinely do that; nothing else here does it; and once leases
exist, suspensions are the same machinery pointed the other way.

**V.38 — Why times are absolute.** "2 hours" can't work in a file: the machine reading it
next week has no idea when you wrote it. That's exactly why `@lease=2h` is inert today.

**V.39 — Why `install`/`uninstall`/`forget`.** A symmetric pair plus one word that can't be
misread. `remove` and `unmanage` sat one word apart and did opposite things to your disk —
reach for the wrong one and you don't get an error, you get a deleted package.

**V.40 — Why three landing modules.** Provenance ends up in the filename: open
`modules/hooks.txt` and see exactly what got in behind Shall's back. One `local.txt` mixes
them and forgets which was which.

**V.41 — Why "detected, not configured".** Shall should not be *told* you have btrfs; it
should look. Not told you have four cores. Almost every "local fact" in `config.toml` is
something Shall could work out in a second and instead asks you to maintain by hand, forever,
on every machine. **That is not configuration, it's homework.**

**The `max_parallel` exception (owner ruling, 2026-07-17).** This rule's first draft called
`max_parallel` homework too — and noted it was overwritten at `sync/mod.rs:296` anyway, "so the
setting is already a lie." Both halves are now dead: the overwrite is gone (`sync/mod.rs:293-297`
reads it as *"the user's knob"* and honours `self.config.max_parallel.max(1)`), and the owner has
ruled to **keep** it. The distinction that saves the rule: the core count is a *fact* (detected),
but *how many of those cores to use* is a *preference* — you may want to cap it to keep the
machine responsive while a big sync runs. A preference Shall cannot look up is not homework. So
`max_parallel` stays: detected as the default, overridable by hand.

**V.43 — Why the guard has ten refusals and not five.** The first draft said five (then
listed six). It was written before anyone re-read `policy.toml`, which held five rules and
was marked in II.17 as moving to `[guard]`. Two of them had somewhere to go —
`deny_packages` was already in the list, and `allow_backends` is what the `priority` file
means (V.15). **The other three had nowhere, and "delete" was never decided — it was
overlooked.** `pinned_only`, `require_snapshot` and `deny_vulnerable` are all exactly the
shape V.26 defines: not "I want this" but "I will not do that". They are refusals, so they
live where refusals live, and `-y` cannot skip them for the same reason it cannot skip any
other (V.22). *Corrected knowingly against the headline: a wrong number in a document is
cheaper than three deleted safety rails. If a rule here ever stops being a refusal and
starts being a preference, that is the signal it does not belong in `[guard]`.*

**V.46 — Why set math costs a package its module name, and why `include` died.** *(Decided
2026-07-17, during Phase 2f. II.4 required set math and nothing implemented it:
`model::profiles::evaluate_expression` had no caller outside its own tests, and the only
working implementation was `compose()` in the old `app/profile.rs`, over flat strings.)*

**The shape does not fit, and pretending otherwise is the bug.** Resolution is
`profiles → the modules they reach → the packages in those modules`. Set math breaks that
chain: `(Work | gaming) & security` is **an intersection of package sets**, and there is no
module whose contents are that intersection. So a profile using set math resolves to packages
directly rather than naming modules.

Making `&` operate on module *names* was the alternative, and it answers a different question
than the one asked: the intersection of `{editors}` and `{security}` is empty even when both
hold `vim`. Inventing a synthetic module to hold the result was the other, and it names a
module that does not exist on disk, so `upgrade --module` would match something nobody can
open.

**The predicted cost turned out not to exist, and that is worth stating plainly because this
document predicted it wrongly.** The first draft of this entry said set math costs a package
its module name. It does not: the implementation maps expression atoms back to **the
statements they came from**, not to strings, so a package that survives an intersection still
carries its `Origin` — its file, and therefore its module. `upgrade --module editors` finds
`vim` through an `exclude`. There is a test (`a_package_surviving_set_math_still_knows_its
module`). The only lines that get profile scope alone are ones written in the profile itself,
including a bare package atom inside an expression — which is correct, because that line
really is in the profile. **Keep mapping back to statements. Mapping back to strings is what
would make the predicted cost real.**

**`include` died because `use` already is it.** II.4 listed `include`/`exclude`/`intersect` as
the three directives while II.2 listed `use NAME` as the way to reference a module or profile
— and for the union case those are the same operation with two names, which is the exact
"two ways to do one thing" disease this design exists to cure, sitting inside the spec. `use`
wins: it is II.2's word, it is the one modules use too, and one word for "bring this in"
everywhere beats two. `include` is an error that says so.

**V.42 — Why the comment rule.** This codebase has been touched by many AIs, and this is what
that leaves behind: models narrate what they just wrote and congratulate themselves for it,
because that reads like effort, and each one looks fine on its own. The repo already proves
the rule works — `core/manager.rs:86-93` explains *why* the `tracks_manual` gate exists and
what happens if it's wrong; `generic.rs:363-370` explains in nine lines that choco lists
Title-case "Wget" for install-id "wget" so `remove` silently no-ops, and why the fix must be
Windows-only because npm has `socket.io`. **Those two are worth more than the other 137
combined, and they're the same length.** The cost of the rest is that **they trained everyone
to skip** — the reason 32 comments in this repo are outright false, each of which someone read
past. *(The first draft's example, `audit()` documented as "a **destructive** Discovery cycle …
without generating files or acquiring state", has since been fixed in the code and now reads
correctly. The measured 32 are the ones that remain.)*

**V.44 — Why `activate` writes a list and there is no `-r`.** The file is the state, so a
command that activated *without* writing `active` would be a second place the answer lives —
the exact defect `-g` and `keep.txt` died of (V.1, V.6). Set, add, subtract, because those
are the three things you do to a list. **`deactivate` rather than `activate -r`** because
`install`/`uninstall` already settled that the opposite of a verb is a verb (V.39), and a
flag that silently inverts a command is how you delete something at 2am by leaving off one
character. The empty list is the one refusal: `shall activate $PROFILE` with `$PROFILE`
unset would otherwise read as *"turn everything off"* and be perfectly valid. The guard would
catch it (V.19) — but the guard is for decisions you meant, and this one nobody means.
**`activate NAME…` still overwrites `when` blocks without asking**, and that is not an
oversight: it is the set form, it sets, and a form that quietly kept part of the old file
would leave the machine in a state you did not type. The file is in git; that is what git is
for (V.30). **It does not ask and it does not stay quiet** — it names each block it removed.
*Asking and reporting got argued as one thing and they are not: the case against a prompt is
that overwriting the list is the command's own job (S6), and none of that is a case for
hiding what the job did.*

**Why `deactivate` reaches into a `when` block when `activate -a` does not** *(decided
2026-07-17, after the first draft of this entry said the opposite)*. The first rule here was
that Shall never edits a block — a block is something you wrote — so `deactivate Travel` would
remove the top-level line and report *"it is still activated by the `when` block on line 4."*
**That sentence is the argument against itself.** It is a command named "deactivate"
announcing that it did not deactivate. **A verb that reports the state it failed to reach is
the `-g` disease in miniature: the name says one thing, the file says another, and you find
out later.** So it removes the name wherever this host would read it, and the empty block goes
with it.

**The asymmetry with `activate -a` is real and it is not a compromise: adding has a choice of
where to put the name, removing has none.** `-a` appends at the top level because a block is a
rule you wrote and a new name has no business joining it — there is a right answer and it is
"outside". `deactivate` gets no such freedom; the name is where it is, and the only way to
leave the block untouched is to not do the job.

**And why it stops at blocks that do not apply to this host.** Not caution — the same rule,
read carefully. `deactivate` turns off what is on; on the desktop, `when host == laptop {
Travel }` has nothing on, so there is nothing to turn off, and removing the line would be a
different command (*"never activate Travel anywhere"*) that nobody typed. **`active` is a file
you commit and share (V.30), which makes "edit it wherever the name appears" a way to change a
machine you are not sitting at from one you are.** The blast-radius reasoning is V.22's, and
it lands in the same place: **the refusal is cheap and the mistake is not.** It says why, and
names the line, so the hand-edit is one keystroke away for the person who did mean every
machine.

**V.45 — Why a cycle is an error and not deduped.** If `active` were the only consumer you
could visit each profile once and move on, because union doesn't care how many times it sees
a name. But profiles have `&`, `\` and `-` (II.4), so `Work include Gaming` /
`Gaming exclude Work` has no answer to settle on — not a redundant answer, **no answer**.
Deduping picks whichever order the resolver happened to walk in, which is V.5's defect
wearing a different hat: files were read in filesystem order and first won, and the fix was
to stop guessing and say so. Naming the whole loop instead of the last edge is II.2's rule —
the error names the file and the line — and it is the difference between *"there is a cycle"*
and a user who can see which of the three lines they meant to delete.

**V.48 — Why an artifact is selected and not scored.** *(Adopted 2026-07-20 from Part VIII;
owner rulings D3, D3b, D4.)* The bug this prevents was live in the tree, not hypothetical.

`GithubBackendCore::score_asset` added points for an OS token, points for an arch token,
points for looking like an archive, five points for `musl`, and took `max_by_key` over the
result. **Three separate defects, each of which shipped:**

1. **It picked a maximum even when the maximum was negative.** A release offering nothing this
   machine could run still returned an asset, which was then downloaded and unpacked. The
   failure surfaced later, somewhere else, as a binary that would not execute.
2. **`name.contains(arch)` is a substring test.** On a 32-bit box `x86` matched inside
   `x86_64`, so the wrong artifact scored *higher* than the right one. Substring matching over
   filenames is why the replacement matches whole tokens and lets the longest alias at a
   position win.
3. **There was no tie-break at all**, so between two equally-scored assets the winner was
   whichever order the GitHub API happened to return them in. **The same declaration could
   install a different file on two machines on the same afternoon** — which is precisely the
   property a declarative package manager exists to deny.

**The score also could not be argued with.** A user who got the wrong file had no line to
change, because the answer was a sum of magic numbers with no vocabulary. `formats` replaces
the sum with an ordered list of names the user can read, write and override.

**Why `formats` and `channel` stay two keys.** They look alike — both narrow "which of these
do I get" — and folding them into one key would produce a value whose meaning depends on which
backend answered. That is the `backend_priority`/`enabled_backends`/`default_backend` defect
(V.15) in miniature: one name, several meanings, and no way to tell from the file which one is
in play. **A snap channel is not an artifact.** Snap ships one artifact and several streams of
it; GitHub ships one stream and several artifacts. Two questions, two keys, each an error where
it does not apply.

**Why an unmatched selection is an error and never a fallback.** "Whatever was first" is how
the score behaved and it is what made the bug invisible: something always installed, so nothing
ever looked wrong. The error prints what the release actually offered and why each asset was
passed over, so the fix is visible without opening a browser.

**Why the tie-break is printed and locked rather than merely applied.** Shortest-filename-wins
is a heuristic, and the honest objection to it is that it is indefensible as a written rule. It
survives here only because it is not silent: the plan names what was chosen and what was passed
over, and the lock records the resolved filename so a pinned declaration cannot quietly resolve
to a different file next month. **A guess nobody can see is the guess that drifts; a guess that
is reported is a default the user can override with `@asset=`.**

**Why `@bin=` turns the guess off instead of falling back to it.** A fallback would put the
guess back exactly where the user reached for the option to turn it off — and the case where
`@bin=` is reached for is the case where the guess was already wrong.

**Why several artifacts under one line keep their own names.** *(Owner ruling, 2026-07-21.)*
The repo's name was the deployed name because a line resolved to one file. `@asset=all` breaks
that assumption and nothing else does. The alternative considered was prefixing every file with
the repo's name, which never collides — and which renames the program you asked for, so the
same tool is `bar` from one line and `bar-bar-linux` from another. The collision it avoids is
better handled by refusing: two archives that both contain `bar` are two answers to one
question, and the user has to say which they meant. **Silently deploying the second over the
first would install a file the declaration does not name, which is the class of bug artifact
selection exists to close.**

**V.49 — Why `rebuild` is a separate command that batches per backend.** *(Adopted 2026-07-20
from Part X.1; owner ruling K1.)*

The bug this prevents is the one convergence cannot see. `sync` computes the difference between
the declaration and the machine, so **every failure where the difference is empty is a failure
it will report as success, forever**: the half-configured install, the truncated download, the
closure someone removed by hand. Re-running `sync` on that machine is not a weak repair, it is
a guaranteed no-op, and the user has no way to tell the difference between "nothing to do" and
"nothing I can see".

**Why not a flag on `sync`.** Two reasons, and the second is the one that matters. It is
destructive on a machine that is fine — a flag is one typo from a routine command. And
`schedules` runs `sync` unattended: a mode of sync is a mode a timer can reach, and a timer
cannot be the thing that notices a package is broken. The parser now refuses `run = rebuild`
outright rather than relying on nobody writing it.

**Why batch-per-backend and not the two obvious answers.** All-at-once genuinely forces orphan
collection and can leave the machine without a shell partway through. One-at-a-time is safe and
collects almost nothing, because a dependency shared with a still-installed package is never
orphaned at any instant — it would be a repair that does not repair. **These are different
features wearing one name, and the backend is the granularity at which the underlying question
is even defined:** `apt` cannot orphan a `cargo` crate.

**Why foundation backends go first, and why the original reasoning for it was wrong.** X.1
argued from blast radius — put the risky batch first so a strand lands furthest from the
machine's ability to boot. *That argument does not survive contact:* if `apt` goes first and
`apt` strands, the machine has no shell, which is the worst available outcome, and running it
last would have left it untouched. **The ruling is right for a different reason — dependency
direction.** A crate can need a system compiler; no `apt` package has ever needed a crate.
Rebuilding user-space software first would rebuild it against the system state the rebuild is
about to replace, leaving it stale the instant the foundation batch lands. Foundation is
`needs_root()`, which already draws that line, rather than a second hand-kept list.

**Why removal and reinstall are two transactions.** The transaction engine runs independent
graph nodes concurrently, and a `Remove` and an `Install` of the same package have no edge
between them. In one graph they race, and the winner decides whether the package exists.

**Why a bare `rebuild` warns and proceeds, rather than refusing.** *(K2, owner ruling
2026-07-24, reversing the recommendation this entry originally carried.)* The first answer was
"scope is required — a bare `rebuild` errors and lists the forms", on the reasoning that `--all`
is too large a thing to reach by pressing enter. **The owner ruled the other way, and the reason
is what `rebuild` is for.** Every other refusal in this design guards against *software being
removed*; this one would guard against *software being repaired*. The failure this command
exists to fix is a machine whose declared software is broken while `sync` reports success
forever, and a refusal makes the repair one step harder to reach while doing nothing whatsoever
about the scope — the user re-runs it with `--all` and gets the identical blast radius, having
learned only that Shall is fussy. **A warning carries the same information and does not stand
between the user and the fix**, and it names the narrower forms in the same breath, which the
refusal also did. This is the one place in this document where the answer to "large and
consequential" is a loud sentence rather than a no, and the reason it can be is that
`rebuild` never touches undeclared software: everything it removes, it removes to put back.

**Why protected packages are dropped from the scope rather than exempted in the guard.** A
rebuild's removal is only safe because a reinstall follows — and if that reinstall fails, the
machine is genuinely without the package, which is exactly what the guard exists to prevent.
Teaching the guard that one caller means it differently would make the refusal conditional on
intent, and intent is what every caller claims. Narrowing the scope keeps `rebuild --all` usable
on a machine whose `bash` is protected while leaving the refusal absolute. **The skips are
printed**: a rebuild that silently dropped half its scope would report success over a machine it
never repaired, which is the same lie convergence was already telling.

**V.50 — Why `setting:` is a statement, and why it reads before it writes.** *(Adopted
2026-07-20 from Part X.4; owner ruling.)*

The bug this prevents has two halves, and they need two different rules.

**Why a statement and not a `de:`/`gsettings:` backend.** A desktop is packages plus files plus
a session, all of which already have statements. Inventing a fourth spelling of the same three
things is the two-of-everything failure the rewrite exists to end — and the adapter is chosen by
*what is running*, not by what the user typed, so a `backend:name` prefix would encode a choice
the user does not get to make. A GNOME key and a KDE key are the same declaration; only the tool
that applies it differs.

**Why read-before-write is the whole mechanism, not an optimisation.** A line that shells out
every sync is a hook — a command that runs whether or not anything changed, and whose effect on
a converged machine is "run `gsettings set` again for nothing". A line that reads the current
value and writes only on a difference is a *declaration*: it describes a state, and does nothing
when the state already holds. The first belongs in `after_install`; only the second belongs in a
model whose entire promise is that a settled machine is quiet. This is also why KDE waits — a
store you cannot cleanly *read* cannot host a read-before-write declaration, so `kwriteconfig`
is not adapted until that read exists, and a desktop with no adapter is an error rather than a
blind write.

**Why removal resets to the schema default and not to the prior value.** Every other statement's
removal means "Shall stops asserting this", and for a setting the honest meaning of that is "the
desktop's own default applies again", not "whatever this machine happened to hold before Shall
first ran". Restoring a prior value would demand a per-machine store of pre-Shall state — the
exact hand-maintained per-box state II.1 forbids — to serve a case (a key customised by hand
*before* adoption) that is rare and that `gsettings reset` handles acceptably by returning it to
a known value rather than a remembered one.

**V.51 — Why `vars` values are typed and never coerce.** *(Adopted 2026-07-20 from Part IX,
W2; owner ruling.)* The bug is a comparison that answers a question the reader did not ask. Once
a provider can return JSON, `gpu` is `true` the boolean and `ver` is `"1.6.0"` the string, and
those types are information the user produced on purpose. Flattening everything to text at the
boundary throws that away and then quietly lies: `"1" == 1` becomes true, a version string
sorts by ASCII, and `when $gpu` fires on the string `"false"`. So the type is kept, and each
place two types could meet is decided rather than left to chance — no cross-type equality
(`"1" == 1` is false), ordering only between numbers (`"10" > "9"` refused, not answered
wrongly), and no truthiness (a bare `when $flag` is a parse error, so `false`/`""`/`0`/`[]`
never blur together). The one deviation, string equality being case-insensitive, is not a
coercion — it is the behaviour a detected fact has always had (`os == LINUX`) and the place
case matters least.

**V.52 — Why a variable carries a `$` and a fact does not.** *(Adopted 2026-07-20 from Part IX,
W4/IX.4.)* This is a future-fact collision, the quiet delayed kind this document has recorded
too many times. Without the sigil, `when role == travel` and `when os == linux` are one syntax
over two namespaces, and the day Shall learns to detect `distro` or `init`, every file that
named a variable `distro` silently changes meaning. **A detected-fact namespace that can never
grow is a worse cost than one character.** With the sigil, facts can be added forever and no
user file is touched, and a reader can tell at a glance which half of the condition they
decided and which half the machine reported.

**V.53 — Why a provider is chosen by filename and ambiguity is refused.** *(Adopted 2026-07-20
from Part IX, IX.6; owner ruling.)* Two bugs, one entry. First, a **silent precedence guess**:
if `vars` and `vars.py` both sit in a repo and Shall picks one by directory order or a built-in
ranking, the resolved state of the machine depends on something nobody wrote down, and the day
someone adds the second file the machine changes with no edit to explain it. So two providers
and no `[vars] source` is a loud error listing them (P3), never a winner. Second, the filename
*is* the kind — `vars.py` is visibly a program — so what a file does is legible in the repo
rather than hidden behind a config key that could disagree with the file's contents. The
embedded provider gets the full standard library (clock, shell, files, env, network) for the
same reason a hook does: it is a script committed to your own repo, so withholding powers an
external `vars.py` already has would only push people to the external one and inherit its
interpreter dependency across the fleet.

**V.54 — Why a plan freezes its resolved variables.** *(Adopted 2026-07-20 from Part IX,
W4/W13; owner ruling.)* This is the admission price for letting a value come from the clock or
the network, and without it `plan` is a lie. A value that can move between two commands means
the preview you read and the action you confirm resolve `$x` independently and can disagree — the
preview shows nothing to do, and the sync a few seconds later removes packages it never
displayed. That is not a bug to fix later; it is what "the value moved" means. So a variable is
resolved **exactly once per invocation**, and the saved plan carries the values it resolved; the
`apply` that runs a plan reuses them rather than re-running a provider. The preview and the
action agree by construction, which is the property II.8 rests on and the only condition under
which admitting the clock is safe at all. It also means a `vars` edit reaches the guard like any
other change — the desired state is computed from the frozen variables, so a one-line edit that
would remove a hundred packages is caught by `max_removals` before anything runs.

**V.55 — Why a `vars` provider goes through the hook ledger.** *(Found by audit 2026-07-22;
owner ruling the same day.)* II.6b handed `vars.shall` the shell, the filesystem, the
environment and the network on the stated grounds that it "is trusted the same as a hook" — and
that trust boundary was a sentence, not a mechanism. No hash was recorded and nothing ever
asked. II.12's rule is *"hash everything, including your own scripts. One rule, no exceptions"*,
and the provider files were the exception, which is the shape of every V entry here: the
document described a protection the code did not have, so reading the document could not find
the hole.

Three things made it the worst place in the tree to leave one. **Variables resolve at step 0 of
II.7**, before any `when` and before the plan, so the script runs on `status`, `plan` and even
`plan --dry-run` — the commands someone runs *precisely* to avoid acting, and the ones whose
whole promise is that nothing happened. **`watch --pull` pulls a config repo and reconciles
unattended**, so a provider file pushed to that repo executes on the next tick with nobody
present — and it runs before `verify_all_approved`, so the hook ledger that would have caught an
equivalent hook never gets the chance. And **the hole was two holes**: the embedded provider and
the external `vars.py`/`vars.js` path had it identically, so closing one would have moved the
problem rather than fixed it.

Ruled to match hooks exactly rather than strip the standard library. Removing `sh` and
`http_get` would not buy safety — it would push people to the external provider, which has the
same exposure plus an interpreter dependency across the fleet — and it would break the feature's
reason to exist, since detecting what a machine *is* needs to ask the machine. The rule is
therefore the ledger, applied to both providers in the same change.

**V.56 — Why a removal is always a list of names, and why `remove` is not `purge`.** *(Found by
audit 2026-07-22; owner ruling the same day, taking the one-line change the 2026-07-19 entry in
Phase 5 had already offered.)*

`remove-orphans` had two branches. The enumerated one is correct and was built that way
deliberately — list, show, guard the total, remove exactly those names. The second ran the
manager's own verb, `apt autoremove -y`, for backends that could not enumerate. That was a
recorded judgement call rather than an oversight: deleting a working capability looked like
feature removal nobody had approved, and the code was honest about it, printing that those
removals could not be previewed or checked against the protected list.

The honesty is where it broke. That sentence is printed by the **confirmation**, and the
confirmation returns yes under `--yes` — so on the path where a human would have read the
warning, the warning is the thing that gets skipped, and `remove-orphans -y` became unguarded
root-level mass removal on the single most common backend there is. `apt autoremove` routinely
takes old kernels. II.10's own text says `--yes` never overrides the guard, and here it did not
need to override anything: there was no list, so there was nothing for the guard to judge. **A
protection that only exists inside a prompt is not a protection**, which is the general lesson —
the same shape as a check that cannot fail.

The rule is therefore about the *verb*, not the flag: a manager's bulk-removal verb chooses its
own set at execution time, after the guard has judged and after the plan was read, so no amount
of confirming can make it safe. Where the set can be fetched instead — `--dry-run`,
`--assumeno` — it becomes an ordinary enumerated removal and the whole problem dissolves; where
it cannot, the backend loses the capability and says so. That is V.7c's shape again: a manager
that cannot answer gets asked differently or is recorded as silent, never guessed at.

**And `purge` is the same mistake one layer down.** apt's remove arguments were
`["purge", "-y"]`, so *ordinary drift removal* — deleting a line from a module — destroyed the
package's `/etc` configuration. Nobody asked for that and no message said it happened. Deleting
a line means "stop installing this"; it is a statement about what should be installed, and
`/etc/nginx` is not that. Purge stays available because wanting it is legitimate, but it is
opt-in, and — because a removal happens *after* its line is gone, leaving nothing to carry a
per-package option — the machine-wide setting is the only form drift removal can have, which is
exactly why it must default to off.

**V.57 — Why a harness must fail, and must run somewhere other than one laptop.** *(Found by
audit 2026-07-22; owner ruling the same day. The rules are in IV.1 and IV.2.)*

Session 9 fixed a check that could not fail — `command -v` answering from the shell's hash table,
so a package deleted in section 4 still "existed" in section 9 — and recorded that *"a check that
cannot fail is worse than no check."* The audit found three more still live, one of them the
direct twin of a fixed one: the Windows script greps `shall` against `git log` where the
container greps `shall:`, and the config directory is named `shall-it-win-config`, so it matches
on every run forever. Another asserts that the build artifact is still on disk and calls it
*"the shall binary survives an uninstall attempt."* One fixed, siblings live, in the sibling file — the
exact pattern `CLAUDE.md` names.

The larger version of the same fault is an image that claims coverage it does not have.
`Dockerfile.tools` says the harness runs a real install→list→remove for composer, opam,
luarocks, nimble, spack, pixi, helm and krew; none of those names appears in the harness. The
README describes a coverage audit that hard-fails on an untouched backend; no such code exists.
`run.sh` maps `tools → apt`, so the image is `ubuntu` with a forty-minute build — which is
exactly why ubuntu, arch and tools all report the same 82. Every expansion backend was therefore
proven only against mocked output, and mocked output is the one thing that never drifts, while
output-format drift is where every real bug in Part VII came from.

`FAST` is the mechanism-level version: declared in `run.sh`, two Dockerfiles and both
release-check scripts, read nowhere. It is `SMOKE_ONLY`'s bug, left live in the same file during
the session that fixed `SMOKE_ONLY` and wrote three paragraphs about it. A toggle that is
documented and unread does not make a run narrower — it makes a run that *looks* narrower
identical to one that is not, which is the vacuous check again, one level up.

And none of it ran anywhere but one machine. There is no Docker job in CI, no call to
`release-check`, and the branch carrying all of this sat 219 commits ahead of the remote, so CI
had never executed against the rewrite at all. **A gate that depends on someone remembering to
run it is not a gate**, and the evidence is that the three faults above survived a session whose
entire subject was the harness. The fast images belong in CI for that reason and the slow ones do
not: a forty-minute required check is a check people route around, and a routed-around gate fails
the same way a vacuous one does.

**V.58 — Why the version went down, and why a rename sweeps the scripts.** *(Found by audit
2026-07-22; owner ruling the same day. The rule is in II.18.)*

`Cargo.toml` said `6.0.0`. The CHANGELOG called the same tree *"v7, the declarative rewrite"*
and filed it under `[Unreleased]`. Both cannot be true, and the one that reaches a user is
`shall --version`, which was answering `6.0.0` — a number describing the model this rewrite
exists to delete. Nothing has ever been released: the branch sat 219 commits ahead of the
remote, no tag was ever pushed, and the tag-triggered release job in CI has never fired. So the
number was not a version, it was a counter of internal rewrites, and it was **counting up while
the thing it named was being thrown away**. `0.1.0` is what it means to have shipped nothing
yet, and going down is the only honest direction from a number nobody was ever given. The
rewrite keeps its name — "v7" is what Part VII and the CHANGELOG call this work — because a
codename and a version answer different questions.

The install scripts are the same fault at the other end. Both fetched from
`github.com/OWNER/shall`, a placeholder that was never substituted, and both finished by running
`shall migrate` — a command **II.17 has listed as deleted since the rename to `adopt`**. So the
one documented path a new user takes installed the binary and then failed on the step that takes
over the machine, and the spec already contained the sentence that predicted it. `src/` was
swept, `scripts/` was not: the family rule, on the layer furthest from the code and therefore
the one nothing in the build ever compiles, lints or tests. Which is the point — **the install
path has no compiler**, so it needs the harness to run it, or it needs a human to notice, and
neither had.

**V.59 — Why `restore` is a command and not a README.** *(K9 answered 2026-07-22, owner ruling,
after it had been deliberately left open since 2026-07-19. The rule is in II.8; the requirement
it satisfies is X.5's.)*

`bundle` was built as half a feature and read as a whole one. It packs the config root, `locks/`,
the resolved package list, the full manifest history as `config.bundle`, and optionally the
artifacts — and then writes `RESTORE.md`, a file telling a person which directories to copy
where. So the restore path was prose. Nothing in the tree had ever performed one; the only test
asserts that a tar archive round-trips, which proves the archiver works and says nothing about
whether what comes out is a machine.

**That is the vacuous-check family again (V.57), one layer out.** A test that cannot fail is a
check with no teeth; a restore that is documentation cannot even be a check, because there is
nothing to run. And the thing it is supposedly protecting is the case where everything else is
already gone — the one moment when finding out that a step was mis-described is most expensive
and least recoverable. **A backup nobody has ever restored is not a backup, it is an intention.**

It matters more than a spare feature because of X.5. A git-less machine is a supported machine —
session 9 spent the gentoo image proving that history *refuses honestly* there rather than
lying — and git is what provides history, rollback and `diff`. Take git away and `bundle` is the
only mechanism that carries a config off a machine at all. So the git-less case, which the
document says is supported, rested entirely on the half of `bundle` that did not exist.

K9 asked whether the backup command is `bundle`, an alias, or nothing, and fenced the answer with
one constraint: **not a second archive writer.** That constraint decides it. There is no room for
a new backup feature beside a bundler that already writes everything a backup needs; the only
move left is to finish the one that exists. Hence `restore DIR`, and hence its refusal to write
into a non-empty config directory — the machine you reach for a backup on usually still has
something on it, and a restore that silently overwrites the work that made you want a backup has
chosen the wrong default.

**V.60 — Why a snapshot provider must be able to refuse a restore.** *(Found by audit
2026-07-22; owner directed the fix.)* `SnapshotProvider::restore` ran
`btrfs subvolume snapshot <snap> /` for btrfs. That command does not roll a mounted root back to
a snapshot — with an existing destination it **creates a new nested subvolume** and exits **0**.
A live btrfs root rollback means moving the current subvolume aside and setting the default
subvolid, which cannot be done over the running `/` at all. So the status check passed, the
caller took that as success, and the machine was reported restored while nothing had been
restored.

Every recovery path in the binary consumed it. `rebuild` printed *"Rolled back to snapshot X —
the machine is as it was before the rebuild started"* over a machine whose packages were still
removed; `upgrade --canary` printed *"System left unchanged"*; `bisect` relied on it between
steps. Worst is `purge-undeclared`, which prints *"Snapshot taken: X. That is your undo"* — the
command that removes everything unmanaged, offering an undo that does not exist, in the one
message II.11 calls the most important sentence it can print.

Two rules come out of it. **Taking and restoring are separate capabilities**, so a provider that
can do the first and not the second must say so where it can still be acted on — in `doctor`, and
before the change, not after it fails. And **a claim about the machine is never inferred from an
exit code**: "rolled back" is the one sentence a user cannot verify at the moment they read it,
which is exactly why the code has to. There was also a second implementation in `undo.rs`
carrying the identical bug and printing *"SUCCESS: System root has been restored."* — and
handling only btrfs and Timeshift, so ZFS and Windows silently restored nothing at all, while
the provider it duplicated implements both. **One restore, not two** (P-prefer-deleting): the
weaker copy is the one wired to `undo`.

When providers became declarable (U27, 2026-07-26), this rule set the one field that stays the
author's to state rather than data to infer: **a declared provider must say whether it can restore
a running machine.** Everything else about a provider — the commands, the filesystem — is
observable, but live-restore capability is the thing whose wrong guess is a machine reported safe
that is not, so it is a required field with no default. Omitting it is a loud refusal, not an
assumption in either direction: the design would rather decline a rollback it could have done than
promise one it cannot.

**V.61 — Why the data directory takes a lock.** *(Found by audit 2026-07-22; owner directed the
fix.)* `registry.json` was loaded once per process into a `tokio::Mutex` — which coordinates
tasks inside one process and nothing between processes — and written back whole, with no re-read
and no compare-and-swap. `fs2` was in the dependency list and used at exactly one site, around a
single subprocess, never around state.

That would be a latent race in most tools. Here it is a live one, because **Shall installs
package-manager hooks**: `DPkg::Post-Invoke` and its dnf/zypper/apk/xbps/portage siblings spawn
`shall hook-reconcile` on every ordinary `apt install`. So the second writer is not another
Shall the user ran — it is `apt`, run by someone who does not know Shall is involved, at a moment
nobody chose, possibly during a `sync` or between two ticks of a `watch` loop that never reloads
state. Two whole-file writes are last-one-wins, and **the entry that loses is not lost data, it
is a removal**: a package installed and managed, missing from the registry, is a managed package
nothing declares — which is drift, and converging drift is what `sync` does.

The lock is on the data directory rather than the file because the registry is not the only
thing a run writes; the journal and the `locks/` ledgers move with it, and a lock that covers one
of a set that must agree is the same as no lock. It is taken for the whole run and names its
holder when it is contended, because "waiting" with no reason given is indistinguishable from
hanging.

**V.62 — Why a name is terminated, and why an uncalled check is deleted.** *(Found by audit
2026-07-22; owner directed the fix. The rule is in II.12b.)*

The pass-5 security review concluded that the core was sound because *"every package-manager
command is built as argv (no `sh -c`, no `format!`-into-shell)"*. That is true and it is not
enough. Argv stops a **shell** from reinterpreting a name; it does nothing to stop the **manager**
from doing so. The grammar constrains a package name to "one word", and a leading `-` is caught
only in the `Subtract` position at the start of a line — so `apt:--allow-downgrades` parses as an
ordinary package, and no backend emits a `--` terminator before its names. `generic.rs` install
and remove, `brew`, `snap`, `flatpak`, `nix`, `conda`, `krew`, `mise`, `setting`, `service`,
`vscode` — around thirty call sites, roughly half of them running under sudo. `conda` extends
the reach to a value read out of `preferences.toml`.

**The fix already exists in the tree and was applied once.** `fleet.rs` rejects a leading dash
and emits `-- `; nothing else does. That is the family rule in its plainest form: the correct
version was written, and its thirty siblings were never visited. Terminating is the rule rather
than name-filtering because the flag set belongs to the manager, not to us — a denylist of
dangerous options is a promise to track every manager's option parser forever, and `--` is a
promise the managers already keep.

**Terminating is a promise the managers keep, and four of them do not.** `--` is not universal:
`asdf` dispatches on `$1` and answers `No such plugin: --`; `spack` reads it into the spec;
RubyGems' `--` separates gem names from C-extension build arguments, so `gem install -- colorize`
names no gem at all; `nimble`'s reaches the Nim compiler and breaks every build that produces a
binary. All four were listed as terminating by someone who recognised the family, and each one
broke every install that went through it. Hence the default in `core/argv.rs` is **does not
terminate**, and a binary joins the terminating set when someone has *run* it.

**And "someone has run it" is now a field, not a memory** (2026-08-04). The table was two lists —
one of them `#[cfg(test)]`, so half the production facts compiled only into tests — with a test
whose whole job was to catch them contradicting each other. It is one list, each row carrying
either the tool's own sentence or an admission that nobody asked, and the admissions are counted
by a ratchet that may fall and never rise. `tests/terminator_probe_tests.rs` is what lowers it:
it runs each manager's real argv twice, once with the terminator and once without, and believes
the tool honours `--` only when the two runs agree on exit code, on whether the operand was
echoed back, and on there being no bare `--` anywhere in the output. Differential, so it never
has to understand any tool's error prose — and it reads the argvs out of the registry, because a
hand-written table of "the verb to probe each manager with" would be the second copy of the truth
that this rule is about.

**The mirror-image bug: a name that is safe until someone pins a version.** `VersionPin` had
three variants — `Flag`, `TrailingPositional`, `RequiredFlag` — with character-for-character the
same body. They built identical argv; only the *label* decided whether the terminator survived,
because a version spelled `-v 1.6` is an option and one spelled `1.6` is an operand. Three
backends carry a bare operand version — `luarocks`, `mix`, `pub` — and they were spread across
two labels, so `luarocks install -- jq` kept the terminator and `luarocks install jq 1.6` dropped
it. Same tool, same command, protection that came and went with whether the line named a version.
The variants now say only *where* the version goes, and whether it is an option is read off the
token, because an option starts with `-` and a version does not. A fact the data already states
cannot be restated by hand without eventually disagreeing with itself — which is V.62's own shape
one layer in, and the same lesson as the two tables above.

**The same audit found the mirror image**: `Validator::validate_command` and `validate_path` —
carrying the `rm -rf /` / `mkfs` / fork-bomb denylist, a trusted-binary-path list, and a
forbidden-path list including `/etc/shadow` and the SAM hive — have **zero callers** outside
their own tests. The tests pass. The module reads as a security layer to anyone grepping for
one, and enforces nothing at runtime; `validate_package_name_for` *is* called, but only on
desired-state specs, not on removal targets, CLI arguments, or link and hook inputs.
`FORBIDDEN_PATHS` is additionally duplicated in `undo.rs`.

These are one bug wearing two faces: **a protection that is written but not on the path.** A
missing check is visible — someone looks for it and it is not there. An unwired one answers the
search and fails the job, which is the vacuous-check family (V.57) at the level of the source
rather than the harness. So the rule is symmetric: every check is called where it claims to
apply, or it is deleted, and the choice between wiring and deleting is made deliberately per
check rather than left to whoever greps next.

**V.63 — Why `sync` is additive and `purge-undeclared` is exclusive, for every backend.**
*(Owner ruling 2026-07-23, N1; the rule was always true and had never been written.)*

The firewall proposal asked whether a declared perimeter is exclusive — whether a rule Shall
never declared counts as drift. It is a reasonable question and it should not have been askable:
**the model answered it years of decisions ago, for every backend at once, and nobody had put
the sentence anywhere a reader could find it.**

The split is what makes Shall safe to point at a machine that already has software on it. `sync`
only ever removes what the ledger says Shall put there, which is why running it on an unadopted
box does not empty it. `purge-undeclared` removes what Shall did not declare, which is why it
carries a ratio guard, a full listing and a snapshot — it is the one command whose whole job is
acting on things Shall does not own.

**The bug this prevents is a second `purge-undeclared` per backend.** A backend that ships its own
exclusive mode has re-implemented that command with none of its protections: no ratio check
noticing you have not adopted the machine, no listing, no snapshot, and a different opt-in for
the user to learn. It would also make the answer to *"will this delete something I made by
hand?"* depend on which backend the line happened to name — which is the two-of-everything
failure at the level of a promise rather than a function.

So: **a backend does not decide its own exclusivity.** If a new backend seems to need an
exclusive mode, the thing it needs is `purge-undeclared` to learn about its resources.

**V.64 — Why a recovery path may not remove.** *(Owner ruling 2026-07-23, S24; the bug removed
software on the owner's machine.)*

`heal()` recovered an interrupted install by uninstalling the package and reinstalling it. The
package was declared, wanted, present and protected, and the command that triggered it was
`install nimble:nimjson`. It reached no guard, was counted nowhere, appeared in no plan, left no
history entry, and `--dry-run` performed it.

**The obvious fix is to send that removal through the guard, and it is the wrong one.** II.10
claimed "every removal path calls it" for thirteen sessions, through an audit whose entire
purpose was finding false claims, while this path called nothing. Adding the call would leave a
delete sitting on the path nobody watches, protected by a check whose absence nobody noticed for
months. **A guard is a good defence against a removal you know about. It is no defence at all
against one nobody remembers is there.**

So the rule is about the path, not the check: **anything that repairs, retries, rolls back or
completes an interrupted operation reinstates what was wanted, and does not delete to get
there.** These paths need it more than ordinary ones, not less, precisely because they run
outside the plan the user read and usually when nobody is watching.

**Where a manager genuinely cannot recover without removing first, that is a capability it
declares** — and then the removal is an ordinary removal, with the guard, the count, the plan
line and a real error on failure. The point is not that a recovery may never delete. It is that
**a deletion is never a hidden step inside something else.**

---

**V.65 — Why a health check that cannot revert is refused, rather than run.** *(Owner ruling
2026-07-24, U7.)*

A health check exists to answer one question: *did this change break the machine?* The answer
is only worth having because of what follows it — going back. A check that runs on a machine
with no snapshot provider still answers the question, and then does nothing: it reports that
the machine is broken and leaves it broken.

That is **strictly worse than not checking at all.** Not checking leaves you with a machine in
an unknown state. Checking-without-reverting leaves you with a machine in a known-bad state,
having spent the one moment when the situation was still recoverable on producing a message.

So the absence of a revert path is decided **before the first package is touched**, where it is
still actionable, and not afterwards, where it is only a description of the damage. The refusal
names the checks and the missing provider, because the two fixes — set up snapshots, or drop
the checks — are both the reader's to make and neither is guessable from "health check failed".

**The same argument makes the two scopes one path.** `@health=` on a line and the machine-wide
`health` list answer different questions, but a broken nginx and a broken boot mean the same
thing to the machine: go back. Giving them separate revert paths would mean maintaining two
answers to a question that has one.

**V.66 — Why `exec:` is a verb, and why a false `when` is not an undo.** *(XIII.3; U3 ruled
2026-07-24.)*

Every other statement in II.2 is a noun: it names a thing that should exist, so the machine can
be compared against it and the difference removed. `exec:` names an *action*, and an action has
no state to compare against — which is why it was nearly not built at all, and why it is the one
place this model bends.

**The bug it would cause if it were treated like a noun is flapping.** A script that succeeds
usually makes its own condition false: `exec:./enable-thing.sh` guarded by `when` the thing is
not enabled. Under the ordinary rule — false `when` means undeclared means remove — the script
would run, succeed, become undeclared, and be "removed" on the next sync, which would make the
condition true again. The machine would oscillate forever and every sync would report work done.

So `exec:` is keyed by the **content hash of the script** in `locks/exec.toml`: what decides
whether it runs is whether *this exact script* has already run, not whether a condition still
holds. `@runs=1` is the default and `@runs=always` opts out, visibly.

**And what removing the line means is the honest answer, not the convenient one.** If the line
carries `@undo=`, that is what runs. If it does not, Shall **drops the record and does nothing
else**, and `plan` says so in those words. The alternative — inventing an inverse for a script
whose author did not write one — is Shall claiming to undo something it cannot, which is the
same class of lie as printing "rolled back" on the strength of an exit code (V.60).

**V.67 — Why a dotfiles tree links files and never directories.** *(U22–U25, ruled 2026-07-24.)*

A dotfiles tree is the one declaration whose *layout is the statement*: `dotfiles:./home` stands
for as many declarations as the folder holds, and adding a file to it is how you declare a new
one. The temptation is to symlink the directory — one link instead of two hundred, and new files
appear for free.

**The bug that closes it is that a symlinked directory is a directory the application writes
into.** Link `~/.config/nvim` and every plugin nvim installs, every lockfile it generates, every
piece of session state it caches, lands **inside the git-tracked repo**. The repo stops being a
declaration and becomes a mirror of runtime state; `shall diff` fills with noise; and `bundle` —
whose whole promise is that the archive is safe to hand to someone — hands over whatever the
application happened to leave there. Per-file links cost more link calls and are the only form
where what Shall manages is what the user wrote down.

**A destination that already holds the user's own file is refused by name, not replaced** (U23).
The tree has no place to write a per-line option, so there is no `@force` for it and there
cannot be one — which is the same structural reason it **never decrypts** (U24): a `.age` file in
the tree is a file, and there is nowhere to say otherwise. A secret that needs decrypting is a
`link:` line, where the option can be written and read.

**V.68 — Why `firewall:` is built in, and why the lockout check comes first.** *(N1–N7, ruled
2026-07-23/24.)*

A firewall looks exactly like something the onboarder should cover: `ufw` is a command with
subcommands, and XIII.2 exists so a user can add a manager Shall never heard of. **It is not,
and the reason is the one thing a `[[backend]]` naming `ufw` could never give:
`firewall:22/tcp` must mean the same thing on the Debian laptop and the Windows workstation.**
A per-machine adapter definition makes the *declaration* per-machine, and a declaration that
means something different on two machines is not a declaration. So the statement is built in and
the **adapters** are rows (K17) — `ufw`, `firewalld`, `windows-defender` shipped as data, a
fourth added without a release.

**The lockout check is this feature's precondition, not one of its features.** Shall detects the
port carrying the controlling connection and refuses any plan that would deny it — from `sync`,
from `purge-undeclared`, and from an unattended `watch` tick. The tick is the dangerous one:
nobody is there to read the refusal, and the machine that locks you out is the machine you can no
longer reach to fix it. **Building the backend before the check is building the lockout**, which
is why the check sits at the bottom of the module and everything above it is written against it.

**V.69 — Why `@scope` exists on exactly three statements, and why writing the default is
allowed.** *(U19, ruled 2026-07-24.)*

Shall used to act, implicitly, as whoever typed the command. The Linux backends mostly agree
with that by accident. **The Windows registry cannot**: `HKCU` and `HKLM` are a real choice with
no default that is right for both, and picking one silently means a config that reads identically
on two machines configures the account on one and the machine on the other.

So the question is asked where it can vary — `setting:`, `link:`, `shim:` — and **nowhere else**.
A `service:` is the init system's business and a `repo:` is the manager's; putting `@scope` on
them would be a key that means nothing, and a key that means nothing is a key someone writes and
Shall silently ignores, which II.2 closes with in exactly those words.

**Writing the default is not an error.** `@scope=user` on a store whose default is already user
is accepted and means what it says. A configuration is allowed to state a thing it also gets for
free: saying it out loud is how the next reader learns the answer without going and looking it
up, and refusing it would punish the person being explicit — which is the opposite of what a
declarative system should reward.

**V.70 — Why a `link:` backup is restored rather than retained, and why the opt-out is per
line.** *(T6, ruled 2026-07-23 and closed 2026-07-26.)*

`backup_once` exists for one reason: a user should not be silently robbed of a config file they
hand-wrote because a `link:` line replaced it. The question that hung over it for weeks was how
many such backups may accumulate, and whether there should be a retention key or a command to
list the orphans.

**Restoring on teardown dissolves the question instead of answering it.** Removing the `link:`
declaration puts the original file back and deletes the backup — so a backup exists only while
the thing that displaced it exists, and a pile cannot form. A retention policy would have been
machinery for a problem created by not having this rule.

**The opt-out is per line because a machine-wide one would travel.** `preferences.toml` is copied
between machines and pasted from the internet like every other config; a key that turned backups
off everywhere would arrive that way, and the file it silently stopped preserving would be one
somebody hand-wrote. Stating the exception on the line that wants it puts the decision next to
the file it is about. *(The fix that implemented this found a worse defect in the same three
lines: the teardown was handed the declaration's **source**, so undoing a `link:` deleted the
file in the user's own dotfiles repo and left the deployed copy standing. A link is keyed by its
**destination** now, which also means editing `@target=` undoes the old destination instead of
orphaning it forever.)*

**V.71 — Why ten looking commands became one, and why `heal` survived.** *(U9, ruled 2026-07-24.)*

`status`, `doctor`, `unmanaged`, `absent`, `conflicts`, `insight`, `metrics` and `audit` were
eight answers to one question — *how is this machine doing?* — each with its own output shape,
its own flags and its own idea of what counts as a problem. **The failure is not that there were
too many; it is that the correct answer to "which one do I run" was "several, and compare".** A
user who ran `status` and got a clean result had learned nothing about drift, conflicts or
approvals, and nothing told them so.

One command with named sections makes the set visible: `check` prints a line per section, so the
questions you did not think to ask are on the screen next to the one you did. Naming a section
prints its detail. The old names are **deleted, not aliased** — an alias would leave the ninth
way to ask the question standing, and this repo's whole disease is two ways to do one thing.

**`heal` survived the collapse and `doctor --fix` did not, and that is the dividing line the
collapse rests on: `check` looks, `heal` acts.** A repair verb hidden behind a flag on a status
command is a mutation reachable from something a user believes is read-only — which is the same
shape as `--dry-run` performing a removal (S25). The line is drawn at the verb, not at the flag,
because a flag is one keystroke from a routine command.

**V.72 — Why `shall lock` approves the whole `adapters/` folder, not a named list.** The
approval step listed three files by name — backends, settings, bootstrap — and every adapter
file not on that list was unapprovable, so its rows were refused on every sync while the file sat
in the repo doing nothing. `firewall.toml` had been in exactly this state: a live guard-on-one-
command-is-a-guard-on-nothing, in the folder whose entire job is to gate argv a shared repo can
run. A hardcoded list is the same "a list is an assertion about what is absent" trap II.10's
paragraph warns about (S24): it is checked by reading the three it names and always passes,
because the file it forgets is never on it. Reading the directory means the assertion is made
against the code, and a new adapter kind is approvable the day it lands with no second place to
edit. The approval predicate itself is now one shared function (`hook_lock::adapter_refusal`), so
the onboarder and the snapshot loader cannot come to disagree about what an approved file is —
two copies of an approval rule is how one path starts trusting a file the other refuses.

**V.73 — Why init systems are rows and the `enum InitSystem` is gone.** A closed enum behind a
hardcoded command match covered systemd/OpenRC/SysVinit/launchd/`sc` and gave every other init
*no branch to take* — a `service:` line on an s6 or dinit box did nothing and said nothing, the
P3 silent-wrong failure. It is the snapshot vec's problem (V.60's neighbourhood) in a different
file: interchangeable "run these commands" providers frozen into Rust. The shipped five are now
rows in `init_providers.toml` going through the loader a user's `adapters/init.toml` row goes
through, because an adapter mechanism the built-ins bypass is one nobody has tested. A row that
cannot both start and stop is refused rather than half-loaded: a provider that starts a service
it cannot stop is a teardown that silently leaves it running. systemd's `--` terminator is kept
in the row data (the unit is a trailing positional); the other inits put the name between
positionals, where a `--` would be read as the service name — the tested argv behaviour, now
expressed as data rather than a match arm.

**V.74 — Why a config-driven snapshot provider is create-only unless it says otherwise.** This
is V.60 restated for data: `restore` that exits 0 and rolls nothing back (`btrfs subvolume
snapshot SRC /`) is the bug the whole `RestoreCapability` split exists to prevent, and a
config-declared provider is a new mouth for it. So `restores_running_system` defaults to `false`
and, even when true, a provider with no `restore` command is still create-only — the capability
must be *named in the file and backed by a command*, and naming it is the line a reviewer sees in
the diff. The unsafe reading is never the default: a row that omits the field can snapshot and
can refuse a rollback; it can never run a "restore" and hope. A provider registers LAST and never
shadows a built-in (the `custom_backends.toml` rule applied to the safety layer), so a stray file
cannot replace the tested btrfs/zfs/timeshift path with an untested one.

**V.75 — Why the active snapshot provider is chosen by a declared priority, not by capability.**
A machine can have more than one provider available (btrfs *and* a config-declared lvm), and
which one is the safety net must be the user's decision, stated, not Shall's guess. Choosing "the
one that claims live restore" would let a newly-added, less-trusted provider silently displace a
proven one the moment it declared a capability; choosing by registration order would make the
answer depend on an implementation detail nobody wrote down. `snapshot_priority` is the
`priority`-file shape reused (V.15's reasoning): the first *available* provider in the declared
list wins, an empty list keeps the historical registration order, and a name that matches nothing
present falls back rather than leaving the machine with no net it could have had.

**V.76 — Why APFS is declared create-only.** macOS ships APFS on every machine and `tmutil
localsnapshot` takes one with no configuration, so the second platform Shall supports finally has
a safety net — but an APFS *restore* needs a reboot into the recovery environment, which Shall
cannot drive on a running system. Claiming `Live` would be V.60 exactly: an undo offered where it
cannot be kept. So APFS snapshots and refuses the rollback with the manual steps. And because
`tmutil` does not record which snapshots Shall made, retention never reaps an APFS snapshot
(`is_shall_owned` is false for them) — the safe direction: Shall never deletes a restore point it
cannot prove it created (S3).

**V.77 — Why a user verb may only compose built-in verbs.** A `[verbs]` entry is `defun` over
the command surface — `refresh = sync, then upgrade` — and it is safe precisely because it
sequences operations Shall has already audited, producing nothing the guard, the plan and the
ledger did not already see. The moment a verb can run arbitrary argv it is `exec:` wearing a
command's clothes (U4's settled question), so a step that names anything but a built-in is refused
and pointed at U33's off-by-default key. A verb also takes no arguments of its own: threading
`shall refresh --dry-run` into some steps and not others is the surprise a closed vocabulary
exists to avoid, and a verb never shadows a built-in, so a shorthand can never mask a real
command. `shall repl` sits under the same principle from the read side (the U20 rule): it is a
thin front end over the one parser and resolver, never a second implementation, because this
repo's history is that a second implementation of anything eventually disagrees with the first.

**V.78 — Why a missing module parameter is a loud error, not an empty string.** `param` (U32)
gives a module arguments, and the substitution reuses the existing `$name` machinery one scope
wider — the params bind first, `when` and every value see them, and an unknown `$ref` is left for
the global `vars` pass rather than errored, so the two scopes compose. The one rule that is not
negotiable is the failure mode: a `param` with no default that a `use` omits is an error naming
the module and the parameter, never a silent empty string. An empty string would make
`when $gpu == nvidia` quietly false and `link:@target=/home/$user/…` write to `/home//…` — the
P3 silent-wrong failure the `vars` work was hardened against (IX.3), arriving through a new door.
An argument that names no parameter is likewise an error, not a no-op: a closed vocabulary names
its typos (VIII.2), and binding `gpu=nvidia` to a module with no `gpu` param would drop the
intent without a word. Substitution reaches exactly the fields the global `vars` pass reaches
(V.62), one shared helper, so the two cannot come to disagree about where a `$ref` is a value.
The expansion is ordinary declarations, visible in `shall eval` and the removal preview before
anything runs — a macro that could produce an action you cannot see is the one thing U32 must not
be, which is why generated declarations are U33's separate, off-by-default question.

**V.79 — Why `generate:` is off by default, and how it stays on the safe side of the line.**
`generate:` (U33) runs a command and treats its stdout as declarations — the one surface where
the config *computes* its state instead of stating it, which is the property XIII.32 says openness
must not cross. The owner ruled it in anyway, so the whole weight falls on four rules, none
waived: (1) **off by default** — `allow_generators` unset makes a `generate:` line a refusal
naming the key, so the computing-config surface is dormant unless deliberately turned on; (2) **the
ledger gates it** — it is approved by `shall lock` content-addressed like `exec:`, and an
unapproved or changed command stops resolution, `-y` cannot approve; (3) **a failure is a failed
resolution, never an empty set** — a non-zero exit is an error, because "the generator broke" read
as "nothing is declared" is a mass-removal input, VI.0's whole family; (4) **the output is shown,
not trusted** — it is spliced into the statement stream *before* bare-name probing and collection,
so a generated line passes the same conflict check, guard and removal preview as a typed one, and
a generated `apt:foo` reconciles with a typed one rather than doubling it. The approval is scanned
from the files, and scanned *first* in `shall lock`, because resolving the model now runs
generators — a generator cannot be approved by a command that must resolve past it to find it.
**The exec half of U33 is the U4 amendment, and it is a documentation change, not a new gate:**
`exec:` already runs arbitrary code, and its gate already exists — the II.12 ledger, which
approves each script individually, so nothing runs unreviewed. U33 lifts only the *guidance* that
`exec:` is "not for installing software"; adding a second, blanket config gate on top of the
per-script ledger would break every existing `exec:` line for no safety the ledger does not
already provide. The ledger is exec's config key, per-script and already off until you approve.

**V.80 — Why storage objects are ordinary backends, and their `remove` gets the normal guard.**
A ZFS dataset and an LVM volume (U30) join btrfs as a declared, sized, mounted storage object —
one family, because they are the same idea, and Rust rather than a `ManagerConfig` because a
volume has a size and a mountpoint, not a version. The edge that decided the shape is the
`remove` path: `zfs destroy` and `lvremove` erase a filesystem and everything on it. The ruling
is that they go through the **normal** sync guard — no special escalation — and the reason is
that "normal" is already the strongest thing there is: because they are backends, deleting a
`zfs:tank/data` line makes it drift, drift makes it a removal, and the removal runs through the
same guard as any package — so a volume is protectable (`[guard] protected_packages` matches
`zfs:tank/data`), it counts against `max_removals`, and the destruction is previewed before the
guard clears it. A storage backend that ran its own removal outside the guard would be the
teleport bug (the 2026-07-17 lesson) with a filesystem on the end of it — which is exactly why
being an ordinary backend, not a special one, is the safe answer.

**V.81 — Why a declared secret provider must promise stdout-only, or it is refused.** U38 opens
decryption to any command that turns a reference into plaintext (sops, Vault, 1Password, a KMS,
GPG) — the same "rows, not Rust" move, on the one surface where the output *is* a secret. So it
carries the strictest version of the capability-must-be-declared rule: a provider block that does
not say `stdout_only = true` does not load, because Shall will not hand a secret to a command
that has not promised to keep it off disk and out of the logs. The promise is what lets the
provider plug into the *existing* decrypt path, where the T-series rules already live — the
plaintext is captured from stdout in memory, the destination is restricted before it is written
(T5), never backed up (T1), never allowed into the git-tracked repo (T2), and the run is bounded
by the touch timeout (T3). A provider gets all of that for free precisely because it promised
stdout-only; a provider that writes its own file would bypass every one of those, which is why
the unsafe reading is not merely discouraged but refused at load. `age` and `sops` stay built in
(age carries the hardware-token handling W-series and T-series argued for); the door U38 opens is
the one the mechanism already made trivial, and it opened last, after the T-series settled.

---

**V.82 — Why the built-in Windows snapshot provider is a row of typed placeholders, not a string.**
*(Owner decision session 2026-07-26; resolves the SEC5 tension U27 left open.)* U27 ruled that the
built-in snapshot providers stop being a hardcoded `Vec` and become rows through the one loader, so
the mechanism is proven by the providers that ship and cannot fork into a privileged path nobody
tested. btrfs, zfs, timeshift and lvm are plain argv and became rows without incident. Windows
System Restore was the exception that nearly earned a permanent exemption: it is not a program you
exec with argv, it is elevated PowerShell cmdlets (`Checkpoint-Computer -Description '…'`,
`Restore-Computer -RestorePoint {id}`), and SEC5 exists because those cmdlets were once built by
string interpolation — a `'` in a label or a non-numeric id would have run as an elevated shell.
SEC5 closed that by making the id a `u32` and the label a fixed enum, so nothing untyped could
reach the interpolation. **A naive "row" reopens SEC5 exactly:** a free-text template a shared repo
fills is a string with the id spliced back in. The resolution is that the row for a cmdlet provider
carries *typed slots*, not a shell line — the loader substitutes the id only after parsing it as a
`u32` and the label only from the `SnapshotLabel` enum, so the property SEC5 established ("nothing
but a `u32`/enum reaches the PowerShell") holds after the conversion as much as before it. The
owner chose this over a hardcoded exemption because the exemption would mean the K17/U1 invariant —
every built-in goes through the tested door — is only *almost* true, and an "almost" on the safety
layer is the thing that hid the eighth removal path. **The unsafe reading is not merely
discouraged; it is unrepresentable:** there is no field on a snapshot row where a user could type a
PowerShell string with an id in it, because id and label are the only variable parts and both are
typed. And because the whole thing is expressible and testable on a Windows host, this is not a
"trust the design, verify on hardware later" case — it is verified where it runs.

---

**V.83 — Why a declaration names what `list` shows, not what `install` takes (U39).** `helm
plugin install` takes a URL and `helm plugin uninstall` takes the name inside the plugin's own
`plugin.yaml`. Shall declared the URL, because that is the string install needed, and the install
worked — once. Every sync after it asked `helm plugin list` for a package called
`https://github.com/databus23/helm-diff`, was told it was not there, decided that was drift, tried
to remove it by that name, and failed with `Plugin: <url> not found`. **A failed removal is not a
one-command failure: it leaves the same state behind, so it recurs on every sync forever, and every
other backend queued behind it stops too.** One helm plugin wedged the whole model. The rule is
therefore about *which string survives*: install runs once, list and remove run for the life of the
declaration, so the name has to be the one those two answer to, and anything install needs beyond
it is an option. **Deriving the name from the URL was rejected outright** — `helm-diff` → `diff`
is a convention, not a contract, and the version that is wrong installs a plugin under a name
nothing can remove, which is the exact bug with a smaller blast radius and no error message. The
refusal is louder and cheaper: a `helm:` line with no `@url=` never installs anything.

The fix itself then demonstrated the rule it exists to serve. The first version added
`install_source_option` to the backend and tested it by building a `PackageSpec` in code — so
nothing ever asked the **grammar** whether `@url` was a legal key. II.2's option table is closed,
so it was not, and every real `helm:diff@url=…` line came back as a misspelling while the whole
suite passed. It was caught by running an actual `helm` in a container, which is the same way the
original bug was caught, and it is the argument for `capability::INSTALLS_FROM_SOURCE` being **one
table read by both ends** rather than the key being written down twice.

---

**V.84 — Why Shall reads every command's output, and why a child never gets the screen (U40,
S42/S43).** *(Found by the production-readiness review, 2026-07-27; ruled and built the same
day.)* `RawExecutor::execute` asked one question — is *Shall's* stdin a terminal? — and used the
answer to decide all three of the child's handles. When it was, the child inherited stdout, so
`output.stdout` came back empty and all 79 `run_output` call sites parsed an empty string. `shall
list -b apt` reported **609 packages piped and 1 under a terminal, on the same machine, from the
same command.** The failure is worse than a wrong answer because it does not look like one: what
reaches the screen is `dpkg-query`'s own output, which reads like a package list to anyone who
is not comparing formats.

**The rule is therefore about *who* decides, not about which way it goes.** Capture belongs to
the call — a read parses, so a read captures; a mutation may need a password, so a mutation may
share stdin and nothing else. Making it ambient made Shall behave one way for the machines that
test it and another way for the machines that run it, and only the first kind reports back.

**The same inheritance turned a read-only command into a hang.** With stdout inherited,
`systemctl` concluded a human was watching and piped itself into a pager; `shall status` waited
for a keypress and had to be killed, and across three identical runs printed 80, 640 and 83
lines. So the pager suppression is not a second fix for the same bug — capturing removes the
usual trigger, but `$PAGER` and `$SYSTEMD_PAGER` force one regardless, and a forced pager puts
`lines 1-16/16 (END)` and a screenful of escapes into the text a parser is about to read. It is
set on the env map every spawn inherits, because a suppression applied at some call sites is the
`command -v` case again: the sibling that was missed is the one that runs.

**Mirroring exists so the fix does not cost what it fixes.** Inheriting the handles was the wrong
mechanism for a real requirement — a five-minute `apt install` that prints nothing is a tool that
looks wedged. The bytes now go both places, and the mirror is stderr because stdout is where
Shall's own answer goes and interleaving the two makes both unreadable to whoever piped us.

**What this cost, and why the test matters more than the fix.** 1,324 tests, four container
lifecycles and three OS builds were green throughout, and not one of them could have observed
any of it: every gate in the repo runs with pipes on every handle. A green suite was not evidence
against the finding — it was the reason the finding survived. `tests/pty_tests.rs` closes the
gap with `script -qec` and a stub manager on `PATH`, asserting that what Shall printed is what
Shall parsed, and it was watched failing against the old behaviour before it was made to pass.

---

**V.85 — Why a rollback needs to know what was there before (U41, S45).** *(Found by the
production-readiness review, 2026-07-27; ruled and built the same day.)*

`Transaction::rollback` compensated a `GraphAction::Install` by calling `remove()`. That is
correct only if the package was absent before the transaction — and often it is not.
`spec_is_missing` returns true for a **version or channel change on an already-installed
package**, which schedules an `Install` node for software the user already has. One later
failure anywhere in the graph then uninstalls it. **The compensation for a failed upgrade is the
old version, not the absence of the package**, and nothing in the engine could tell the two
apart because nothing recorded which one it was doing.

**The same absence of knowledge is what made it dangerous rather than merely wrong.**
`needs_change` read *"I could not ask the manager"* as *"it is not installed"*. Under the defect
in V.84 that condition was universal: `info()` returned nothing for everything, so every managed
package got an `Install` node, each `apt install <already-present>` succeeded trivially and
landed in the history, and a single failure rolled back across the whole set. **A mass-uninstall
reachable from an ordinary interactive `sync`, built out of two independently reasonable
defaults.** Neither one alone would have done it. That is the argument for the rule rather than
for either patch: a recovery path may only undo what it can prove it did.

**And the guard was not there at all.** `transaction.rs` carried zero references to it.
`guard::enforce` runs at plan time over the planner's `Remove` nodes; rollback's removals are
issued at execution time and passed through nothing, so `protected_packages` and OS-essential
protection did not apply to them — while II.10 said "every removal path calls it". This is S24's
lesson repeating in a new place: *a list is an assertion about what is absent, and nothing
verifies that half.* The enumeration named twelve `GuardScope`s and rollback is not one of them,
because rollback never asked for a scope.

**What happens when the guard refuses is the part that had to be ruled rather than coded.** A
refused compensating removal leaves the transaction partly applied, and there is no
implementation that makes that go away — the choice is only between telling the user and not.
The guard wins, the package stays, and the rollback returns an error naming it and the reason.
The alternative — exempting recovery paths so the rollback can always complete — is the shape of
S24 exactly: a delete that runs where nobody is watching, on the argument that it is only tidying
up.

**`Prior::Unknown` is the third state that both defects needed and neither had.** Absent, present,
and *could not tell* are three answers, and the bug in each case was a two-valued type flattening
the third into the one that removes. It is the same distinction `search_output` already draws
between "no result" and "could not answer" (V.7c) — written down twice now, in two modules, which
is the argument for reading V.7c before adding the next boolean about a manager's reply.

---

**V.86 — Why the command surface was not consolidated, and why one command was renamed (U42).**
*(Raised by the production-readiness review, 2026-07-27; measured, ruled and built the same day.)*

The review counted 45 top-level commands and named four overlapping clusters. The count was 62
and **ten of the thirteen commands it named do not exist** — `remove`, `prune`, `orphans`,
`clean`, `unmanaged`, `status`, `doctor`, `migrate`, `clone`, `generation`. Both of its headline
examples were about commands that are not in the program. This is S24's lesson wearing different
clothes: *an audit reads what is written; only running it reads what is there.* The cluster list
was assembled by reading, and `shall --help` would have taken ten seconds.

**So the first rule here is about rulings, not commands: a decision to remove a feature is
checked against the running program before it is made.** A consolidation argued from a wrong
inventory removes real capabilities to fix an overlap that was never there.

The removal verbs are not synonyms, and the proof is that no two of them can be swapped:
`uninstall` takes a package away; `remove-orphans` takes away what the *manager* considers
orphaned; `purge-undeclared` takes away everything Shall does not manage; `unmanage` takes away
nothing and forgets one package; `reset` takes away nothing and forgets all of them;
`clean-cache` takes away archives and no packages at all. Two of those six delete software, two
delete records, one deletes downloads. A count is not a smell.

**What was real was a name.** Going back has two mechanisms — the filesystem, and the manifest
history — and II.13 already says so in one line: *"Git is your intent. Snapshots are your
machine."* The command surface did not say it. `undo` was the snapshot gallery; `history` and
`rollback` are the manifest history. The most natural word in the program pointed at the less
likely of the two meanings, so someone wanting to undo their last `sync` reached for `undo` and
got a filesystem restore. **A verb inherits the vocabulary of the mechanism it drives** —
`snapshot restore` sits with `snapshot list` and `snapshot prune`, and says which of the two it
is before it is run rather than after.

**`undo` is retired, not reassigned.** Giving a word that already meant the wrong thing a second
meaning leaves every existing mention of it ambiguous, including the ones in a user's shell
history. There is no legacy here, so the name goes.

---

**V.87 — Why an ordinary run says nothing about itself (U43).** *(Raised by the
production-readiness review, 2026-07-27; measured, ruled and built the same day.)*

The default log level was `info`, and 256 `info!`/`warn!` sites sat above it. What that produced
on every ordinary run was Shall narrating its own startup — `No state file found at …`, printed
*every* time and not just the first, because a read-only command never writes the registry it
has just reported missing. The user asked what is installed and was told, first, about a file
they have never heard of.

**A program's output is its answer. Everything else is asked for.** That is the rule, and the
default follows from it rather than from a preference about verbosity.

**The half that had to land first is the half that makes the rule safe.** Some `info!` lines
were not narration — they were the whole answer. `sync` on an up-to-date machine printed
`already up to date` at `info!` with **nothing on stdout**; `lock` and `unlock` reported
everything they did the same way. Dropping the default level without moving those would have
made a no-op sync completely silent, which is worse than noise: noise is ignorable, and silence
is indistinguishable from a crash. So the ruling is two rules and the order between them
matters — **a command's answer goes to stdout; only narration goes to the log** — and twenty-three
lines moved before the default changed.

**The flag that was supposed to cover this did not work, and the reason is the general one.**
`--verbose` promised debug-level logging and delivered none: the subscriber was built at
`main.rs:41`, clap did not parse until `:81`, and `cli.verbose` was read into the executor and
never into the filter. It had been that way long enough for the help text to be quoted as though
it were true. **A flag whose effect is set up before its value is read is a flag that does
nothing, and nothing about it looks wrong** — no warning, no error, and a help string that
promises the behaviour. The level is now read from argv directly, which is also what lets it be
correct before the shim hijack runs.

**`-q` beats `-v`.** A run that asks for both meant the quiet half; nobody types `--quiet` by
accident.

---

**V.90 — Why a failed install takes its line back, and why only sometimes.** *(Owner ruling,
2026-07-27 — Q1.)* `install` writes the line first and syncs second, deliberately (S15):
backwards, every refusal on the write landed *after* the package was already on the machine, in
no file, and drift by the next sync. The cost of that ordering is that a failed sync leaves a
line behind, and **every later command parses the model** — so one line nothing can satisfy
breaks `sync`, `upgrade`, and every install after it, until someone finds and hand-edits a file
nothing named.

The code already knew this. The comment above the withdrawal path stated the failure mode in
the author's own words. What it withdrew on was `Unresolvable` alone — *no backend claims this
name* — and that is not the case people hit. **A qualified typo (`scoop:definitely-not-real`)
resolves perfectly well**, because the backend is real; the failure arrives as `CommandFailed`,
and the line stayed forever even though it could never succeed. A bare `shall install typo` was
withdrawn correctly the whole time, which is why it went unnoticed.

The missing fact was already computed. Every failure carries a `Retryability`, filled in by the
backend's own `ExitPolicy`, and scoop's policy already marked this exact failure `Permanent`.
Shall classified it as impossible and then kept it anyway.

**Three limits on the widened rule, and each is load-bearing:**

1. **Permanence is read off `CommandFailed`, not off `Error::retryability()`.** That method also
   returns `Permanent` for `Refused`, `Cancelled`, `Config`, `Validation`, `Permission` and five
   more. Every one of those is permanent in the retry sense and none of them means the name was
   wrong. Withdrawing on `retryability()` would delete the line a user just asked for because
   they answered "no" to a prompt — a worse bug than the wedge, and a silent one.

**And then permanence turned out to be the wrong question (N-1, 2026-07-29).** The rule above is
still right about what it forbids and was wrong about what it permits. Reading
`CommandFailed { retry: Permanent }` as "this name cannot exist" fails in both directions:

- **Too narrow.** Only 12 of 48 backends had an `ExitPolicy` at all. The other 36 answered
  `Unknown` to everything, so they could never produce the verdict withdrawal was looking for —
  and a mistyped `npm:` package wedged the config while the identical typo behind `scoop:` did
  not. Nothing about npm was special; it was one of the 36, and it was the one that got typed.
  The rule was verified against the two backends that had a reproduction attached, which is the
  habit this whole register exists to break.
- **Too wide.** helm's `plugin already exists` is permanent about a name that is plainly there,
  and `cargo`'s `no binaries` is a real crate that simply ships no program. Withdrawing on
  either deletes a declaration whose package exists — the same class of harm as the wedge, in
  the other direction.

So the two questions are separated in the data: `permanent_markers` answers *would another
attempt differ?*, `absent_markers` answers *does the name exist?*, and only the second withdraws.
Absence implies permanence and permanence implies nothing. Backends that resolve names
themselves — a git host, an index — return `NoSuchPackage` carrying the name they looked up, so
nothing has to be recovered from prose; `pixi` wraps its output through the middle of a package
name, which is what a prose-parsing reader looks like when it finally meets a manager that
formats.

**And then absence turned out to be a claim about an index (2026-08-02).** Separating the two
questions was right and still left one road open, because it never asked whether the manager was
in a position to answer. Measured: `choco install -y bat --source=https://127.0.0.1:9/api/v2/`
— a port nothing is listening on — prints `bat not installed. The package was not found with the
source(s) listed.` That is choco's `absent_markers` entry, word for word, and the only thing
separating it from a genuine typo is three connection lines above it. **A dropped VPN therefore
deleted declarations for packages that exist.** apt is worse and more common: a `sources.list` it
could not fetch makes `Unable to locate package` the answer for every package on the machine.

This was never a permitted behaviour — `target-state` already said *"Kept: everything else. A
dropped network, a held lock, a failed hook — you did mean it, and retrying is right."* The code
simply could not obey it, because absence was consulted before transience and no amount of
network vocabulary would have been reached. So `transient_markers` now outranks
`absent_markers`, in `retryability` and in `names_an_absent_package` alike, and a manager that
says in the same breath that it could not read the index does not get to say what is in it.
`permanent_markers` still outranks both: a request that is wrong stays wrong however the network
behaved.

The shape of the miss is the familiar one. The pair `choco`/`winget` was found by reading the
policy table by hand, exactly as the 36 policyless backends were, so the property is derived and
ratcheted now rather than re-read: `tests/benign_exit_contradiction_tests.rs` fails on any policy
that forgives an exit code it has no vocabulary to contradict, which is the defect underneath CI
30684191791.

**And nothing was watching the clock at all (2026-08-02).** `shall -y uninstall choco:bat` ran
76 minutes and removed nothing. The child was `Checkpoint-Computer`, the pre-sync restore point,
and the Windows event log settles what it was doing: **8194, "Successfully created restore point
(Description = Shall: pre_sync)", eighteen seconds after the process started.** It did the work
and then never returned — parked on its own progress bar at 99%, four threads blocked in a COM
call and two in a sleep loop, not one byte on either pipe for the remaining 76 minutes. The
identical call had returned in seconds eleven minutes earlier in the same run, so it is a race,
not a configuration.

The root is one line that isn't there. Every command Shall runs funnels through
`RawExecutor::execute`, and it awaited the child with no bound of any kind. The only timeout in
the tree wraps `execute_internal()` — the transaction DAG — so it covers task commands and
nothing else: not the snapshot, not the state reads, not the guard, not `plan`. And the omission
is provably an omission rather than a decision, because the spawn already sets
`kill_on_drop(true)` with a comment naming *"a worker whose task is aborted — a failed node, the
global timeout"*. The machinery to cancel was built and correct. Outside the DAG nothing ever
pulled the trigger.

**Not the first time, and the first two were never diagnosed.** `history.md` records
`uninstall gem:colorize` at eight minutes and `install github:sharkdp/fd` at fifteen, both on
Windows, both killed by hand, both written up as *"the shape: on Windows a sync-path command can
stop returning"*. What got fixed then was the **harness** — the sweep learned to wrap its calls
in a timeout. The product kept the bug, which is why the third one cost 76 minutes instead of
being a named failure. The note also reaches for `network_timeout_secs`, an HTTP timeout, to
explain a wedged subprocess; that reflex is itself the evidence that no command-level bound
existed to reach for.

**The bound is on silence, not duration, and that is the whole design.** A `cargo install`
compiling from source and an `apt dist-upgrade` both run for tens of minutes and are working
throughout. No wall-clock cap separates them from a wedged one — there is no number above the
first and below the second — and a cap that killed real builds would be a worse bug with a nicer
name. What does separate them is that working commands *talk*. So the bound is on a child that
has produced nothing on either stream and has not exited. The honest cost: a legitimately silent
command (`Checkpoint-Computer` is exactly that) is only bounded by a number above its real
duration, so the default is 900s — a hang ends in fifteen minutes with a sentence naming the
argv, rather than never. `latency.rs` cannot help here and it is worth saying why: it reports
**after** a command returns, so the one failure it can never see is the one where nothing
returns.

**The sibling was a second way to wait forever.** Auditing the tree for the family turned up
eleven spawns outside the executor, and ten of them captured both output streams while leaving
stdin **inherited** — `git` (every invocation), `--version` and `--help` probes, `generate:`
scripts, vars providers, the `sh()` builtin, download commands. A child that prompts there asks
into a pipe nobody displays and then blocks on a terminal it was never handed: invisible, and
permanent. The executor has closed stdin on reads since it was written, with a comment saying
why; the bypasses simply predate it. That rule now binds every spawn in the tree.

**The message keeps exactly one job.** Not "does the name exist" — a property answers that — but
"which of the lines this command just wrote was the manager talking about", which no property
can answer for a batch. A wrong answer there keeps a declaration that could have been withdrawn,
which is the safe direction; a wrong answer to the first would delete one.
2. **Only lines the manager named.** Managers name the package they could not install, so a
   batch whose manager stopped at the first bad name leaves the rest alone. Guessing which line
   a message meant is how a correct declaration gets deleted.
3. **A line kept on purpose says where it is.** The wedge was never only that the line stayed —
   it was that no message mentioned the file, the line, or `unmanage`. Keeping a line is a
   design decision; keeping it *silently* is the bug.

**What made this durable is worth more than the fix:** the rule existed in three places and
nowhere authoritative. `run-in-container.sh` said the line must not be left; `integration-
windows.sh` said the line stays on purpose and cited `V.7c`, which is about telling a search
that found nothing from a search that could not run. Neither claim was in Part II. **And both
harnesses deleted the line themselves before asserting it was gone**, so neither reading was
ever tested and both printed PASS for months. A rule that lives only in comments is a rule two
comments can contradict.

**V.90b — Why the resource half of the model is one computation and not five.** *(N-2,
2026-07-29.)* G-1 listed three failures of the extras family: the teardown was unguarded,
uncounted and invisible. Round 2 closed the first two at the mechanism and the third looked like
a reporting detail. It was not. It was the model missing half of itself, and the reason it
survived a green run is that **every command that could have contradicted the others was asking a
different question of a different source.**

Measured: `check` reported "the machine matches your files" with three `link:` lines declared and
nothing on disk, and again after a file Shall had placed was deleted behind its back. `sync`
placed those files and printed `already up to date`, because its summary counted packages and the
apply loop's per-item lines went out at a level below the default filter. `plan` froze
`{"installs": [], "removals": []}` in both directions while `--dry-run sync` on the same tree
named all three teardowns — and the guard's refusal text, new in round 2, tells the user to run
`shall plan` to "see exactly what would be undone".

**The two questions are separate and only one of them has a record.** The extras ledger knows what
a previous sync put in place, which answers *has this ever been applied?* for all six kinds
identically. It cannot answer *is it still in effect?* — a `link:` whose target a user deleted is
recorded as applied and is gone. That half has to ask the machine, and the machine can only be
asked about some kinds: a `link:` and a `shim:` are file tests, a `setting:` reads back through an
adapter with no current value. So the answer is three-valued, and the third value is **named**.
A command whose job is "does the machine match?" may say no, or yes, or *yes except these, which I
did not look at* — but never the second when it means the third. That is the whole finding in one
sentence, and it is the same sentence as the `[READY]`-backends-answer-`list` oracle in V.99.

**A found sibling, recorded because it is the same disease in miniature.** `ShimManager` held two
copies of the `.exe` rule. `create_shim` replaced any extension that was not `exe`; `remove_shim`
appended one only when there was none. `shim:tool.bat` therefore deployed `tool.exe`, removal went
looking for `tool.bat`, found nothing, and returned `Ok` — a shim left on PATH under a successful
teardown. Nobody wrote the second copy wrong; it was written *the same day* and drifted. Giving
the path one definition was a prerequisite for asking "is this shim in effect", which is how the
divergence was found at all.

---

**V.91 — Why "not installed" is not "critical".** *(Owner ruling, 2026-07-27 — Q2.)* `check
health` opened with `Backends: 25 OK, 0 degraded, 23 critical (of 48 total)` on an ordinary
Windows box with nothing wrong. The 23 were apt, brew, pacman and the rest of Linux — managers
that machine will never have. It is the first thing a new user sees.

This is fail-loud pointed at something that did not fail. The principle exists so a thing that
did not work cannot pass silently; a manager nobody asked for did not fail to do anything.
Spending the word "critical" on it is worse than cosmetic — it makes the real criticals
unreadable, which is the exact cost of an alarm that is always on.

The tell was that Shall already disagreed with itself: the `check` rollup printed `ok health 25
backend(s) ready` while `check health` called the same machine 23-critical. **Two counts of one
machine, and no rule said which was right.** So `Absent` is a state rather than a filter, and
both views read one tally — a second way to count is a second answer.

**V.92 — Why a typo is exit 1 and not exit 2.** *(Owner ruling, 2026-07-27 — Q3.)* The readme
publishes four exit codes and says "the same four everywhere, so a script can branch on them".
Code `2` means *a read-only command looked and found work to do* — it exists so `shall check` in
CI can report drift without failing the job. Measured: `shall nosuchcommand`, `shall
--nosuchflag` and `shall sync --badflag` all exited **2**, because that is clap's convention for
a usage error and clap exits before Shall's own mapping runs.

So the one code whose entire purpose is unattended scripting was ambiguous in exactly the
unattended case: **a CI job following the published table reads a mistyped command name as "the
machine has drifted"** and acts on it. A fifth code would have fixed the collision and broken
the property the table is for. `1` is already published, already means "something went wrong",
and is true — Shall did not do what was asked.

Ruled alongside it because it is the same contract: **a refusal that exits 1 is a broken
promise, not a rounding error.** `purge-undeclared`'s ratio refusal used `anyhow::bail!` rather
than `Error::Refused`, so it never reached the `Exit::Refused` mapping. `3` is distinct from `1`
precisely so a script that retries on failure does not retry a refusal. **Neither harness could
see it**: both assert refusals with `nok`, which accepts any non-zero code and cannot tell 1
from 3 — an assertion too coarse to detect the thing it is named after.

**V.93 — Why nothing is labelled "experimental".** *(Owner ruling, 2026-07-27 — Q4. The owner
rejected the recommendation, and the reason is the more important half.)*

The readiness assessment proposed splitting the backends: *supported* for the 22 that have
passed a real lifecycle against the real tool, *experimental* for the other 30, said so in
`check health`, in `priority`, and in the readme. It was presented as the single change that
would stop the defect class regenerating, and it was recommended.

**The ruling was no, and the reason is a rule about this project: it does things; it does not
cover for not doing them.** A label converts an unfinished job into a permanent disclaimer, and
a disclaimer nobody has to retire is one nobody does. "Experimental" would have made the honest
statement of a real gap into the reason the gap could stay — the gap would be *documented*,
which reads like *handled*, and the 30 untested backends would still be untested a year later
with a caption explaining why that is fine.

So the coverage is the work, and **missing coverage is a release blocker rather than a caption.**
Shall does not go to production until every registered backend has been thoroughly tested and
reviewed. Nothing about the program changes: `init` still scaffolds every manager it finds,
because scaffolding fewer is the same disclaimer written as a default.

**This is the general shape and it applies past backends.** Every defect the assessment found
had a cheap version of this available — soften the check, widen the exemption, note the caveat,
downgrade the gate to informational — and the codebase's own history is a list of times the
cheap version was taken and the class survived: a `fmt` gate rated informational, a catch-all
that softened any install failure to "ecosystem variance", an exemption list nobody validated,
an assertion that deleted its own evidence. Each of those is a label in a different costume. The
answer is the same one: **do the thing.**

**V.94 — Why `@unverified` reaches past the backends that download.** *(Owner ruling,
2026-07-28 — Q5.)* The flag was written for the three backends where Shall itself fetches a URL,
makes the result executable and puts it on `PATH`, and it read as a relaxation of *Shall's*
checksum rule. That framing turned out to be one case of a wider one: the thing being relaxed is
not "Shall checked the bytes", it is **"something checked the bytes"**, and a manager can be that
something.

helm v4 verifies a plugin's signature before installing it. A plugin source that cannot carry one
— a git URL, which has no `.prov` file beside it — is not installed with a warning, it is
**refused outright**:

```
Error: plugin source does not support verification. Use --verify=false to skip verification
```

*(Measured against helm v4.2.3 on 2026-07-28; the output is `tests/fixtures/helm/`.)* So before
this ruling there was no declaration that installed a helm plugin from a git URL at all — and the
one obvious repair, adding `--verify=false` to helm's install command, is the exact failure SEC2
is built around: one edit turns signature verification off for every helm plugin every user ever
installs, invisibly and forever. That is the global "require checksums" switch this design
refused to have, wearing a different name.

`@unverified` already meant precisely the decision being made, so it says it: on `helm:` the flag
becomes `--verify=false`, and without it the manager's verification stands. The alternative
considered and rejected was a helm-specific flag (`@no_signature`): two spellings of one idea, in
a repo whose recurring failure is two of everything.

Three properties survive the widening, and each is a test:

1. **`allow_http` did not travel with it.** The two flags never imply each other (SEC2), and
   they are now checked in separate branches rather than the one loop that made them look like a
   pair. helm's plain-HTTP switch addresses OCI registries Shall does not reach, so `@allow_http`
   on `helm:` is still a line that does nothing, and still refused.
2. **The opt-out stays per line.** A batch whose specs disagree becomes two commands, because a
   flag on a shared command hands one line's decision to a line that never made it.
3. **It stays visible afterwards.** `status` lists what skipped a check for as long as it is
   installed — and the heading no longer says "downloaded", because for helm Shall downloaded
   nothing.

The refusal text is the last piece. helm's own advice names `--verify=false`, an argv no
declaration can write; Shall now appends the flag a user can actually put on the line.

**V.95 — Why a config file may take `apt` away from the built-in, and why it must say so.**
*(Owner ruling, 2026-07-28 — Q6.)* The onboarder's rule was absolute: custom backends register
last and a name already in use is skipped, "so a stray config can't hijack `apt` or `brew`". The
security half of that is right and stays. The absolute half was wrong, and the reason is the one
this codebase keeps meeting from the other direction: **the built-in is a snapshot of someone
else's CLI, and it goes stale.** helm v4 started refusing unsigned plugin sources; pixi renamed
`global upgrade-all`; nimble's `--` stopped meaning what it meant. Each of those was a day, a
week or a release where Shall was simply wrong about a manager and the person in front of the
machine could see exactly what the fix was and had no way to apply it. `overrides = true` is that
way.

**The key is the whole design, not a formality.** Without it the two behaviours are
indistinguishable from the outside: a definition named `apt` either silently replaces the real
one — which is a supply-chain attack with no attack in it, since a pulled config would only have
to guess a popular name — or is silently ignored, which is what a person fixing a broken backend
experiences as "my file does nothing". Requiring the sentence separates them. Taking a built-in's
name now costs **two deliberate acts**: writing `overrides = true`, and approving the file through
II.12's ledger, which is the same door every other executable thing in `adapters/` comes through.
Neither act alone is enough, and neither is a name.

**Loud, and loud every time.** The replacement is announced on every run that loads it, naming
the backend and the program it now runs — not once at approval, because the run that matters is
the one where something goes wrong months later and nobody remembers the file. `check health`
needs no special case: it probes the definition that won, so an override whose binary is not
installed reports that backend critical, which is the true answer about this machine.

**Scoped to backends on purpose.** Snapshot providers, init systems and secret stores register
last and still never shadow a built-in. The argument for widening is the same one, but the blast
radius is not — a wrong `apt` installs the wrong thing, a wrong snapshot provider takes away the
rollback that was supposed to save you — so that is a separate ruling and has not been made.

---

**V.96 — Why the guard covers a `link:` and a `service:`, not only a package.** *(Owner ruling,
2026-07-28 — Q7.)* The guard was built against one story: managed state goes wrong, the planner
schedules every managed package for removal, and the engine carries it out one purge at a time.
Everything about it — the name `protected_packages`, the count called `max_removals`, the advice
to run `shall unmanage` — was written in that story's vocabulary. So when the resource teardown
was added (S20), it was built as the extras' own business and never met the guard, because
nothing in the guard's vocabulary suggested it was about a symlink.

Measured on 2026-07-28: five `link:` lines deleted from a module, `[guard] max_removals = 1` and
`protected_packages = ["f3"]` both configured and both confirmed effective by `shall protected`.
`sync` deleted all five, including `f3`, exited 0, and printed `already up to date`. The preview
printed `already up to date` too.

**Three failures, and only one of them is the guard.** The removal was invisible — no plan line,
no preview line, and the teardown was announced at `info!`, below the default filter. It was
uncounted — the number five never met the limit of one, because the count was never computed.
And it was unprotected. A user who had done everything the documentation asked, in the file the
documentation named, got none of it.

**Why "the same rules" and not "just report them".** The alternative was to leave the guard
packages-only and merely print the teardown first. The blast radius decides it: a `link:` target
can be a decrypted secret, a `service:` is something running right now, and a `setting:` is a
system-wide preference. Those are not smaller than a package, and `readme.md` had been promising
for months that they were covered. The choice was between making the sentence true and deleting
it, and deleting it would have been the first time this project answered a false claim by
lowering the claim.

**Two carve-outs, both from the code rather than from taste.** OS-essential does not apply
because no resource manager publishes such a list, so querying one can only ever return nothing.
Undeclarability does not apply because it asks "could a package line have held this name?", and
for a resource the answer is structurally no — `link:/home/u/.vimrc` is not a package line and
never parses as one. Applying that test to resources marks all six kinds undeclarable and
refuses every teardown on every machine forever, which is a guard that has stopped being about
the user's intent. Both carve-outs are pinned by tests, so a later reader cannot mistake them
for omissions.

**The ceiling counts the command, not the phase.** A sync dropping three packages and three
links removes six things. Checking each phase's own list separately lets a plan pass a limit of
five twice while exceeding it once, which is a ceiling that reports success at the moment it
fails. The package count is threaded into the teardown check for that reason and no other.

**And the enumeration is the real fix.** Both this and V.97's refusal family were found the same
way — not by reading the sentence that quantifies over the paths, but by counting the paths.
`readme.md:266` claimed every removal path was guarded; there were eleven paths and nine guards.
That sentence was true when written and was never re-derived. `tests/removal_guard_enumeration_tests.rs`
re-derives it on every run, and fails naming the file when the count moves. A rule nothing
re-counts is a rule with an expiry date nobody wrote down.

---

**V.97 — Why a refusal about security returns the same code as a refusal about removal.**
*(Owner ruling, 2026-07-28 — Q8.)* The exit-code table exists so a script can branch. Its whole
value is that `1` and `3` answer two different questions: "something went wrong" and "Shall
decided not to". For the entire SEC/T series it answered the first when the truth was the
second, so a CI job could not distinguish "the download was refused because it was plain HTTP"
from "the network was down" — and those want opposite responses. One is a config to fix, the
other is a retry.

**How it happened is the whole lesson.** Nobody chose `Validation` over `Refused` for these.
`Validation` is what you reach for when you are writing a check about a URL, and each of the
nine was written on its own day by someone thinking about that check and not about the exit
table three files away. E25 found one of them, in `purge-undeclared`, and fixed that one. The
family was never swept, because there was a sentence saying it did not need to be.

**The sentence is the defect.** `main.rs` asserted that the `Error::Refused` arm was the one
point every refusal in the program passed through. It was true of every refusal the author had
in mind and false of nine they did not, and because it was written as a guarantee, the next
person to add a security check had a documented reason not to check. A comment that quantifies
over paths is a claim with no test attached, and `history.md` already records that shape as
costing more than the rest combined. It is now a test that enumerates the paths, which is the
only form of that sentence that stays true.

**The hook is worse than the code.** `on_guard_refusal` exists so a person can be told, without
watching, that Shall said no. It fired for a mass removal — which is loud anyway, because the
command that triggered it is one somebody typed — and stayed silent for an unapproved hook, an
unverified binary and a secret written where nothing protects it, every one of which happens on
an unattended run. The hook was loudest where it was least needed.

**Two decisions inside the sweep, both the non-obvious way.** `rehearsal`'s "no container
runtime" stays a refusal rather than becoming a failure, because 7h's exit condition says it
refuses and names what is missing: the alternative is rehearsing on the host, which answers a
different question and calls it the same one. And a refused declaration is **kept** rather than
withdrawn, unlike E1's unresolvable name — Shall refused the line as written, the refusal names
what to change, so the line is the thing to edit. What changed there is only the sentence: it no
longer says `sync` will try again, because it will refuse identically until something changes.

---

**V.98 — Why `--dry-run` stopped being a thing each verb remembers.** The flag was read at the
top of whichever verb its author thought of, which means the property "a preview performs
nothing" was never a property of the program — it was a count of how many authors had
remembered. Two audits found the count, a year apart, and the second one found five more:
`activate`, `deactivate`, `lock`, `git init`, `config init`.

**The worst of the five is not the one that wrote the most.** `--dry-run activate Work`
switched the active profile and **printed nothing**. `active` decides which modules are in the
model, so it decides what the next `sync` installs and removes; a user asking "what would
switching to Work do" was switched to Work and told nothing had happened. The preview was not
merely wrong, it was quiet, and quiet is what makes a wrong preview survive.

**Why a process-wide value and not a parameter.** Threading the flag to every write would be the
same rule with a longer signature: a new call site would still have to be handed it by hand, and
being handed it by hand is precisely what nine sites failed at. `--dry-run` is parsed once,
before any command runs, and there is no run in which one write is a preview and another is not.
So it is a property of the process, set in `main` and read at the write. The default is "write
for real", because a library embedding this crate that never sets it must not silently perform
nothing.

**The exception is louder than the rule, on purpose.** `profile show` writes `active` twice — to
the profile being asked about, then back — because that is how it resolves the answer. Gating
those writes would make `--dry-run profile show Work` print the wrong profile's contents, which
is the same class of defect one level down: a preview that silently answers a different
question. It is written as its own function with its own name, so the next reader sees an
exception rather than an omission.

**A writer that honours the flag is no protection while a writer that ignores it sits beside
it.** *(Round 4, 2026-07-30 — the third finding of this same defect, and the first where the
mechanism was already in place.)* `write_config` did exactly what this entry describes, and
`atomic_write` — the primitive underneath it — was public, three characters shorter, and what
every `save()` method had been calling since before the rule existed. So `--dry-run adopt`
recorded 112 packages in `data/registry.json` as managed while its *manifest* write correctly
went nowhere. Managed and undeclared is the one state the model reads as **the user deleted
every line**: `shall check` then reported `112 to remove` and told the user to run `shall sync`,
and it removed them. Driven end to end in a disposable data directory on one package, and above
`max_removals` the count guard would have refused first — but any machine with fewer than twenty
adopted packages gets it with nothing in the way.

The fix is the one this entry already argued for, applied one layer down: **one writer**, named
`persist`, with the primitive private behind it. A verb cannot reach the disk during a preview by
picking the shorter name, because there is no shorter name. `hold`, `unhold`, `adopt` and
`path --set` phrase their output from what that writer *answers* — `Held` or `would hold` — so
the past-tense sentence and the write can no longer disagree.

**And the check is a gate over every verb, not over the five.** The audit that found these had
probed 13 of 61 subcommands, so its honest conclusion was "at least five" — a number nobody
should have to re-derive by hand a third time. The gate snapshots the config directory, previews,
snapshots again, and demands the bytes match. Its second half matters more: it also runs the
command *without* the flag and demands that something changed. A dry-run assertion over a
command that could not have done anything is the vacuous assertion this whole programme exists
to remove, and it is the exact mistake the grader made on `activate` before catching itself.

---

**V.99 — Why `list` refuses a backend name and `install` was not enough.** *(Owner ruling,
2026-07-28 — Q9.)* The two verbs are asked the same question — "is this a manager you use?" —
and gave answers a user cannot reconcile: one refused with a message naming the file to edit,
the other printed nothing and reported success. The failure is not that `list` was quiet. It is
that its silence is *already meaningful*: zero rows and exit 0 is what a real, empty manager
prints, so the typo did not produce an absence of information, it produced **wrong**
information, in Shall's own voice.

**The second answer is the one that is easy to skip.** Making the typo loud is worth little if
`flatpak` on a machine without flatpak still prints nothing, because the user still cannot tell
which of the two they are looking at — and only one of them is a mistake they made. So a
registered backend that cannot run here says so and exits 0, and a name nothing claims is an
error. Two facts, two answers. Refusing both would have traded one wrong answer for another,
which is why the registry is asked whether the name exists *before* it is asked whether it
works.

**And it un-disarms a measurement.** The readiness rubric asks that every `[READY]` backend be
able to answer `list`. That was measured at 24 of 24 and was worthless: 13 of the 24 returned
no rows, and a backend that does not exist returned no rows too, so for half the subjects the
check could not fail. This is the vacuous assertion the whole assessment is about, found inside
the check written to demonstrate the opposite. The oracle is now itself tested — a nonexistent
name must be distinguishable from a real one before the 24-of-24 figure means anything.

**And the ruling's own enumeration was half of one (2026-07-29).** Q9 binds "every verb taking a
backend name" and listed `list`, `upgrade`, `rebuild` and `repo` — the four that take it as a
`--backend` flag. A backend name has a second spelling, the `backend:` prefix on a package spec,
and nine verbs took it without checking: `hold` went as far as *recording* a hold under a manager
that does not exist and reporting success. The rule was right and its coverage was decided by
which spelling the reporter happened to type, which is why the check for it is now derived from
`--help` rather than written as a list — `tests/unknown_backend_family_tests.rs`, whose
exemptions are themselves asserted to exist so they cannot become E29's `undo`.

---

**V.100 — Why a failure that survived its retries stops calling itself transient.**
`Retryability::Transient` is a claim: *a second attempt could differ*. The container harness
proves that claim the only way it can be proved — it retries once and calls a repeat a defect.
The product asserted it from a substring and nothing ever asked whether the substring was
right.

Measured: `luarocks install luafilesystem` on a machine where `https://luarocks.org/manifest-5.5`
returns 200 but the `wget` first on PATH is a scoop shim that rejects the flags luarocks passes.
The output contains "failed downloading", `exit_policy::luarocks` lists that as transient, so
Shall kept the declaration and told the user `sync` would try it again. It fails identically
forever. The policy's own doc comment named that exact cause and classified it as the network
anyway.

**The evidence was already being collected and thrown away.** The transaction retries a
transient failure with backoff, so by the time it gives up it has re-run the command — four
times, here — and seen the same answer. That is the experiment. Nothing recorded its result:
the final error still carried the classification the first attempt's string match produced.

So the retry count now falsifies the claim, and the verdict has its own name. `Exhausted` is
kept apart from `Unknown` because the two lead to different sentences — `Unknown` means nobody
looked, `Exhausted` means somebody did — and apart from `Permanent` because `Permanent` is the
verdict that *withdraws a declaration*, and "we tried and it did not differ" is not "this can
never work". The wget on that PATH could be fixed tomorrow. Guessing `Permanent` would delete
a line over a broken environment, which is a worse bug than the one being fixed.

**What this makes the markers.** They stop being a promise and become a hypothesis with an
experiment attached: a wrong entry now costs a few seconds of backoff and an honest message,
instead of a sentence the program repeats forever while its own retries disprove it.

---

**V.101 — Why the sweep has a floor that moves.** *(Owner ruling, 2026-07-28 — Q12; the owner
ruled the shape and left the number to the builder, so the mechanism is one that needs no
number.)* The coverage audit was written to answer "is any backend untouched?", and it answers
that well. It was then read as if it answered "is this run as good as our runs usually are?",
which it never did: a plan-smoke satisfies it, and a plan-smoke proves an argv was *constructed*.

The measurement that exposed it: a clean Windows sweep, nothing broken, `4 real lifecycle, 12
install-attempted, 44 plan-smoked`, `PASS`. Four, because 8 of 15 canaries were already
installed on that host and the harness refuses — correctly — to remove software the user already
had. So the better-used the machine, the less the gate tests, and the gate says the same word
either way.

**A threshold was the obvious fix and it is the wrong one.** Whatever number you pick is right
for one machine on one day. Pick CI's and every developer's box is red; pick a developer's and
CI can silently halve its coverage. Both failures end the same way — someone stops reading the
line. A ratchet asks a question that has an answer on every machine: *did this host class do
worse than it has done before?* Nobody has to guess, and the only way to make it green
dishonestly is to edit a number in a committed file, which is a line in a diff someone reviews.

**The class had to be got right, and neither of the first two attempts was.** `uname -s` under
git-bash is `MINGW64_NT-10.0-26200`. Keyed on that, every Windows update would mint a new host
class with no record and a free pass — a ratchet that resets itself is a ratchet in name. The OS
token is normalised to `windows`/`linux`/`darwin`, and `ci` is separate from `local` because that
distinction is the finding rather than noise around it.

The second attempt made the container's **distro** part of the key, "because ubuntu and the
`tools` image are not comparable runs" — and read it from `/etc/os-release`, which answers
`ubuntu` for both, because `tools` is built on Ubuntu. Measured on CI run 30503630610: `tools`
completed 25 real lifecycles and the ubuntu image 7, both filed under
`container-linux-ubuntu-local`. Whichever wrote the record made the other permanently wrong — one
held to a number it cannot reach, the other free to lose 18 without a word. The key is the
image's own declared `SHALL_IT_IMAGE`, and an image that declares none is a named gate failure
rather than a silent merge into whatever it was built on.

**And a first run must not be a pass.** "No record for this class yet" was a counted PASS, on the
argument that failing a new platform is how a gate stops people adding platforms. That argument
is right and the conclusion does not follow from it. The same run took that branch on **7 of 7**
host classes — every container leg and both native CI legs — because one developer's machine was
the only place that had ever written a record: a ratchet in force nowhere, reporting green
everywhere. It counts as neither pass nor failure now, which is what a comparison against a
record that does not exist has earned. Nothing in the suite noticed except the mutation gate,
counting one more check that survives a do-nothing binary.

**And it goes in both harnesses.** The same audit exists in the native sweep and the container
sweep, and putting the ratchet in one would be `guard.rs`'s own lesson — a check on one path is a
check on nothing — repeated in the file that measures the checks.

**V.102 — Why Shall asks before setting a manager up, and why it does it at all.** *(Owner
ruling, 2026-07-29 — Q10, Q11, Q13.)* Three managers in the `tools` image failed **every**
install, and not one of the failures was a Shall defect: `mix` had no Hex, `asdf` had no plugin
for the tool it was handed, `opam` had no switch. Each printed an accurate message that the
person reading the CI log could act on, and Shall — which knew the command — printed it and
stopped.

**Doing it silently was the obvious answer and it is the wrong one.** `asdf plugin add` clones a
third-party git repository whose shell scripts asdf then executes. `opam switch create` builds a
compiler and pins it for the whole account. Those are not "one more command"; they are the kind
of thing that must not happen because a config file said so and nobody looked — the same
sentence II.12's ledger exists for, and the same one `[[bootstrap]]` was already written around.

**Printing it and stopping was the other obvious answer and it is also wrong**, for the reason
P8 gives: Shall does the thing, it does not hand you the thing to do. A tool that knows the
command, is holding the terminal, and asks you to go and type it yourself has chosen the least
useful of the three options.

So it asks, and `--yes` answers in advance. The flag was not invented for this — it is the one
that already means "I have decided, proceed" — and a second one would have split the same
question in two.

**The probe is the part that keeps it from becoming noise.** A row that could not tell whether it
was needed would offer on every sync, and an offer you see every day is an offer you stop
reading. It also had to be the *right* probe: `asdf plugin list` exits 0 and prints `No plugins
installed`, so an exit-code probe reports every missing plugin as present — the shape of the
`command -v` bug in `CLAUDE.md`, one tool over. And line-exact rather than a substring, so `jq`
is not answered by `jqx`.

**Two defects were hiding behind the first one**, which is why this entry names them. The mix
canary was `mix:hex`, and `mix archive.install hex hex` cannot succeed even with Hex present —
so an impossible canary and a real defect were reported as one failure, and fixing either alone
would have left the check red and looking fixed. And `mix archive.uninstall` without `--force`
prompts, takes the empty answer from a closed stdin, exits 0, and leaves the archive installed:
Shall reported removals that never happened, which is E7's shape in a manager nobody had looked
at. A prerequisite that hides two other bugs is the ordinary case, not a surprise: nothing
downstream of a manager that cannot install anything is ever exercised.

**V.103 — Why a bare keyword is a parse error, and why nothing was quoted to fix it.** *(Owner
ruling, 2026-07-30 — Q16.)* Thirteen of the fourteen words that introduce a statement are also
real package names in real indexes: `cargo:link`, `cargo:when`, `pip:absent`, `scoop:shim`,
`gem:if`, `npm:else`. A package name is one bare word (II.2), so `link` on its own was a valid
package line — and the most likely typo in the whole format, a resource prefix typed without its
colon, therefore declared a package, resolved it against a live index, and got `shall check` to
recommend the `sync` that installs it. Every preview in the program agreed, because the model
genuinely contained it; **no gate downstream of the parser can catch a model that is wrong in a
well-formed way.** A typo that stops costs a user ten seconds. A typo that installs software
costs them a machine they no longer recognise, and the count guard never fires because it is one
package.

The owner asked for a way to still mean the package, and the answer was that the language
already had one: a bare `NAME` is *defined* as short for `list:NAME`, so `list:link` says
precisely what the bare form used to and needs no new grammar. **Quoting was considered and
rejected, because V.10 already rejected it for the reason that still holds** — `"` needs `\"`
needs `\` needs a newline rule, and a language that has to explain its escaping is not the
language this one is trying to be. The ruling adds a refusal and removes nothing.

**Built 2026-07-30 against twenty-two words, not the thirteen that were measured**, because the
thirteen were a sample of the family and not the family. Nine more reach the package parser by
the identical route: four statement prefixes the grade never tested (`exec:`, `dotfiles:`,
`firewall:`, `generate:`) and five directives whose bare form has no delimiter to catch it
(`exclude`, `intersect`, `module`, `use`, `param`). Shipping the refusal for `link` and not for
`exec` would have been the reported symptom fixed and the class left live.

**And the keyword list is now one list.** Three had grown: the "unrecognised line" message knew
six prefixes, the dispatcher eleven, and the set-expression guard a *different* nine. That last
disagreement was a live bug of its own, and it was measured rather than argued: on the old list
`generate:C:\tools\list-packages.ps1` parsed as a **set expression**, because the copy deciding
whether a backslash meant set math had never heard of `generate:`. (`setting:` was missing from
the same copy and is *not* affected — a setting is `SCHEMA/KEY` and its validator rejects a
backslash before the ambiguity can arise. The first draft of this entry claimed it was, and the
test written to prove it disproved it instead.) A bare keyword cannot be refused reliably while
the answer to "is this word a keyword" depends on which of three copies you ask.

**V.104 — Why `@unverified` is silent on a tool that does not verify.** *(Owner ruling,
2026-07-30 — Q14.)* helm 3.21.3 does not verify plugins at all: `helm plugin install --help`
documents `--help` and `--version` and nothing else — no `--verify`, no `--keyring`, no
provenance. It verifies *charts*; helm 4 added plugin verification, which is where
`--verify=false` came from. So on helm 3 the state `@unverified` asks for is the state the
machine is already in.

That distinction is the whole entry. **"Accepted and does nothing" is a defect; "accepted and
already true" is a correct no-op**, and reading the second as the first would have refused a
correct line and removed the only way to install a helm plugin on helm 3 — the capability Q5's
ruling existed to create. The register had this filed under the wrong diagnosis for a week
because nobody had run `helm plugin install --help` on a helm 3.

It is silent rather than warned for the reason every other rule here is: a warning on a run that
did the right thing teaches people that warnings are noise, and the next one that matters is
read the same way. And it is what makes the capability table testable — the assertion is
two-directional, *a flag where the tool verifies and none where it does not*, which can go red on
either version. The gate it replaces could only be written on a helm 4 host, and was red on helm
3 for a reason that was never drift.

**V.105 — Why a preview does not write the file it was told to write, except `plan`.** *(Owner
ruling, 2026-07-30 — Q15.)* The tempting line is "the user named the path, so nothing was
surprised" — and it is wrong for `bundle`, because the artifact outlives the run. A restore
bundle exists to be carried to another machine and unpacked there; one produced by a preview is
indistinguishable from one produced deliberately, and the next person to find it has no way to
know it was a rehearsal. `--dry-run bundle` printed *"Bundle written to X"* over nine real files,
which is the same past-tense-about-a-write-that-did-not-happen defect as B-1 with the sign
flipped: it happened, and said so, under a flag that promises it did not.

**`export` was ruled with `bundle` on the reasoning and had never been measured** — the grader's
fixture had nothing to export, so neither run wrote anything and there was no control. Measured
2026-07-30 against a fixture with 111 adopted packages: **`export` already complied.** It prints
`[DRY-RUN] would write <path>` per manifest and writes none of them, while the control writes
both. So the ruling changed nothing about `export`, and the code change is `bundle` alone. It is
recorded because "ruled on the reasoning, then measured, then found already correct" is a
different fact from "ruled and built", and a reader who cannot tell them apart will not know
which parts of a ruling were ever tested.

`sbom` is **not** in this family, confirmed rather than assumed: it takes no output flag and
prints its document to stdout, so there is no artifact for a preview to manufacture.

**`bundle` writes through a facade rather than a check at the top.** The obvious fix — return
early under the flag — would produce a preview that says nothing, and the round-5 finding on
`--dry-run activate` is that acting silently is the *worse* half. So every write in the bundle
(the config copy, the git bundle, the registry, `packages.json`, `RESTORE.md`, `plan.json`, the
archive, and the artifact pre-fetch, which is a network download) goes through one `Writes`
value that counts what it would have done. The summary is the same summary; only the tense
changes, and it changes because the writer says so rather than because the flag was read a
second time.

`plan` is exempt because **its file is the preview, not the result.** `--dry-run plan` that wrote
nothing would be a command with no output — the flag would turn the command off rather than make
it safe. The line the rule draws is therefore not "did the user name the destination" but
"**is the file the description or the thing described**", and that reading is why `export` lands
with `bundle`: a Brewfile is something you hand to brew, not something you read to find out what
Shall would do.

**V.106 — Why the option table and the keys backends read are one list.** *(Owner ruling,
2026-07-31 — Q18.)* Part II said both things at once: its option table permitted fifteen keys,
and its storage paragraph said a volume "has a size and a mountpoint". Both halves were
implemented faithfully, which is how `lvm:` came to be **unwritable by construction** — the
backend refused every line without `@size` because `lvcreate` has no default size, and the parser
refused every line with one. The backend's own error told the user to write a line the grammar
rejected, and there was no third form. It had been that way since the day it was merged, and
nothing noticed because a backend that operates on block devices was excused from every harness
until Q17 gave it a privileged container.

**The table was the half that was wrong, and the fix is the join rather than the four keys.**
`PACKAGE_OPTION_KEYS` and the keys backends actually read were two lists with nothing holding
them together, so the same defect was sitting on three more keys nobody had looked for: `snap`'s
`@classic` (its `--classic` branch had never executed), and `@shim` / `@sandbox`, which `sync`
reads to decide whether a tool gets a PATH stand-in. That last one is the one to remember. **R3
deleted the imperative `shim` command in July and pointed at `@shim=true` on the package line as
the declarative way to ask for one** — and a different change in the same month closed the option
table into a whitelist that did not contain `shim`. So the ruling pointed at the one form that
did not parse. It did not leave shims unmakeable, and the first draft of this entry claimed it
had: a standalone `shim:NAME` statement still parses and is still reconciled, which reading
`app/apply/dependents.rs` settled. The lesson survives the correction, because the shape is the
same one and it is the shape that matters: **two changes, each defensible alone, and between them
a documented form that no file could contain** — while every test stayed green, because nothing
asserted that the keys the code reads and the keys a line may carry are the same keys. The join
is now `backends/capability.rs`, one table read by the grammar and by the install path, with a
test across it.

**Why scoped and not simply permitted.** A key legal everywhere is a key that lies on most lines:
`apt:curl@quota=10G` would read as the machine having been told something when nothing anywhere
would act on it, and the option-nobody-reads class is the one II.2 exists to refuse. So each key
is legal exactly where something reads it and refused by name elsewhere, in the shape `@url`
(U39) already uses.

**Why the mount half shipped with the rest, and what it cost.** The narrower option was to land
`@size` and `@quota` and leave `@mount` refused until the fstab path had been proven. The owner
ruled against narrowing: broaden until everything the code can do can be written. That is the
right call and it was not the cheap one, because making `@mount` reachable exposed the state the
fstab code was actually in — it dropped every fstab line *containing* the mount point as a
substring, so declaring `/mnt` would have deleted `/mnt/data` and `/mnt/home`; it wrote `subvol=`
as the declared path rather than the path from the filesystem root, which is the same offset bug
`list` had been fixed for one day earlier, mirrored; and removal left the entry behind. **An fstab
entry that outlives its subvolume is not untidy — it is a machine that stops in the initramfs at
the next boot**, so the entry now goes before the volume does, in that order, and the mount is
released first because a mounted subvolume cannot be deleted. A key made legal over code in that
condition would have been a footgun with a specification blessing it.

**The general lesson is what "unexecuted" is worth as evidence, which is nothing.** Every defect
above was in code that compiled, read plausibly, and had never once run. Reading it found the
substring match and the missing removal; *running* it found three more — a UUID parser that
wanted a line starting `uuid:` from a report that says `Label: none  uuid: …`; the same query
put to the subvolume when `btrfs filesystem show` only answers for a filesystem; and `info()`,
which is what the planner actually asks, answering `Path::exists` so that any directory was an
installed subvolume. No amount of review would have produced the third one, because it is only
wrong in company. **A backend that has never been executed has not been reviewed, it has been
proofread.**

**And a declaration must be able to tell that it was only half applied.** The failed mount left
the subvolume created, so the name was present, so `sync` reported *already up to date* over
work it had never finished — for ever. A declared `@mount=` that does not match what the machine
reports is drift now, in the same place `@version` and `@channel` are decided. **Mounted nowhere
is a state, not an unknown**: the first draft copied D13's rule of leaving an unreadable value
alone and thereby restored the whole bug, because an absent mountpoint is not an unreadable one —
it is the machine saying no. Re-applying a mount is idempotent, so the cost of being wrong in
this direction is a repeated no-op, and the cost in the other is a declaration that never comes
true.

**And one more thing `@mount` creates: a second name for one object.** A subvolume mounted
somewhere else is reachable by two paths, and the second one is undeclared — `remove-orphans`
would have offered to destroy the volume the user had just declared, under its other name. `list`
now answers one package per subvolume, identified by the device it lives on plus its path from
that filesystem's root, and reports it by the name reached through the mount closest to the
filesystem root. Not the *shortest* name, which was the first thing tried and is wrong: a mount
at `/srv` is shorter than `/mnt/fs/data`, and answering `/srv` would leave the declaration
looking unfulfilled and `sync` re-creating it on every run — the 2026-07-30 bug arriving from the
opposite direction.

**V.107 — Why an edited size resizes the volume, and why shrinking says so on the line.** *(Owner
ruling, 2026-07-31 — Q19.)* V.106 made the geometry writable and applied it at creation. **What
it did not do was decide what a changed number meant, and the answer the code gave was
"nothing":** the volume exists under its name, so there is no drift to act on, so editing
`@quota=100M` to `200M` — or `@size=10G` to `20G` — left `sync` reporting success over a
declaration it had stopped applying. That is V.106's own lesson one turn later. A declaration
that cannot tell it was half applied is the same defect whether the half that failed was the
mount or the number, and the fix has to be the class rather than the case.

**Growing and shrinking are two decisions, not one command with a sign.** Growing hands back
space nothing was using. Shrinking takes space off a live filesystem, and on one that cannot
shrink at all it takes away whatever was past the new end — so the builder's recommendation was
to grow and to *refuse* to shrink. **The owner overrode the refusal and required a flag
instead:** shrinking is allowed where the line carries `@allow_shrink=true`, and refused with
both sizes named otherwise. The reasoning is the one this whole document is built on — the
register records what the owner decided, and a tool that decides for them is a tool that gets
worked around. What the flag buys is that **nobody shrinks a filesystem by editing a number and
pressing enter**; what a flat refusal would have bought is a user doing it by hand, outside
anything Shall can see.

**`--resizefs` is the rule, not the implementation.** It runs in both directions, and on the
shrink it is the thing that makes the flag a permission to *resize* rather than a permission to
truncate: `lvreduce` alone chops the volume out from under a mounted filesystem, while
`lvreduce --resizefs` shrinks the filesystem first, so the bytes given up are ones nothing is
using — and xfs, which cannot shrink at all, fails there, **before** the volume is touched. A
flag guarding a bare `lvreduce` would have been a consent form for data loss. The cost is named
rather than hidden: a volume carrying no filesystem fails to grow, because `fsadm` cannot find a
type. That is the honest limit of resizing by declaration, and better than silently applying half
of one.

**The comparison is where this feature dies if it is done casually, and Q19 said so before it was
built.** A quota is printed `10.00GiB` by btrfs, `10.00g` by lvs and `10G` by zfs, and the
declaration says whatever the user typed. **A comparator that reconciled display strings would
report a change on every sync, for ever** — D13's failure mode, which is why D13 required a
*readable* current value in the first place. So every tool is asked for raw bytes (`zfs list -p`,
`lvs --units b --nosuffix`, `btrfs qgroup show --raw`) and **only the declared side is ever
parsed**. And three states are reported, never two: a byte count, `none` where the backend looked
and found no limit, and no property at all where it could not look. Collapsing the last two is a
coin-flip between two permanent bugs — read "could not read" as "no limit" and the quota
re-applies for ever; read it as "satisfied" and it never applies at all.

**The sibling that would have shipped past this fix.** `@mount`'s drift check *returned* from the
function, so a line carrying both a mount and a quota had only the mount looked at — the second
option was dead the moment anyone wrote the two together, which is the ordinary way to write them.
The facets are OR-ed now. **`@mount_options` was dead the same way and for the same reason:** the
fstab entry is rewritten on every install, but no install was ever scheduled, so a changed option
field kept yesterday's options through every sync and every reboot. One reported symptom, two
live siblings — the same count as the container-harness `command -v` bug, which is not a
coincidence but a measurement of how far a fix travels when nobody goes looking.

**V.108 — Why a changed `@classic` re-confines a snap, and why only in one direction.** *(Owner
ruling, 2026-07-31 — Q20.)* This entry exists because of how it was found. Nobody hit it. V.107's
fix was written, and then the question "what else is applied once and never again" was asked of
the rest of the tree — and `@classic` came back, read in exactly one place, when the install argv
is built. A snap that gained the option after it was installed stayed strictly confined for ever,
with `sync` reporting nothing to do. **The same defect, a different backend, and it had been
sitting there since `@classic` was written.**

**The owner ruled it the same way, and the two directions still came out asymmetric — because
snapd is asymmetric.** `snap refresh --classic` relaxes confinement in place; nothing narrows it
back. Going from classic to strict means remove-and-reinstall: a *removal*, of a package the user
declared, to satisfy an option. That is the guard's decision and emphatically not a backend's, so
`@classic=false` on a classic snap is refused by name with the by-hand path spelled out — the
same shape as V.107's shrink refusal, and for the same reason: **the direction that destroys
something says so out loud rather than doing it quietly on your behalf.**

**Omitting the option manages nothing, and that is what makes the refusal safe.** If an absent
`@classic` meant "strict", every existing classic snap whose line never mentioned confinement
would start failing every sync with that refusal — a fix that breaks configs nobody edited. So
absence is unmanaged, exactly as a dropped `@quota` is (V.107), and the refusal can only be
reached by someone who explicitly wrote `@classic=false`.

**And the sibling was inside the sibling.** `@channel`'s drift check `return`ed from the
function — the identical fault V.107 had just fixed for `@mount`, in the branch immediately above
it. A snap carrying a channel *and* `@classic` had only the channel looked at. The argv had the
matching bug one layer down: the refresh was built from `@channel` alone, so a line asking for
both changes would have silently dropped one. **Two spellings of one mistake, twenty lines apart,
and the only reason the second was found is that the first was being fixed next to it.** The
lesson is not about snaps. It is that "check the neighbouring branch" is not a courtesy — the
neighbouring branch is where the same author made the same assumption on the same afternoon.

**V.109 — Why a parked decision's condition is checked by a script.** *(2026-07-31, from D15.)*
`PARKED` is not a state, it is a promise to come back: *not asking you yet, and here is what I am
waiting on.* D15 said, in those words, "parked until D5 is answered". D5 was ruled on 2026-07-24
and built on the 26th — so from the 24th, D15 was a live question the owner had never been asked,
still filed under the status that means *needs nothing from you*. It surfaced on the 31st only
because someone asked what was open and read D5 by hand.

**The register already had a checker, and it passed every day of that week.** It counts the
entries and fails CI if any written total disagrees — which is why the arithmetic cannot drift.
But it verified the *totals* and never the *claims*, and a parked entry makes a claim: that the
thing it waits on has not happened. So the totals were right the entire time the register was
wrong, which is the most expensive kind of green.

The fix is the same shape as the count: a parked entry's `Status:` line must carry
`waits on <what>`, the checker fails if the clause is missing, and it fails if the clause names a
decision that is now ANSWERED. A condition naming an event out in the world — D16 waits on
someone actually hitting the case — is allowed and left unchecked, and **saying that out loud is
the point**, because the alternative is a clause that reads as checkable and quietly is not.

**V.110 — Why "an option converges when you change it" is a rule and not five fixes.** *(Owner
ruling, 2026-07-31 — Q21.)* `Q19` found four options applied at creation and never again; `Q20`
found a fifth on a different backend, by the simple method of asking the rest of the tree the
question `Q19` had just answered. **Neither was reported. Both were found by looking**, in one
afternoon, in code that had been green through thousands of checks.

That is the shape of a class, not a coincidence, and the mechanism is worth naming precisely:
**a lifecycle is install → list → remove, and by construction it never edits a declaration.**
Every harness this project has, every plan-smoke, and most of the unit tests install once. So an
option read when the install argv is built and nowhere else is invisible to all of them — not
under-tested, *untestable* by the shape of the tests. Five features existed in the documentation
and not on the machine, and no amount of running the existing suite harder would have said so.

So the rule is the generalisation rather than the five repairs: changing an option changes the
machine, or the line is refused with a reason and a way out. **"Nothing happens" is not a third
option**, and neither is its mirror — a comparison so loose it reports a change on every sync for
ever (D13). Both are ways of not converging, and a declarative tool that does not converge is a
config file with opinions.

**Two corollaries, each learned the expensive way in the same session.** *Absence manages
nothing*: if a missing `@classic` meant "strict", every existing classic snap whose line never
mentioned confinement would start failing on a refusal it never asked for — a fix that breaks
configs nobody edited. And *the proof is per option, not per backend*: `snap:` had a real
lifecycle for months, which is why the `@classic` defect survived one.

**V.111 — Why `@shim` is a resource and not a package option that gets re-applied.** *(G-1,
2026-07-31.)* `@shim` and `@sandbox` were the sixth and seventh options found dead by the sweep
V.110 ordered, and they failed differently from the other five: not "read once when the install
argv is built" but "read from the frozen state registry" — the map only an install writes. So a
manifest edit that scheduled no install could never reach the decision, in either direction.

The obvious repair was to make the package's drift check name the two keys, the way it names
`@quota` and `@classic`. That would have converged, and it would have been wrong twice over.
**It converges by reinstalling the package to obtain a symlink** — for `@sandbox` the reinstall
does nothing at all, since the confinement lives in `shall run` — and it leaves the
frozen-snapshot reader standing, one install away from the truth for every package the current
sync does not touch.

The right shape was already in the tree and had been since S20: **a shim is a noun with an
inverse.** `shim:NAME` is a declaration, `locks/extras.toml` records that it was placed, the
removal guard counts it, `--dry-run` names it and a deleted line tears it down. A package line
asking for a shim is asking for that same resource by another route, so it resolves to that same
declaration, and every one of those behaviours arrives without being written a second time. The
reconciler that decided from `state.packages` is **deleted**, not repaired: two things placing
shims is the two-of-everything disease, and the one being deleted is the one that could not see
what the file says today.

The safety ledger got stronger by the same move. `remove_shim` had been accounted for by
inheritance — *its only caller runs inside a plan the guard already enforced* — which is a
sentence about paths, of the kind this repo has learned to distrust. It is now counted by
`guard::enforce_extras` over the drift set, like every other resource that can be taken away.

**V.112 — Why a byte-order mark is read rather than refused.** *(Owner ruling, 2026-07-31 —
Q22.)* Every other rule in this document leans the same way: **fail loud, never silent.** This
one goes the other way, and the reason is that the loud failure has nobody to talk to.

A refusal teaches a rule the user can act on — *"`link:` needs a `@target=`"* names a thing they
typed. A BOM is not a thing they typed. Notepad writes it, PowerShell 5.1's `Set-Content
-Encoding utf8` writes it, no editor displays it, and the file looks correct in every tool the
user has. The refusal Shall actually produced was the proof:

```text
`<U+FEFF>cargo` is not a backend Shall uses
  add `<U+FEFF>cargo` to your `priority` file, or check the spelling.
```

— and before the same session's `printable` fix, those two names rendered *identically*. The
advice was to do the thing the user had already done. A message that cannot be acted on is not a
loud failure; it is a silent one with more words.

**So the line is drawn at what the byte means, not at how loud the outcome is.** A BOM at the
start of a file is an encoding artefact — the editor's, not the author's — and reading past it is
what every other tool that reads text files does. A U+FEFF *inside* a line is different: nothing
puts one there but a paste from a web page, it is invisible where it stands, and it is still
refused by name. Stripping every occurrence would be the silent-repair habit this codebase is a
reaction to, and it would hide a real trojan-source vector one codepoint away from U+202E.

**And it is applied at the parser, never at the read.** `model/edit.rs` reads these same files in
order to append to them, and II.16 says Shall must not rewrite your files. That includes their
encoding: a file that arrived with a mark keeps it. Stripping at the read would have quietly
re-encoded a user's config the first time any command touched it, which is a bigger promise
broken than the one being fixed.

**V.113 — Why the first character of a name is the one place `@` is not an option.** *(Owner
ruling, 2026-07-31 — Q23.)* The option syntax and npm's scope syntax want the same character,
and the collision is not hypothetical: `@angular/cli`, `@vue/cli` and `@bazel/bazelisk` are
ordinary packages that `npm ls -g` prints, that `shall list` therefore reports, and that no
module could contain. The refusal a user met was *"`@bazel/bazelisk` is not a list of
`key=value` options"* — advice about a mistake they had not made, on a line they had copied out
of Shall's own output.

**The rule is positional rather than contextual, and that is the whole of its defence.** "An `@`
means an option unless the name looks like an npm scope" would need a table of which backends
have scopes, and the table would be wrong the first time another ecosystem adopts the
convention. "The first character of the name is part of the name" needs no table, no backend
knowledge and no lookahead — and it leaves every existing line meaning exactly what it meant,
because a line beginning `npm:@` did not parse at all before it.

**Two things it deliberately does not do.** It does not make `@` legal *inside* a name — the
second `@` still opens the options, which is what keeps a pin writable. And it does not
introduce quoting: the owner named quoting as a fallback if the rule ever confuses anyone, and
**V.10** already rejected quoting once, because a quote needs an escape, an escape needs a
backslash rule, and a backslash rule needs a newline rule. One positional exception is cheaper
than a lexer.

**And the rule has two halves, which is how it caught its own author out.** The grammar was
taught that a backslash belongs in a package name; `core/validator.rs` was not. So `adopt` asked
"can this name be written?", the grammar said yes, 340 winget rows went into `adopted.txt`, and
every command after that failed to parse the file — a wedged model, which is `E1`'s class
arriving through the door this rule had just opened. **A name is admitted by a grammar and a
validator, and admitting it in one place is not admitting it.** The measurement is on the native
sweep: `adopted.txt:78`.

This is the third defect of one shape in one session — `\` read as set math (G-2), a
byte-order mark read as part of a name (Q22), `@` read as an option (Q23). The shape is: **a
manager prints a name that Shall's own grammar cannot take back.** Where the two disagree the
grammar gives way, because the manager's names are facts and the grammar is a choice.

---

**V.114 — Why the bound is on silence and not on duration.** *(Owner ruling pending — `Q24`,
built 2026-08-02. The pointer to this entry shipped before the entry did; that is the drift this
file exists to stop, and it is closed here.)*

`shall -y uninstall choco:bat` ran for 76 minutes and removed nothing. The child was
`Checkpoint-Computer`; Windows event 8194 records the restore point written **18 seconds in**,
and the process then produced nothing on either stream and did not exit. Nothing in Shall
bounded it — the only timeout in the tree wrapped the transaction DAG, and snapshots, state
reads, the guard and `plan` all run outside it.

**No wall-clock cap can be both above a working command and below a hang.** A `cargo install`
compiling from source and an `apt dist-upgrade` each run for tens of minutes and are working the
whole time. There is no number above the first and below the second. What separates them is not
how long they run but whether they are *saying* anything: the measured hang said nothing for 76
minutes while still holding its pipes open.

So the bound is on silence. `command_idle_timeout_secs` (default 900, `0` removes it) kills a
child that has produced nothing on either stream for that long, and the error names the argv.
900 because the adversarial case is a command legitimately silent for its whole run —
`Checkpoint-Computer` is exactly that — so the number has to clear a real one. **It is a
judgement and not a measurement**, and `Q24` says so: nobody has measured the longest legitimate
silence in Shall's own workload.

---

**V.115 — Why one command per manager per wave.** *(Owner ruling, 2026-08-02 — `Y1`. Rule in
II.19.)*

Measured in a disposable Ubuntu container with each manager binary wrapped by a counting shim:
six declared packages produced **six separate `apt` processes**, argv captured verbatim, and
12,465 ms. `apt install` of *eight* packages as one command took **3,161 ms**. Scaling the same
packages one at a time: 1 → 2,131 ms, 2 → 4,017 ms, 4 → 7,372 ms, 8 → **31,901 ms**. Superlinear,
because each invocation re-reads the package cache, re-takes the dpkg lock and re-resolves a
dependency graph the batch resolves once.

**The batching code was already written.** `generic::install_group` allocates
`Vec::with_capacity(specs.len())`, partitions `@unverified` specs into their own command, and
accumulates names across specs; `push_names` takes an iterator for the same reason. Every one of
those had only ever been handed a one-element slice, because the DAG made one node per package
and every node called its backend with `std::slice::from_ref`. Sixteen hand-written backends
loop where `generic` batches. The fix was a caller, not an implementation.

**And it is why the serialisation was invisible.** A per-manager mutex means all `apt` work is
sequential; combined with one process per package that is the worst of both — Shall neither
batched the manager's work nor overlapped it. Shall's own report said otherwise: six tasks under
a heading reading `Parallel Task Breakdown`, each claiming `12413ms`, out of a 12,465 ms run.
Six identical durations there is what a fully serialised run looks like when every task's timer
spans its wait for the mutex. **A user reading that output was told the opposite of what
happened**, which is why it survived unexamined. The durations are still identical, because now
they really were one command — and the line says so.

**The same shape, one layer down.** Eighteen backends answer `info(name)` by listing the whole
machine and finding one entry, and the callers ask once per *declared* package. Measured: a
read-only `check drift` on Ubuntu made exactly `declared + 1` `dpkg-query` calls; on Windows it
cost **~247 ms more per additional declaration**, because `winget list` takes over a second and
there is no cheaper question to ask it. A listing does not change while nothing is being
installed, so it is fetched once per manager per run and a mutating command is what forgets it.

---

**V.115a — Why Shall never asks what a package depends on.** *(2026-08-06, `Y9`. Rule in
II.7 and II.19.)*

V.115 says a wave splits on a dependency edge. It did not say where the edges came from, and
most of them came from Shall asking. The planner ran `get_dependencies` on every declared spec
— `apt-cache depends`, `dnf repoquery --requires --resolve`, `pacman -Si`, `brew deps`,
`xbps-query -x`, `snap info`, `flatpak info --show-metadata` — added every returned name to the
desired set as an install node of its own, then asked *those* nodes the same question.

**Three things were wrong with it, and only the third was ever reported.**

**It manufactured managed packages.** `sync/mod.rs` writes one `state.add` per install node, so
`apt:nginx` on one line took ownership of nginx's direct dependencies in `registry.json` — with
`source: None`, an origin no user could be shown. And *"what Shall may remove:
what it manages and you stopped declaring"* (II.7) then points straight at them. They survived
only by being re-derived identically on the next run: `direct_dependencies` drops a spec's
entry on any error, so **one failed `apt-cache depends` takes every one of those packages out of
the desired set at the same moment** and the next plan is a mass removal, stopped — if it is
stopped — by `max_removals` alone. `Queryable::tracks_manual` refuses a backend that cannot tell
a dependency from a choice, with the reason written beside it: *"gets a system's entire
dependency graph adopted and then purged."* The planner was writing the same rows behind that
refusal. `adopt` was fixed for this in 2026-07; the planner was not, because the fix was drawn
around `adopt`.

**It split the command line it had the best reason to keep.** The node wired an edge, and an
edge splits the wave — so the one case where Shall *knew* two declared packages were related is
the one case it refused to put on one `apt install`. V.115 measured that at 3,161 ms against
31,901 ms. `rebuild --backend apt` takes a backend's whole set down and puts it back up, which
maximises the number of such edges.

**It cost a subprocess per declared package**, plus one per discovered dependency, before any
install began — upstream of the fan-out, so the time was unrecoverable downstream.

**Measured, both sides, in the Arch integration image with `pacman` wrapped in a counting shim**
(`docker/integration/measure-batching.sh`; Y1's instrument, this question). Six declared
packages, five of them missing:

| | before | after |
|---|---|---|
| `pacman` invocations | **8** | **2** |
| of which `pacman -Si` (the dependency query) | **6** | **0** |
| child time, summed | 3.70 s | 1.20 s |
| wall clock | 1.58 s | 1.33 s |
| install commands / widest | 1 × 5 names | 1 × 5 names |

The wall clock moves least, and that is the honest reading: the six queries ran concurrently
(`--timings` reported 2.3× overlap), so they cost ~0.47 s of latency rather than 2.67 s — Rust's
fan-out was hiding most of the waste rather than avoiding it. **What the run does now is two
commands: ask pacman what is installed, then install the difference in one line.** There is no
third thing left to remove; `--timings` reports 2 waves, and the one quiet moment is the answer
to "what is already here", which has to land before anything can be planned.

**And the queries were buying nothing at all on this manager, literally.** `pacman -Si` prints
`Depends On      :` with six spaces; the parser stripped a five-space literal, so it matched
nothing and `pacman:` answered every dependency query with an empty list for the whole life of
the backend. Six subprocesses per sync to parse nothing, and nothing ever noticed, because the
only consumer installed whatever came back and was better off with nothing. The parser is fixed
in the same change — matched by key, with `pacman -Si jq`'s real output as the fixture — because
`shall info` now *shows* that answer to a person, and an empty one there is a lie rather than a
lucky escape.

**The buy was nothing.** Every manager here resolves and installs its own dependency closure at
install time; `apt install nginx` installs libfoo whether or not Shall mentions it, and `apt
install nginx libfoo` orders the two correctly on its own. `planner.rs`'s own recursion guard
said so — *"Every real package manager resolves and installs the full transitive closure itself
at install time, so Shall re-deriving it is redundant"* — directly above the code that re-derived
one level of it.

**And it had already been diagnosed, one backend at a time.** Every `ManagerConfig` in
`registry.rs` sets `depends_args: None` — 17 literals and zero `Some`, including the shared
`base_config` the rest are built from; zypper's carries the whole finding as a comment — *"zypper
resolves its own dependency closure at install time, so Shall re-deriving one adds nodes the
planner then tries to install by name"* — after `zypper info --requires` returned `Loading`,
`Reading` and `No`, and the first real zypper run in the project's history died on a `requires`
cycle between three adverbs. apt's had a dedicated test asserting apt returns nothing, whose
comment said it *"guards against the expansion being silently re-enabled"*. Every one of those
was drawn around the backend under review at the time, and seven hand-written backends — brew,
dnf, flatpak, pacman, snap, vscode, xbps — answered for real the whole time.

So the rule is at the caller, where one sentence covers all 23 `MetadataProvider`
implementations and the next one:
**planning never reads a `MetadataProvider`**, gated by
`tests/a_plan_installs_only_declarations_tests.rs`. Reporting one is untouched and is the
feature: `shall info <name>` prints dependencies and `shall why` searches them for reverse
dependencies.

**And the row itself now has to say where it came from** *(2026-08-06, with the ruling)*.
Banning the caller stops the expander; it does not stop the *next* thing that builds a spec by
hand from writing an unattributable row, and `sync/mod.rs` had two sites that would have — they
stored whatever `__source` held, `None` included, where `verbs/plan.rs` supplied a fallback.
Nothing reached them, because `model/resolve.rs` stamps `__source` on every resolved line; the
invariant was true and unenforced, which is a sentence in a document rather than a rule.
`ManagedPackage::source` is a `String` and `StateRegistry::add` takes a `&str`, so a row Shall
cannot attribute no longer compiles, and one already on disk is refused by `load_from` with the
`adopt` instruction rather than dropped — dropping it would unmanage a package that is still
installed, which is II.7's blind spot arriving from the other side. **A ledger of what Shall
will delete is a ledger that owes an answer to `why` for every line in it.**

**What could reverse it:** a manager that installs a declared package and *not* its
dependencies, leaving the closure to the caller. None of the 23 that answer does; a backend
that did would have to say so, and would need its dependencies declared rather than
discovered.

---

**V.116 — Why processes and sockets get different numbers.** *(Owner ruling, 2026-08-02 —
`Y2`. Rule in II.19.)*

`max_parallel` defaults to the core count, which is right for work that ends in a CPU. It was
also bounding pure network fan-out: `search`'s ~22 registry queries and the priority chain's
remote lookups. On a four-core laptop that ran the registries in **six sequential waves** — for
no reason but that the laptop has four cores, when nothing about waiting on a socket competes
for one. `search` measured 15.5s / 25.5s / 48.0s / 160.2s across four runs.

So there are two knobs and nothing reads a third. `network_parallel` defaults to 16: high enough
that a normal fan-out is one wave, low enough that a registry does not read it as abuse. Where
two fan-outs nest — every bare name at once, and within each name every candidate manager at
once — the cap is held by the leaf that actually talks to a registry, so the two multiply into
one number the user set rather than into their product.

**And the same distinction settles `upgrade`.** It was deliberately serial, recorded as *"it
changes packages, so concurrent sudo operations would interleave"*. That is true of the managers
that share a system package database and false of `cargo`, `npm`, `pipx`, `uv`, `yarn`, `pnpm`,
`vscode`, `emacs`, `krew` and `go` — which contend with nothing and are typically the slow ones,
because each rebuilds or refetches from a registry. A rule applied where its reason does not hold
is a rule that costs without buying. The root-needing set stays strictly sequential.

**A vars provider is a program the user wrote.** II.6b has said "resolved exactly once per
invocation" since it was written, and `HostFacts::with_vars` claims it in a comment. Measured, a
single `shall check` ran the user's `vars.sh` **three times** — so any side effect happened three
times and any `http()` variable was fetched three times over three fresh connections. That is not
a performance defect with a semantic side effect; it is a semantic defect that also cost 1.3
seconds.

---

**V.117 — Why every wait states its bound.** *(Owner ruling, 2026-08-02 — `Y3`. Rule in
II.19.)*

An unbounded wait makes a command's latency the *maximum* over everything it asks rather than the
median. `search` had no per-backend deadline, so one rate-limited GitHub call set the whole
runtime — which is the entire explanation for a command that measured anywhere between 15 and
160 seconds. `check health` had already solved this for its own probe, with a number and the
reasoning for it written down beside it; `search` had no equivalent.

The `@health=` port probe is the sharper case, because it decides whether to roll a sync back. A
*closed* localhost port refuses immediately, which is the common case and why this looked fine.
A **filtered** port — dropped rather than refused, which `apply/firewall.rs` can itself create —
waits out the OS connect default: ~21s on Windows, ~130s on Linux. A health check that decides
whether to revert must not be the thing that hangs.

**A bound is not always right, and where it is wrong it is stated too.** A download carries no
whole-request timeout: a release asset can legitimately take an hour, and a bound sized for an
API call turns a slow link into a corrupt install.

---

**V.118 — Why the restore point starts first.** *(Owner ruling, 2026-08-02 — `Y4`. Rule in
II.19.)*

Measured on Windows: `Checkpoint-Computer` **50.8s**, `Invoke-CimMethod CreateRestorePoint`
**53.3s**, and there is no faster API to swap to. Taken as a barrier that is a fixed ~51-second
tax on every install and every uninstall, in front of work that has to happen anyway.

The code's own comment already said the snapshot is *"a safety NET, not a precondition"* —
policies that genuinely require one gate on `has_provider()` upstream. **A safety net does not
have to be a barrier.** It starts before the read-only pre-flight (the drift event, the removal
guard's per-backend queries, two approval checks) and is joined immediately before the first
mutating command, which is the whole requirement: a snapshot taken after the change would revert
to the change. A refused sync aborts it, so a preview or a refusal leaves nothing half-taken.

**And it says it is happening.** Nothing in the output mentioned it, so a silent fifty-second
pause reads as a hang — which is how it was first reported, twice, and killed by hand both times.

*(Two smaller things on the same path. The snapshot provider's PowerShell ran with neither
`-NoProfile` nor `-NonInteractive`, so a user's profile was executed on every snapshot
operation; `psresource.rs` and `executor.rs` had passed `-NoProfile` all along and this was the
third of three. And the write-ahead journal is `journal.jsonl`, one JSON value per line: it used
to re-serialise the whole map, pretty printed, through a temp file and a rename, on **every**
state change — O(n²) bytes in the number of actions, under the one mutex every concurrent DAG
worker has to take. The more parallel the graph became, the more that cost.)*

**V.119 — Why Shall reports its own breakdown.** *(Owner ruling, 2026-08-03 — `Y5`. Rule in
II.19.)*

`latency.rs` measured the total and warned when a class crossed its budget. That is enough to
*notice* a 98-second `info` (E14) and not enough to *act* on one, because the next question is
always which manager took the time — and Shall could not answer it. The only method available
was to run each manager by hand outside Shall, time it, and subtract, which is how an afternoon
was spent establishing that a 3.2-second `list` is 2.35 seconds of `winget list` plus 0.8
seconds of everything else. `-vv` printed a running commentary with no durations in it.

**The ratio is the finding, not the list.** Every other rule in II.19 is a claim about
overlapping other people's processes, and none of them was checkable from outside. `list`
measured on this Windows box: **19.52s of child time inside a 3.15s wall clock — 6.2×**, with
`winget list` at 2.35s the floor. That single line says the parallelism is real, says what the
floor is, and says the floor belongs to Microsoft rather than to Shall. A breakdown printing
only a sorted list of durations would show the same seconds and settle none of it.

It is off unless asked for, because a measurement nobody requested is precisely the eager work
this round exists to delete; and it is on stderr, because `shall eval --timings | jq` must still
get JSON.

**Instrumented at the choke point** — `CommandExecutor::run_on`, which every manager invocation
funnels through — rather than per verb. A measurement each verb has to remember to take is the
shape `latency.rs` had already rejected for budgets, and it fails the same way: silently, in
whichever verb was written last. The one automatic probe that spawns outside that choke point is
instrumented at its own call site (`psresource`'s PowerShell cmdlet check), because a probe pass
that cost more than every command inside it would be the first thing a reader disbelieved.

**Interactive children are deliberately absent.** `shall shell`, the history pager, `bisect`'s
test command and `setup`'s installer are the user's own program running in the foreground; how
long somebody sat in their shell is not a fact about Shall, and a row claiming otherwise would
make the sum meaningless.

**V.120a — And why it only answers a command that just reports.** *(Rule in II.19.)*

A cached listing may inform a report; it may never source a decision that outlives the run. The
whole bargain of `installed_cache_secs` is that a stale answer costs you a stale *reading*, and
the next run corrects it. That bargain stops holding the moment the answer is written down: a
plan built from a listing taken before the user removed something by hand skips the install and
reports success — a declared package left absent with nothing saying so; `adopt` writes a
declaration for a package that is no longer there, and the next `sync` installs it back;
`plan` freezes that same mistake into a file `apply` runs later. So the disk layer serves
`list`, `search`, `check`, `outdated`, `info` and `why`, and nothing else. It is an allowlist
rather than a list of the unsafe ones, because the next command added to Shall should have to
say it is a reader — not discover that it was assumed to be one.

**V.120 — Why the cache is optional, and off.** *(Owner ruling, 2026-08-03 — `Y6`. Rule in
II.19.)*

`Y1`–`Y5` removed every question Shall asks twice and overlapped what was left. Measured with
`--timings` on this Windows box: `shall list` is **19.5 s of manager work inside a 3.2 s wall
clock, 6.2×**, and the slowest child is `winget list` at 2.35 s. There is nothing left to
overlap — the floor is a Microsoft binary. The only remaining way to go faster is **not to ask**,
and the next `shall list` asks all 24 managers the same question about a machine that, in the
ordinary case, nothing has touched since. With the cache on: **3.99 s → 0.68 s**, 24 child
commands down to one.

**So why is it off?** Because every other rule in II.19 buys speed with concurrency, and this one
buys it with correctness. A stale listing makes Shall wrong about the machine, and being wrong
about the machine is precisely how a declarative tool removes something it should not have.
`I-4` had already deleted a TTL'd cache once and recorded the right reason — process-lifetime is
the correct semantics for a one-shot CLI. That reasoning is still right *as a default*; what it
is not is a reason nobody may ever choose otherwise on a machine they know.

The bound on how wrong it can be is stated rather than hoped for. Shall drops the cache itself
on every mutation — in memory **and on disk**, because clearing the memo while the file survives
means the very next question re-reads the pre-mutation answer, which is the same
invalidation-on-one-of-two-doors shape this repo has now found three times. So it can only go
stale behind Shall's back: a `winget install` typed by hand, bounded by the TTL, bypassable with
`--no-cache`, and forgettable with `clean-cache`.

*(Two smaller properties, both load-bearing. A listing is written through a temp file and
renamed, because a half-flushed one read back is a **shorter** machine and a shorter machine is
a list of things to remove. And every read failure — corrupt, unreadable, a clock that moved
backwards — is a miss that asks the manager, never an error and never an empty machine.)*

**V.121 — Why a package name may be quoted.** *(Owner ruling, 2026-08-03 — `Y7`. Rule in
II.19.)*

V.113 says a name a manager reports has to be a name that manager can be given back. `winget
list` reports `ARP\Machine\X64\Mozilla Firefox`; `winget install` takes it; Shall could not
write it, because *a package name is one word*. `adopt` held such rows back and said so
honestly, and the honesty did not make the name declarable.

**The measurement corrected the diagnosis twice, and both corrections matter.** The backslashes
were never the problem — `2c51968` had already taught the grammar and the validator about them,
so `winget:ARP\Machine\X64\AndroidStudio` parsed all along. On this machine the undeclarable
names were **161: six winget names, every one containing a space, and 155 `service:` names that
are not a package-line question at all.** `docs/archive/GRADE-2026-07-31.md` §5 G-2 describes 185 backslash
names as unwritable; that defect is closed, and the number was re-cited afterwards without being
re-run. This is the second time in two rounds that a *count* outlived the bug it counted.

**Quoting rather than "everything after the colon".** The one-word rule is what makes II.2's
*an unrecognised line is an error* true: without it, VI.1's "any typo becomes a package named
after itself" comes straight back, and `@` stops working as the option separator on the most
common line in the language. Prose is not quoted. So `apt:this is just prose` is still an error,
`winget:"Mozilla Firefox"` is a name, and the two are told apart by something the user typed on
purpose rather than by a heuristic.

**One function spells the line.** `is_declarable` round-tripped `backend:name` while `adopt`
rendered `backend:name` by hand — the same question in two places. The day the grammar learned
to quote, the check would have said *yes, writable* and the writer would still have emitted the
unquoted form: a manifest that does not parse, produced by the command whose entire job is to
produce one that does. That is `2c51968`'s bug with the arrows reversed, and it is closed by
making the check ask the writer rather than by keeping the two in step.

**The lie is fixed; the question under it was the owner's, and V.124 answers it.** 155 of those
161 were `service:` lines, and `service:AppMgmt` parses perfectly. `is_declarable` accepted only
`Statement::Package`, so every service failed a test about **package** lines and was reported as
a name no line can hold — 155 sentences, none of them true of the name they described. The
grammar now answers three ways instead of two (`Declared::Package` / `Resource` / `Nothing`).

**V.124 — Why a service is adopted, and why nothing sweeps one.** *(Owner ruling, 2026-08-03 —
`Y7a`. Rules in II.19.)*

A `service:` line is not a package. It means *this service should be running*, and the two halves
are `install → enable + start` and `remove → stop + disable`. So a manifest holding 155 service
lines holds 155 triggers, and losing one in a bad merge disables a Windows service on the next
sync. That is the argument that kept them commented out, and it is a real cost.

**It is also the smaller cost, because the alternative was already worse.** `purge-undeclared`
does not read the manifest to find victims — it asks every manager what is installed and sweeps
what the model does not name. The service backend answers with every running service, so all 155
were already on that list. The only thing between the list and `sc stop` was `protection_of`'s
opening question, *could a package line ever have held this name?*, which for `service:AppMgmt`
is structurally no — a service line is not a package line. A refusal by coincidence, printing a
sentence that was false. **Correcting that sentence, which was the obvious tidy-up and is exactly
what a later reader would have done, would have handed the sweep every service on the machine.**
Declaring them is what removes the exposure rather than papering over it: a declared service is
managed, and `purge-undeclared` only sweeps what is not. The refusal is still written down —
V.124's second rule — because a service started *after* an adopt is unmanaged again, and that one
must be refused on purpose rather than by luck.

**The observed state, and not one bit more.** `actions_for(None, None)` is enable **and** start,
and on Windows enable is `sc config NAME start= auto`. Plenty of running services are demand- or
manual-started; a bare adopted line would have flipped every one of them to automatic-at-boot on
the first sync after a command whose entire promise is to describe the machine as it already is.
The init only ever reports *running* services — `sc query type= service` and systemd's
`--state=running` — so `status=running` is what was seen and the start type was never looked at.
`Queryable::adoption_options` is where a backend says what must be written beside a name for the
declaration to mean what was observed; it is empty for a package, because `apt:jq` already says
everything `apt` said.

**Asked of the backend, not of the name.** The guard's resource test consults
`Statement::RESOURCE_BACKENDS`, because a `setting:` is illegal as a line until it carries
`@value=` — so round-tripping the name alone would call a perfectly writable setting a name no
line can hold, which is the same false sentence one backend over. Two lists of the same three
prefixes is how one of them silently stops being a resource, so a test checks the constant against
`Statement::listed_as` in both directions.

**What this leaves.** Deleting an adopted `service:` line still stops *and* disables, which is
more than the inverse of a line that only declared `status=running`. That asymmetry predates this
ruling — it is what `ServiceInstallable::remove` has always done to a hand-written line — and it
is stated in the manifest header rather than quietly narrowed here.

**V.122 — Why every manager the run will ask is asked at once.** *(Rule in II.19.)*

`check drift` on a 298-package config took 9.1 s to do 2.3 s of critical path, and the reason
was not that anything was slow. Nine managers — gem, pip, emacs, luarocks, dotnet, dart, nimble,
bun, service — **started 5.4 seconds into the run**, and the run was idle for the second before
they did. Nothing had asked them yet. Two separate faults, both of the same shape:

- **The report asks each manager when its section needs it.** `check` plans drift, then crawls
  for unmanaged packages, then probes health. The crawl wants every manager on the machine and
  the plan wants nine, so fifteen managers waited out a plan that had no question for them —
  and every one of them was going to be asked before the command could finish.
- **The plan's fan-out is over *specs*, not managers.** A spec's answer usually comes from its
  manager's whole listing, so 256 winget declarations put 256 futures into a queue
  `max_parallel` slots wide, all waiting on one `winget list`, while scoop, choco and cargo sat
  unasked for want of a slot. Measured: three managers at 0.3 s, the other six at 1.9 s.

**A concurrency budget spent on duplicate questions is a budget spent on nothing.** Both fixes
are the same sentence — ask each manager once, at the start, for what the run is going to ask it
anyway — and neither adds a question: the memo already collapsed the duplicates, so what changed
is *when*, not *how many*. Measured after: every listing starts within 0.26 s of the first, wall
clock 9.13 s → 3.9 s, overlap 2.7× → 5.4×, and the report is identical line for line.

**Only for commands that already ask everyone.** `App::warm_installed` is called by name at the
two call sites that crawl the whole machine, never from `App::new`. A command that consults three
managers must not be made to wake twenty-four; that would be this same cost, moved to a different
run and charged to a user who asked for less.

**V.123 — Why the registry comes out in a predictable order.** *(Rule in II.19.)*

The backend registry was a `HashMap`, and Rust randomises hash iteration per process. So
`available()` and `all()` returned the managers in a different sequence on every run, and
everything downstream that walks them called the result an order:

- two `shall list` runs a second apart differed by **530 lines** and sorted to the same file, so
  the one thing a listing promises — that you can compare it to yesterday's — did not hold;
- the fan-outs handed their first slots to whichever managers the seed named first, so no timing
  measurement was reproducible and every wave was a different wave;
- anything taking the *first* backend that can answer was tossing a coin.

A map keyed by a name people read should come out in an order people can predict. It is a
`BTreeMap` now: alphabetical, stable, and asserted against a sorted copy rather than a recorded
list, so the test says *in an order somebody can predict* rather than pinning today's backends.

---

**V.125 — Why every answer to "where is the repo" must be absolute, and refused if it is not.**
*(Rule in II.1.)*

`shall --config-dir ./sandbox init` read `preferences.toml` **from the sandbox** and `modules/`,
`profiles/`, `active` and `priority` **from the real repo**. Not a race and not a subtle
ordering: `main.rs` set `config_root` to the raw flag, and `Config::config_root()` — the accessor
`Layout` is built from — discarded any path that was not absolute and fell back to
`safe_config_dir()`, which re-reads `$SHALL_CONFIG_DIR`.

So the flag that `--help` says *"outranks `$SHALL_CONFIG_DIR`"* **lost to it**, silently, and
`shall path` printed `./sandbox` while `shall init` scaffolded into the real platform config
directory. An inspector contradicting the enforcer is worse than no inspector, because it is
believed — `guard.rs:108` says exactly that about a different pair, and here it was the same
defect one door over.

**The fix already existed and was installed at one door out of four.** `shall path --set ./cfg`
had refused a relative path since it was written, with a message explaining why one is wrong.
`--config-dir`, `$SHALL_CONFIG_DIR` and `$SHALL_DATA_DIR` did not. One refusal function now, and
a test per door — because the interesting question was never "is this door right?" but "how many
doors are there?", and the answer was four when the code had been reviewed as though it were one.

Refused rather than corrected: resolving a relative path against the current directory would make
the same command mean different repos from different shells, which is the property that makes it
wrong in the first place. Refused rather than *ignored*, which is what it was doing.

**And why `--data-dir` exists at all.** Config had a first-class flag and state had an
undocumented environment variable, so `--config-dir <fresh sandbox> plan` planned **seven
removals** against the real machine's managed state and no flag could stop it. An isolation
affordance that isolates half a run is a trap rather than a feature: it is exactly convincing
enough to be used.

---

**V.125a — Why a plan that drops something names it.** *(Rule in II.10.)*

With `[guard] protected_packages = ["hello"]` and `hello` managed but declared nowhere:
`uninstall` deleted the manifest line and printed `already up to date`; `sync` printed
`already up to date`; `check` reported `the machine matches your files`. All three were false, all
three exited 0, and the state they left is **permanently wedged** — the manifest does not declare
it, the machine has it, the registry manages it, and every later `sync` drops it again for the
same silent reason. No command reported the disagreement.

The planner's protection check was a `debug!` and a `continue`, invisible at default verbosity.

**This repo had already written the rule down, about the identical situation.** From the entry
above on `rebuild`: *"The skips are printed: a rebuild that silently dropped half its scope would
report success over a machine it never repaired, which is the same lie convergence was already
telling."* `rebuild.rs` implements it and has a test called
`a_protected_package_is_dropped_and_reported`. The convergence path — the one that clause was
*about* — never received it.

**Dropped, not refused, and that half was right.** Making a protected drift removal a hard
refusal would mean one protected package undeclared on a machine stops every sync on it forever.
The defect was never the drop; it was that a user could not find out about it.

**One user-facing concept behaved three ways** before this: a config rule was a silent skip, an
OS-essential flag reached `guard::enforce` and became a loud refusal with a good message, and
only `shall protected` was correct. The skip now carries `Protection::reason()` — the guard's own
sentence — so the inspector, the refusal and the plan say the same thing about the same package.

The second drop site got the same treatment, and it is the reason this is a rule rather than a
patch: a managed package whose backend has left `priority` is also left alone, also correctly,
and was also silent.

---

**V.126 — Why nothing expensive is built at registration.** *(Rule in II.19.)*

`shall path` took **272 ms** against a 61 ms process-spawn baseline on the same host, and
`--timings` said `no child commands — this run asked no package manager anything`. All of it was
fixed overhead: **200.4 ms** of it was one `quanta::Clock` calibrating the TSC, once, inside a
`governor::RateLimiter` that `GithubBackendCore::new` built in its constructor — for a GitHub API
rate limit that an offline run, or a run with `github` absent from `priority`, never spends.

`create_default_registry` runs for every subcommand. So does every backend constructor in it.

**Two neighbours in the same directory already did it correctly**: `web.rs` and `appimage.rs`
build their HTTP clients inside the function that downloads, and their registrations measure
2.1 us and 5.9 us. `github.rs` was the odd one out, twice over — the rate limiter and the client
both. The fix is `OnceLock` on the type itself rather than on the call site, so the sibling in
`vscode.rs` could not be missed and a third caller cannot reintroduce it.

The clock went further than laziness: `governor` is built without its `quanta` feature, so the
calibration is not deferred but *gone* — the fastest quota here is 80 requests **per minute**, and
`std::time::Instant` resolves to nanoseconds.

**Why it survived every gate.** `latency.rs` budgets a whole command in *seconds*, which a fifth
of a second never crosses; every other instrument measures child processes, and this run spawned
none. The part of a run that asks nobody anything was the one part nothing measured. It has a
budget now, and the budget is what the rule is: the registry, for all 48 backends, in 120 ms.

**V.127 — Why `lock` and `unlock` name their axis, and why an upgrade re-records the pin.**
*(Z2, owner ruling 2026-08-03. Rule in II.6 and II.8.)*

**The bug was that the obvious undo did something else, and the something else uninstalled
software.** `lock` wrote `locks/versions.json` and approved every script the config can run;
`unlock` cleared `locks/bare.HOST.toml`, which records which *manager* an unpinned bare name
resolved to. Different files, unrelated jobs, one word apart. Someone who ran `lock`, changed
their mind and typed `unlock` did not undo the pin — they discarded the resolution, and the next
sync installed the package from a different manager and removed the old copy as drift. The help
text said so plainly. **A correct sentence in `--help` is not a design; the pairing is what people
read, and the pairing was a lie.**

**Reading the code to answer the report found a third ledger and two missing verbs.** There were
not two things called "the lock" but three — version pins, backend resolutions, and the approval
hashes in `locks/hooks.toml` that gate hooks, adapters, `exec:`, `generate:`, health-check
commands and the `vars` provider. Two of the three had no inverse at all: nothing could unpin a
version except a text editor, and nothing could withdraw an approval. **A list of what a word
means is an assertion about what is absent, and nothing verifies that half** — the same shape as
the eighth removal path in V.0.

**Why the axis is a positional and not six verbs.** Three ledgers × two directions is six names to
invent, remember, and keep from colliding with `hold`/`unhold` — which is a *different* question
(an exemption from `upgrade`, not a freeze) and which already owns the words a user would guess.
One grammar with the ledger named in it costs two verbs and reads as what it does. It also makes
the dangerous member of the family the one you have to spell: `unlock backends` is the only
command here that can move packages, and it now says "backends" out loud.

**Why a bare `unlock` still means all three, with no prompt.** The axis *is* the care. A
confirmation on the command whose entire job is releasing locks would be the asking that II.15
already rejects — the file is the switch, and typing the command is the decision. What was removed
instead is the accident: a bare name where the axis goes is refused, with the three axes listed,
rather than guessed at.

**And the second defect, which the report did not contain.** `locks/versions.json` was written by
exactly two things: `lock`, and `heal`. Not `sync`, not `upgrade`. So an upgrade moved a package
from 7.81.0 to 8.0.1, the pin still said 7.81.0, and the next ordinary sync — which converges to
the lock since U11 — read the old version back as `@version=`, found that an unadorned version is
an equality constraint, and planned the package straight back down. **The upgrade did not stick,
and nothing said so**, because each half was behaving correctly on its own. So every path that
deliberately moves a version forward now records where it landed.

**Why only pins that already exist are refreshed.** A package nobody pinned has no stale record to
fight; pinning it would make every `upgrade` a silent `lock` — a decision the user did not make,
found weeks later as a machine that quietly stopped tracking `latest`. The repair is exactly the
size of the defect.


**V.128 — Why a true-sounding success is a defect.** *(Owner ruling, 2026-08-03 — `Q28`. Rule in
II.20.)*

Two commands, one session, both exit 0, neither a crash:

| command | Shall said | what was true |
|---|---|---|
| `shall check` | `ok  drift  the machine matches your files` | **false** — a managed package nothing declared was left installed, forever (AU1) |
| `shall --config-dir X init` | `created`, `kept` | true about *what*, wrong about *where*: `--config-dir` was ignored and the scaffold landed in the live config directory |

Read the first row again, because it is the frightening one and it is easy to read past. The bug
underneath was that a package survived a removal — recoverable, visible, and the sort of thing a
test catches. **The damage was the sentence.** A tool that says your machine matches your files
when it does not has not merely failed to act; it has left you with a confident and wrong model
of your own computer, and taken away the reason you would ever go and check. The second row is
the same defect in a different costume: every word accurate, the one fact that mattered absent,
and a user who now believes their sandbox is a sandbox.

Neither instance was caught by a test, and the reason is structural rather than an oversight.
Tests assert what a command *did*. Both of these did the right thing and then described it
wrongly — and the output nobody asserts on is the boring one, the `already up to date`, the
`created`, the empty result. **Silence and success are the least-tested outputs in any tool and
the most confident things it ever says.** AU1 was a false `already up to date` and nothing but a
hand-run reproduction found it.

`Declined::reported` was the fix for the removal path, and its own comment explains the shape:
the type exists so that "does the user hear about this?" cannot be answered by omission, and a
new variant does not compile until it supplies its sentence. That is one path. The rule is what
stops the next one from having to be found the same way, by a grader running the original
reproduction rather than reading the report.

The reason this belongs in Part II rather than in a style guide: the best thing in this codebase
is already its error messages — file, line, what is wrong, what to do, *and what the concept
means*. That standard was never written down as a rule and it was applied only where something
went wrong. **The whole of II.20 is that existing standard, pointed at the paths that succeed.**

And the reason it is worth a rule at all, rather than good intentions: reproducibility answers a
question most people never ask. **Legibility answers the one they live with** — what accumulated
on this machine, what is safe to remove, what breaks if I touch it. A config a person can read
and recognise as a description of their own computer is worth more than one that can rebuild it,
and every sentence Shall prints either builds that recognition or quietly corrodes it.

---

**V.129 — Why the grammar stays open, and why a test pays for it.** *(Owner ruling, 2026-08-04 —
`Q29`, resource-kind half. Rule in II.2.)*

The proposal was to close the language: freeze the keyword list, declare the config **data**, and
route everything future through `generate:`. It was killed by the owner in one sentence — *"i
dont think it is closed, no. we still might add"* — and the sentence is right for a reason the
proposal did not reach. `generate:` output is merged *as if typed*, so it re-enters this same
grammar; a generator can emit a thousand computed `apt:` lines and **cannot emit a statement kind
that does not exist**. Generators expand quantity, never kind. Freezing the kinds would therefore
have closed the one door the escape hatch does not reopen, in exchange for a problem the freeze
was not actually solving.

Because what the freeze was solving is a *documentation* failure wearing a language-design
costume. Part II has now shipped four statement prefixes it failed to list: `exec:`, `dotfiles:`
and `firewall:` were caught after two days, and a paragraph was written into Part II recording
that they had been missed and instructing that the table "must be checked against" the code.
`generate:` then shipped, went unlisted, and sat **directly beneath that paragraph for months** —
read past by every session that read the warning, including the ones that quoted it.

That is the finding, and it generalises past this table: **a prose instruction to check a copy
against its authority is not a check.** It is a copy of the authority's *address*, and it decays
at the same rate as the copy it is supposed to protect — faster, because it reads as though the
work has been done. Four prefixes went missing under a rule that told people to look.

So the ratchet is the price of the open grammar and is cheaper than the ban: Part II's Statements
table and its reserved-word block are asserted against `KEYWORDS` in both directions, grouped by
`KeywordRole`. Both directions, because they fail differently — a word in the code and not the
docs is an undocumented feature, while a word in the docs and not the code sends a reader to
write a line the parser will refuse, which is worse. Grouped by role, because `KEYWORDS`
previously could not distinguish `use` (a directive this grammar has) from `if` (a word it
refuses so that `gem:if` cannot be installed by a typo); without that distinction, promoting a
foreign word into the language would have passed a check that only counted words.

The half that is **not** ruled: whether *computation* is closed — a fourth `vars` provider,
another logic keyword. It is a separate question with a different answer available, and it is not
implied by this one. It stays open in `decisions.md` rather than being quietly settled by the
ruling next to it.

---

**V.130 — Why a Windows mutation does not get the terminal.** *(Owner instruction, 2026-08-05 —
`Q35`. Rule in II.12c.)*

U40 gave stdin to mutations and to nothing else, and gave a reason: *"`sudo` asks for a password
on the terminal it was started from."* The reason is sound and it stops at the platform boundary.
`executor.rs` reads `if sudo && !cfg!(windows) && !Self::is_root()` — **`sudo` is never inserted
on Windows**, so no Windows mutation has that question to ask, while the shared terminal stayed
and could still be read from by whatever the manager decided to ask instead.

Measured on one host, the same install both times, with a fake manager that reads stdin:

| Shall's stdin | result |
|---|---|
| not a terminal | **48ms** — the child gets `Stdio::null`, reads EOF, and is done |
| a real console | **21.9s** — the whole bound elapsing; at the shipped 900, a fifteen-minute silence |

Fifteen minutes of nothing, ending in a failure that would have arrived in 48ms with the
manager's own prompt captured and printed. **A rule outlives its reason quietly**, which is why
the reason is written into the rule in II.12c rather than left here: the next reader sees that
the sharing is for `sudo`, and can check whether `sudo` is in the picture.

This was also proposed as the cause of an observed Windows stall and **was not** — the capture
showed the wedged process had no child at all (V.131). It is recorded because it is real and
measured, not because it explained anything.

---

**V.131 — Why the idle bound covers the read and not only the wait.** *(Owner instruction,
2026-08-05 — `Q32`. Rule in II.12c, beside V.114.)*

V.114's bound watches `child.wait()`. The read of the child's output sits outside it:

```rust
let status = match idle { ... };     // bounded; kills on silence
stdout: joined(out_task.await)?,     // no clock of any kind
```

The `out_task.abort()` that would end it exists only inside the timeout branch, which is
unreachable once `child.wait()` has returned. So a manager that hands its stdout to a background
process and exits leaves Shall reading toward an EOF that never arrives — and this one cannot be
fixed by killing anything, because **there is no child left to kill**. `kill_on_drop` has nothing
to drop; the DAG timeout is elsewhere; `command_idle_timeout_secs` has already been satisfied by
an exit that happened.

Found by photographing a wedged sweep instead of killing it: `shall -y install nimble:nimjson` sat
at **zero CPU with no children at all** while three orphaned `nim.exe` ran at `PPID 0`, outside
Shall's process tree. Reproduced deterministically with a fake manager that detaches — a 20s
bound, a child holding stdout for 60s, **64s wall**.

**And it exited 0 and reported the install a success**, timing the task at 60771ms. That is the
half worth the rule. A bound whose expiry is invisible is a bound that has been walked around; a
bound whose expiry is reported as success is Q28's class with the clock's own name on it. So the
same clock keeps running over the readers, on silence for the reason V.114 gives, and a pipe that
has produced nothing for the bound fails the command by name.

**What this deliberately does not do** is kill the orphan. That needs a Windows Job Object or a
Unix process group, it is platform-specific, and it changes what "kill" means for every command
in the program. It is a separate decision and is not smuggled in as a rider on this one.

---

**V.132 — Why the deploy refusal is asked before the download.** *(Owner instruction, 2026-08-05
— `Q37`. Rule in II.19.)*

`deploy_executable` refuses to overwrite a file Shall did not create, and refuses correctly. Its
test — `is_ours(dest, owned_root, recorded)` — reads only the **destination**. It needs zero
downloaded bytes.

It was asked after the download and after the unpack. Measured inside one `heal`, twice, back to
back:

```
 60.9s gap  ->  could not recover github:sharkdp/fd — refusing to deploy `fd.exe`:
119.1s gap  ->      ...\.local\bin\fd.exe already exists and Shall did not create it.
```

**180 of that run's 201 seconds were spent fetching a file it was always going to reject.** Two
things made it invisible rather than merely wasteful. It is an in-process `reqwest` download, so
it is not a child command and never appears in the `--timings` breakdown at all — which is why
the run showed 205s of wall against 33s of children. And downloads correctly have no whole-request
timeout, because a large download must not be capped by a wall clock — which leaves an
*avoidable* download both unbounded and silent. Three stalls were misdiagnosed as wedges because
of exactly this: zero CPU, no child process, nothing in the log.

**This is why reading does not find it.** Every line of `deploy_executable` is right. The defect
is the order it is called in, which is not visible anywhere inside the function — so the ordering
is held by a scan across the three backends rather than by review.

---

**V.133 — Why a resource already in its declared state is not work.** *(Owner instruction,
2026-08-05 — `Q39`, convergence half. Rule in II.19. The other half — whether `adopt` should take
150 services nobody chose — is still open.)*

`shall adopt` on a Windows host wrote 207 declarations, **150 of them `service:X@status=running`**
— every running service. The next `install` of anything then failed:

```
Error: `sc` failed (exit 1056): [SC] StartService FAILED 1056:
An instance of the service is already running.
```

Two separate faults, and the order matters because only the second one is obvious.

**Shall should not have run the command at all.** `in_effect` — the probe that decides whether a
declared resource needs applying — had arms for `link` and `shim` and fell through to `None` for
everything else. `None` means *unverifiable*, and unverifiable **places**. So every adopted
service was applied on the next sync whatever the machine looked like, and the init could have
answered in one listing the run already had in hand. Measured before and after, on the same host
and the same manifest: **150 resource(s) to place → 2**, and the two are real drift — `gpsvc` and
`smphost`, trigger-start services Windows had idled out in the twenty minutes since `adopt` ran.

**And when it does run the command, already-there is success.** Measured elevated on this host,
both verbs, so the constants in `init_providers.toml` are a reading and not a citation:

```
sc start Appinfo         -> rc=1056   [SC] StartService FAILED 1056: An instance of the
                                            service is already running.
sc stop  AarSvc_1032af   -> rc=1062   [SC] ControlService FAILED 1062: The service has not
                                            been started.
```

1056 is `ERROR_SERVICE_ALREADY_RUNNING` and 1062 `ERROR_SERVICE_NOT_ACTIVE`. For a converger both
are the goal, and neither appeared anywhere in the codebase; `for_manager` had no `"service"` arm at all, so the service backend ran on
`ExitPolicy::default()` with `benign_exits` empty. The codes are declared **per verb**, in the
init's own row, because each is an ordinary failure on the other verb — a stop that came back "already running" did not
stop anything. Writing the pair as one per-provider list is the shortcut that loses that, and
Windows' hand-written `restart = [stop, start]` row is what exposed it: spelled out, both halves
were labelled `restart` and neither could be told which code meant "already in that state". The
row was deleted; the derivation that produces the same two commands labels each with its own verb.

**A third code is not forgiven, and must not be.** Unelevated, both commands return **5** —
access denied — measured on this host before the elevated run above. That is a real failure:
nothing converged and Shall must not claim it did. It does mean an unelevated `adopt` on Windows
writes a manifest that cannot converge at all, which is one more argument for the half of `Q39`
that is still open.

**A third thing fell out of it.** `Extras::changes` short-circuited on "never applied" and placed
without probing, while `Dependents::apply` has never consulted the ledger at all — it skips
whatever the probe reports in effect. So `plan` promised 150 placements `sync` would not have
made, on a machine where the two had never disagreed loudly enough to notice. The probe runs first
in both now, and the ledger answers only the case the probe cannot: a resource nothing can be
asked about has been applied, or it has not, and only one of those is work.

---

**V.134 — Why a bare `adopt` does not take a machine's services.** *(Owner ruling, 2026-08-05 —
`Q39`, second half. Rule in II.9.)*

`adopt` is the command that hands a machine to Shall, and what it writes is a file the user is
then told to read, because *deleting a line from it undoes the thing*. So the file is a claim
about intent, and every line in it had better be one.

Measured on a Windows host, fully isolated config and state:

```
adopt              161 declarations, 150 of them service:<name>@status=running
```

**93% of it was every service Windows happened to be running.** Nobody chose those. Two of them
— `gpsvc` and `smphost` — had stopped again on their own twenty minutes later, because Windows
starts them on a trigger and stops them when idle, so those two lines asked Shall to keep
restarting something the OS deliberately shuts down.

**The rule this needed already existed and was already written down.** `manual_listings` refuses
a backend that cannot separate a user's choices from its dependency closure, and says why:
*"Adopting nothing costs the user a manual manifest entry; adopting a dependency graph costs
them their system."* The service backend answered `tracks_manual() == true` while its own
`manual_source()` read *"every service systemd reports as running (no init records which you
chose)"* — contradicting itself in its own words, one method apart. `adopted_unasked` is that
same question one step along: not *can you tell a dependency from a decision*, but *is being on
the machine evidence of a decision at all*.

**A default, not a refusal.** `shall adopt service` takes them. After the change, on the same
host:

```
shall adopt                          316 declarations, 0 services, and one line saying
                                     which backend was skipped and how to ask for it
shall adopt service                  149
shall adopt service --enabled-only   113
```

**And `--enabled-only` is honest rather than complete.** It reads the machine's own record of
what it starts at boot — `systemctl list-unit-files --state=enabled`, OpenRC's default runlevel,
`StartType -eq 'Automatic'` on Windows — in **one** command, because asking per service is a
process spawn each and there were 150 of them. It drops the 36 demand-start services, `smphost`
among them. It does **not** drop `gpsvc`, which Windows marks `Automatic` and stops anyway. That
is Windows disagreeing with itself, not the filter failing, and it is written down here rather
than left for the next person to discover: the filter narrows the guess, it does not make the
list a record of anybody's decision.

A backend that cannot answer the question at all is skipped and named. A filter that silently
falls back to everything is how you adopt 150 services believing you asked for 40.

---

**V.135 — Why recovery finishes interrupted work only, and why it runs on the engine.**
*(Owner ruling, 2026-08-05 — `Q33`. Rule in II.19.)*

Two halves, and the second is only reachable because of the first.

**`Failed` is not interrupted.** `get_incomplete_actions` returned `InProgress | Failed |
Abandoned`, and `record_start` mints a fresh id per attempt, so a declaration that fails on every
sync appended a new operation every time and none was ever purged: one sweep's journal held **22
operations for a single `scoop:shall-no-such-pkg-zzz`**, all 22 of which `heal` then reinstalled.

The argument that decides it is not that failures are hopeless — a mirror goes down, a network
drops, and those are worth another go. It is that **retrying them here is the same work twice.**
The package is not installed and its line is still in the manifest, so the `sync` that runs
immediately afterwards schedules it again. Recovery retrying it first buys nothing but a longer
wait and an error in a command nobody asked to install anything with.

And it compounded, which is what made it expensive rather than merely redundant. `needs_recovery`
asked a *different* question from `get_incomplete_actions` — `InProgress | Abandoned` — so an
interrupted entry that can never be recovered stays `InProgress` for ever, keeps `needs_recovery`
true for ever, and runs a full recovery of every failure the machine has ever recorded in front
of **every sync**. `watch --once` cost 208 seconds on this host doing exactly that. The trigger
and the work are one predicate now, because when they disagreed this is what the disagreement
bought.

`Failed` therefore becomes terminal and ages out on the same rule as `Completed`. Keeping it for
ever once nothing reads it would trade an unbounded retry for an unbounded file. `InProgress` is
still never purged at any age: it is the only record that something on this machine is half-done.

**And recovery is a graph like any other change.** It was a `for` loop with the install awaited
inside it and `install(std::slice::from_ref(spec))` at the bottom — serial, one package per
command, standing next to a batched parallel DAG and getting none of it. Measured on one host in
one minute:

```
sync --dry-run   2.65s wall ·  21 child command(s) summing to 10.35s · 3.9x overlap ·  2 wave(s)
heal           205.14s wall ·  27 child command(s) summing to 33.31s · 0.2x overlap · 27 wave(s)
```

**27 waves for 27 commands is the definition of serial.** The fix is not a `join_all` over the
same loop — that is the second copy of the engine getting a second copy of the engine's
features. The loop is deleted; recovery builds a graph and hands it to `Transaction`, and gets
per-manager batching, the parallelism cap, the retry policy and the rollback history for free.
Its dependency edges come from the journal's own specs, keyed `backend:name` exactly as
`ChangePlanner` keys them — the bare name would have matched nothing and produced an edgeless
graph, which is a plan that runs in the wrong order rather than one that fails.

**Two settings differ from a sync's, and both follow from what recovery is.** It does not roll
back: each entry is a separate piece of work a dead run left behind, and undoing one that
succeeded to punish one that failed moves the machine further from what was wanted. And it
continues past a failure — `continue_on_error`, off everywhere else — because one operation
nobody can finish must not leave the other twenty-nine unfinished. A node whose *dependency*
failed is still never attempted, and is reported as skipped naming the one that stopped it,
because "jq failed" about a package no command was ever run for is the misattribution V.136 is
about.

---

**V.136 — Why a failure names the declaration and not just the command.** *(Owner ruling,
2026-08-05 — `Q34`. Rule in II.19.)*

`install X` converges the whole configuration. That is not a bug to be fixed — it is what
declarative means, and the alternative (converge only X) turns Shall into a package manager that
happens to keep notes, where your files and your machine can disagree with no command noticing.

The consequence is real all the same: a line nobody has looked at can stop the install somebody
just typed. Measured:

```
$ shall -y install bun:sort-package-json
Error: `sc` failed (exit 1056): [SC] StartService FAILED 1056:
```

Nothing in that names a declaration, a file, or a reason the user should care about `sc`. The
transaction knew which node it was and threw it away one line before returning the error.

So the failure carries it: `` while applying `scoop:shall-no-such-pkg-zzz`
(modules/starter.txt:11) ``. Appended to the message and to nothing else — `retry` and
`absent_name` are what every caller downstream reads to decide whether to try again and whether
to withdraw the line, and a wrapper that stringifies the error into `Other` turns a withdrawable
line into a permanent wedge.

**And the half only the caller knows.** `install` compares what failed against what was asked
for and says outright when they differ. That check found a second defect while being written:
`WhyKept::NameAbsentElsewhere` — the branch whose *name* says the missing package belongs to some
other declaration — told the user *"`sync` will keep failing the same way until the line naming
it is corrected or removed with `shall unmanage bun:sort-package-json`"*. It pointed at the one
line that was fine. The withdrawal logic itself was already careful, and says why: *"Withdrawing
on a guess is the one outcome worse than keeping a line."* The advice beside it was not.

---

## `Q36` — adoption declares only what the manager can reinstall

**The bug.** `shall adopt` on Windows wrote 186 declarations naming packages that cannot be
installed. Not "were hard to install" — cannot. `winget list` merges two different things: what
winget installed from a catalogue, and every Add/Remove-Programs and MSIX entry it finds by
reading the registry. For the second kind it synthesises an identifier, and that identifier
exists only on that machine. Measured, on 280 installed rows:

```
$ winget show --id "ARP\Machine\X86\PHSP_27_2" --exact
No package found matching input criteria.
$ winget show --id "MSIX\AdobeAcrobatDCCoreApp_23.1.0.0_x64__pc75e8sa7ep4e" --exact
No package found matching input criteria.
$ winget show --id 7zip.7zip --exact
Found 7-Zip [7zip.7zip]
```

The split is exact and winget states it outright: **94 rows carry `Source: winget`, 186 carry no
source at all, and no row is on the wrong side of that line.** A blank source is winget saying it
found the entry by rummaging, not by matching a catalogue — and it only ever prints a synthesised
identifier when its own correlation to a real package has already failed.

**Why it looked fine for so long.** The grammar could not hold a backslash, so `adopt` refused
these names and wrote them as commented-out lines with the reason. A 2026-07-31 review recorded
that as *"good defence, and it is why G-2 is medium rather than high."* It was not defence; it was
an accident. `V.113` then fixed the grammar — correctly, because `winget uninstall` really does
take these names and refusing to type one was a real bug — and the accidental protection went
with it. Nobody replaced it with a deliberate one. **Being able to write a name down and deciding
to write it down are different decisions, and only the first was ever made.**

**Why not a filter.** The first proposal was to skip identifiers whose name carries a version,
on the theory that `MSIX\` rots and `ARP\` is stable. The machine refuted it: Adobe bakes the
version into the ARP key too — `ARP\Machine\X86\ILST_30_2_1`, `PHSP_27_2`, `LTRM_15_2` — so a
prefix rule keeps 119 entries that decay exactly like the 66 it drops. Recovering a real name by
searching the catalogue for the display name does not work either: of the 186, **176 have no
match at all, 7 are ambiguous, 3 resolve.** 1.6%.

**Why the export.** `winget export` is one call, and it is the manager's own answer to *what
could I put back*. It returns exactly the 78 distinct installable identities — verified against
the listing with no difference in either direction, the 94-to-78 gap being runtimes that winget
lists once per architecture and a manifest can only hold once. It also names every entry it is
skipping, in the user's language.

**The rule is not "winget is special".** It is that adoption's output is *declarations*, which
have to converge later, and a listing is not that. Any manager whose listing includes entries it
cannot reinstall needs its export, which is why the seam is `ManualListing::ExportFile` and not a
winget branch. What the version-bearing names did was make the failure visible the same day
rather than on rebuild day; the other 120 were equally unenforceable and perfectly quiet.

---

## `Q40`–`Q42` — a read that failed, and the three ways nobody noticed

**The bug, as it presented.** One integration test went red now and then under full-suite load:
`info winget:7zip.7zip` denied a row that `list` had printed a moment earlier. It passed in
isolation every time. That is a flake in the way a smoke alarm is a noise.

**What it actually was.** Sixteen concurrent `winget list`, with Shall nowhere near them:

```
N= 1   min 1165ms   median 1165ms   max 1165ms    0/1 failed
N= 8   min 2306ms   median 2503ms   max 2522ms    0/8
N=16   min  304ms   median 2313ms   max 2612ms    3/16   <-- rc=0x8A150001, 0 bytes out
```

Winget loses ~3 of a cold burst of 16 and none of the next 32; it is contention on its own
source index. Not our defect — but what Shall did with it was.

`run_output` ignored exit status by design, and the design is right: "no such package" and "no
results" are ordinary non-zero replies. It ignored the *silent* ones too. So `Ok("")` → a parser
finding nothing → `list_installed` answering `Ok(vec![])`. **Nothing in the chain believed
anything had failed.** Shall did not think winget was unwell; it thought the machine was empty:

```
round 1 : rows min=0 max=280   EMPTY_LISTINGS=1/16
        rc=0  ms=2285  rows=0   <-- `shall list --backend winget`, 280 packages installed
```

**Three layers, three chances to notice, three misses.** The executor turned a failure into an
empty string. The backend turned an empty string into an empty machine. And three callers turned
an empty machine into a claim: `info` printed *"is not installed on this machine"*, `list`
dropped the manager's rows without a word, and `hook-reconcile` recorded nothing as though there
had been nothing. Each layer was individually defensible and the composition was a lie.

**Why the retry classifier could not save it.** `ExitPolicy` classifies from *text* — transient
markers, permanent markers, absent markers, all matched against a haystack of both streams. This
failure writes zero bytes. The haystack is empty, every list misses, and the verdict is
`Unknown`. The one signal that existed — the exit code — was read by nothing but `is_benign`.
**A classifier looking at the only axis the failure does not use.**

**Why the bound could not save it either, and was wrong anyway.** The first theory was that the
900s idle bound was killing a wedged `winget list`. The measurement refuted it: these fail in
~310ms. But it exposed a real fault beside the imagined one — 900 was chosen for
`Checkpoint-Computer`, a mutation silent for its whole run, and every read inherited it. A
question that takes 1.5s had fifteen minutes of rope.

**The shape of the fix.** Narrow where it must be, general where it can be. A non-zero read with
output keeps its output — breaking that would break every manager that reports "not found" by
exiting 1. A non-zero read with *nothing* is a failure, because no manager expresses "you have
none of these" by saying nothing and failing. Classification gained the code as a fallback under
the text, never over it: a manager that named its problem has described it better than a number
can. Retry is for reads alone — idempotence is the entire justification, and a mutation retried
on a guess installs twice.

**And one sibling that was already right.** `planner::installed_sets` drops a backend it could
not query from its map, and `is_installed` reads a missing entry as *assume it is there* — so a
removal is still scheduled and reports its own failure. Its comment says why: *"Not knowing must
never turn into 'so skip it'."* The same question, asked and answered correctly, two files from
where it was being answered wrongly.

---

## `Q44`–`Q45` — asking N times what the manager answers once

**The measurement that started it.** `shall list --outdated`, on the same host, in the same
minute as the listing that feeds it:

```
shall list --outdated : 771.4s
shall list            :   2.9s
```

Thirteen minutes. And the loop that spent them was not slow — it was asking the wrong question.
`compute_outdated` walked the installed set calling `Searchable::lookup(name)`, and `lookup`
**defaults to a whole `search` for that one name**. So a machine with 280 packages ran 280
registry searches to answer a question every one of those managers will answer in a single
command: `apt list --upgradable`, `pacman -Qu`, `winget upgrade`, `npm outdated -g --json`.

Batching it is 771.4s to **25.6s**. The remaining 25 seconds are the managers with no such verb,
still asked per package but now concurrently instead of one after another — `cargo` has no
outdated check at all, and that is a fact about cargo worth stating rather than hiding.

**Two distinctions the fix had to keep.** `None` from `outdated_all` means *this manager cannot
be asked*; `Some(vec![])` means *it was asked and nothing is stale*. Collapsing them would mark
a manager's entire set current the moment its verb went missing — the same shape as `Q40`, where
a failed listing became an empty machine. And where the manager does answer, Shall does **not**
re-compare the versions: the manager already decided, and a second opinion from a version grammar
it does not use is how `> 3.13.5`, which is genuinely what winget prints for `Python.Launcher`,
turns into a wrong answer.

**Then the same question one layer down.** If a manager answers about many packages at once, it
probably *acts* on many at once too. Five hand-written backends were running one command per
package where the tool takes a list — `brew` under `run_exclusive`, so N packages meant N
dependency resolutions **and** N serialised lock acquisitions. The generic backend had batched
correctly all along; these predate it and never picked it up.

**The sweep was wrong the first time, and that is the useful part.** It reported thirteen
backends, `dnf` and `pacman` among them, and built a story about hand-written backends drifting
from the generic one. The detector matched a `for` loop followed *anywhere in the function* by a
`run()` call. dnf's loop is:

```rust
for name in &names { args.push(name) }   // builds the batched argv
```

A loop that spawns per item and a loop that assembles one command are indistinguishable to a
grep. Re-run with brace matching so the invocation has to sit inside the loop's own body, the
count fell from thirteen to five. **A finding that names the wrong files is worse than no
finding**, because the next person spends their afternoon confirming it.

**And the evidence bar moved mid-task.** These five do not exist on the Windows host, so the
plan was argv-shape tests and an honest note that nothing had actually been run. WSL Docker made
that unnecessary for three of them, and one of those changed a decision: nix's removal was going
to be left alone entirely, because its per-item loop carries a comment about positional indices
renumbering under a batched call. Real nix 2.x settled it — `nix profile remove hello ripgrep`
reported `removed 2 packages, kept 17 packages` and left `jq` alone, and modern `nix profile
list` shows no indices at all. So the by-name path batches on evidence, and the indexed path
keeps its careful ordering because no nix that still reports indices was there to test.
`vscode` and `snap` stayed argv-only, and the register says so rather than letting them borrow
the confidence of the three that were run.

---

**V.137 — Why `adopt` declares OS-essential packages instead of commenting them out.**
*(Owner ruling, 2026-08-05 — `Q47`. Rule in II.9.)*

The manifest used to carry OS-essential packages in a commented-out second section, and the
reason written beside them was that a live line is *"a line whose deletion means uninstall"*.
That reason was already false when it was written. `guard::protection_of` refuses to remove
anything a backend reports as essential, whatever the manifest says; the only way past it is an
explicit `unprotected_packages` entry, which is a sentence somebody types on purpose. So the
comment character was guarding against a deletion that could not happen.

**What it cost to keep it was real.** A commented line is not a declaration, and Shall has no
opinion about a package nothing declares. On the measured host that was 33 packages — the ones
that keep the machine bootable and logged in — sitting outside the model entirely. If one of
them was uninstalled behind Shall's back, `check drift` did not notice, `sync` did not put it
back, and `heal` had nothing to repair. **The packages given the least protection by the model
were the ones the machine could least afford to lose**, and the mechanism that did it was
filed as a safety feature.

This is the same shape as E7 one layer out: protection meant *never remove*, and it had quietly
grown a second meaning, *never adopt*. E7 removed that ambiguity for `protected_packages` and
left it standing for OS-essential — the twin branch in the same `if`, four lines down. The
manifest header now names the exception instead of the comment character: a guarded line is
declared, Shall keeps it, and deleting the line stops Shall keeping it without uninstalling
anything.

---

**V.138 — Why the command that deletes is named `purge-undeclared`.** *(Owner ruling,
2026-08-05 — `Q31`. Rule in II.11.)*

`unmanaged` named two different numbers on two screens of the same program, in the same minute:

```
shall check           ->  ok  unmanaged   everything you chose is managed
shall check drift     ->  ? unmanaged - installed but not in your manifests (34):
shall check unmanaged ->  1 package(s) `shall adopt` would take
```

Neither number was wrong. E6 had already ruled which question the *word* answers — what `adopt`
would take — and the fix reached `check unmanaged` and the rollup, but not `check drift`, not
the readme, and not the command name. **So the most destructive verb in the program was named
after the set it does not act on.** A reader who saw `1 unmanaged` and typed `purge-unmanaged`
was reaching for a one-package cleanup and pointing a 34-package delete at their own OS.

The word was not the fixable half. Both sets are real and each has a command that acts on it, so
one word for both was always going to mislead somebody; the only question was which one got a
new name. The verb did, because a verb is named after what it does, and what it deletes is the
undeclared set.

**The near-miss worth recording:** `Q47` shrinks the gap — with essentials adopted, most of
those 34 become declared — but shrinking a gap is not closing it. A backend that cannot separate
a choice from a dependency still produces two different numbers, and the rename is what keeps
that a definition rather than a surprise.

---

## `F-2` — the gate is drawn around the artifact, and the property escapes through the next copy

**Eight grade rounds named "a check that cannot fail" as this repository's signature defect.
Rounds 2, 7 and 8 name it in nearly identical words. None of them says *why it keeps coming
back*, and a ninth sighting would have been worth nothing.** This is the mechanism.

The gates here are good. `removal_guard_enumeration_tests.rs` scans all of `src/` and fails the
build when a removal appears without a named guard, then self-tests the instrument before
trusting it. `argv_drift_tests.rs` walks every subcommand Shall invokes against the real
manager's `--help`. `help_map_tests.rs` compares the map in `args.rs` to `--help` in both
directions, and its own header cites `undo` — a command that sat in two exemption lists for
months after it was renamed — as the reason it exists.

Each one is scoped to the file that was open when it was written. So on 2026-08-05, with no
top-level `status`, `doctor`, `undo` or `audit` verb anywhere in the program:

- **`app/fleet.rs` asked every host for `shall status --json`.** `shall fleet` could not return
  "in sync" for a correctly installed machine — every host answered "unrecognized subcommand"
  with exit 2 and every row read ERROR. 265 lines of a command that had never once worked.
- **`scripts/install.sh` and `install.ps1` ran `doctor`** to vouch for the binary they had just
  built, and signed off with *"Try `shall status` or `shall doctor`"*. The first thing a new user
  runs, and the health check that certifies the install.
- **`verbs/cleanup.rs` printed `Undo with 'shall undo <id>'`** after `purge-undeclared`, the most
  destructive command in the program.
- **`cli/args.rs` documented `upgrade --security` as upgrading what `shall audit` reports** —
  inside the very file `help_map_tests.rs` gates. The gate compares the *map* to `--help`; a
  dead command in a flag's help text is a different copy of the fact, sitting four hundred lines
  away in the same file.
- **`app/apply/dotfiles.rs`** told a non-interactive caller to run `shall status`, and
  **`backends/init_providers.toml`** explained a `--no-pager` flag by a hang in `shall status`.
- **`readme.md`'s verb tables listed `status`, `unmanaged`, `absent`, `conflicts`, `doctor` and
  `audit`** — six rows across two tables — thirty lines after the file correctly explains that
  `--help` cannot go stale the way a README can.

One fact. Six copies. One gate, around one copy.

**So the gate moved to the property.** `tests/named_commands_exist_tests.rs` reads clap's command
tree — names, aliases, nesting — and asserts that every `shall <verb>` in any file a user reads
or a machine runs walks that tree. It found all six of the above, plus two nobody had named.

**The convention it rests on, and the reason it is exact.** The false-positive problem is prose:
*the shall binary*, *this shall speaks schema 2*. The obvious fix is a list of English words to
ignore — which is one more hand-maintained list beside the program, rotting on the same schedule
as the ones that caused this. Instead: **prose calls the product `Shall`.** A lowercase `shall`
at command position — opening a line, or after a quote, a backtick or a shell operator — is an
invocation. Nine prose strings were respelled to obey it, and the tree now has no exemption list
at all. The scanner skips exactly one file, `tests/named_commands_exist_tests.rs` itself, because
a gate that asserts a string is absent must spell the string out; it is skipped by `file!()`
rather than by a path literal, and a test asserts the file would otherwise have been read.

**Why `docs/` is checked against a weaker property, and checked at all.** *(Owner ruling,
2026-08-08 — `Y18`.)* The first version of this gate left `docs/` out on the grounds that a
record must stay free to name a command on the day it was deleted, which is true and is not a
reason to leave 2.5 MB unread. The clause that makes both work: in `docs/`, a dead command has
to be one II.17's Deleted register records as dead. A name that is neither live nor registered
is a finding. Pointed there, 62 raw hits reduced to three, and all three were Part II itself —
a sync nudge naming `shall clean` where the verb is `remove-orphans`, an `adopt` header naming
`shall forget` where the code already wrote `shall unmanage`, and `shim`, deleted by II.16 and
never entered in II.17. The register being incomplete is what let a **verified** bug in
`bugs.md` go on describing a command nobody could run. **A weaker property that runs beats a
strict one scoped to the files that happened to be open.**

**Two findings in the same report were checked and one of them was wrong.** `F-2` reads
`harness-logic-test.sh:553`'s `install.*` exemption as excusing the install scripts from
subcommand validation, and calls it *"the argument for including it, written down as the reason
for excluding it"*. It is not: that exemption belongs to a different check — "every script in
`scripts/` is run by something" — and `install.*` genuinely is not a gate. The real gap is that
the harness's subcommand check only ever looked at the two container scripts named in `SOURCES`,
so `install.sh` was never in its scope to be exempted from. The Rust gate covers it regardless,
which is the point of scoping to the property: it does not have to be told which files matter.

**And what it cost to fix `fleet` properly.** Renaming the string was not enough. `shall check
--json` emitted `{section, ok, summary, next}`, and every number it reported — how many to
install, how many to remove, how many unmanaged — existed only inside the English of `summary`.
A consumer wanting a count had to regex `"3 to install, 1 to remove"`, which makes the wording of
a sentence an API. Every section now carries a `counts` object beside its sentence, always
present and always including its zeroes, so that "the key is missing" and "the count is nought"
cannot be confused; `fleet` reads those, and reads the drift section's own `ok` for the verdict —
which is wider than the two package counts, because a machine whose packages match and whose
`link:` tree does not has drifted.

**And under that, a family worth naming.** `fleet` reads that output over SSH, so it has to *be*
a document, and two verbs broke that promise on the branch a **healthy** machine takes. `sync
--dry-run --json` emitted its report inside the dry-run block, below the "nothing to do" exit, so
a converged machine answered "is this in sync?" with the words `already up to date`. And
`Adopter::discover` — which the `unmanaged` section calls — printed `Note: your modules did not
resolve …` to stdout, so a machine with a broken config returned something unparseable. **A
`--json` flag gets exercised on the busy path where there is obviously something to print; the
empty case is the one nobody looks at, and it is the one a converged fleet is made of.** Both are
fixed and pinned by `tests/json_output_is_a_document_tests.rs`, which drives the real binary and
carries a busy-path control so the empty-case tests are known to be about the empty case. There
is still no general gate that every `--json` verb emits only a document — four cases are pinned,
the property is not.

**The sibling in the same family.** `scripts/decision-count.sh` gates the register's own counts
and printed `unrecognised 2` before exiting 0, because the unreadable bucket was never added to
the failure count. Two entries carried statuses the counter had never learned — `DEFERRED` and
`HALF RULED` — so every total it verified was computed over 164 of 166 entries. Three of its six
buckets were cross-checked against the docs and three were not, and the two files each broke the
register down as `160 ANSWERED, 2 PARKED, 1 BUILT NEVER RULED, 1 OPEN`: four correct figures
summing to 164 beside a total of 166 that this same script had verified. **An omitted bucket
states no wrong number anywhere**, so every per-figure check passed it. A breakdown is a claim
about the whole register, and it is now checked as one.

---

**V.139 — Why a dotfiles tree is the `link:` lines it stands for, and not a loop of its own.**
*(2026-08-06, `Y10`. Rule in II.2's `link:`/T6 section. Raised by
`lamdan/whole-repo-2026-08-05.md` as F-0.)*

`link:` earned a whole lifecycle over three rulings. `T6` says a line that replaces a file you
wrote backs it up to `<dest>.shall-backup` first and puts it back when the line goes away. The
key is the destination and never the source, because a teardown handed the source deleted the
file in the user's own dotfiles repo and left the deployed copy standing. A copy counts as
"already in effect" as much as a symlink does, because Windows falls back to copying and asking
only `read_link` made every sync back up its own copy under a summary reading
`already up to date`.

`dotfiles:` is the same statement said once for forty files. `verbs/sync.rs` calls it *"a pile
of `link:` lines"* and applies it in the same phase. It had **none** of the three. Its apply was
sixteen lines of its own: create the parent, `remove_file` the destination, symlink. So:

- **`--replace-existing` threw the original away.** The flag waives the refusal to overwrite; it
  has never meant "and destroy what was there". The identical `link:` line on the same run
  preserved its own file. One user, one sync, two statements, opposite outcomes.
- **Nothing recorded what the tree placed**, so the shared teardown could not see it. Deleting a
  file from your tree left a **dangling symlink** on the machine for ever; deleting the
  `dotfiles:` line left the whole tree. That is `S20`'s bug — *deleting a line leaves the thing
  in effect for ever* — still live for one statement kind, eleven days after `S20` was closed.
- **No ledger row meant no removal guard.** A path that deletes files was outside the guard, and
  outside `max_removals`, which is a ceiling over the whole plan precisely so that "three
  packages and three links" is six removals and not two budgets.
- **And the tree re-placed every file on every sync**, because it never asked whether the
  destination was already right. That is the exact defect
  `tests/grade3_resource_idempotency_tests.rs` was written for, surviving in the one statement
  kind that test did not cover.

**The mechanism that produced all four is one thing: a second implementation.** Not one of them
is a hard problem; each is a rule `link:` already holds and the tree's private loop had no way to
inherit. So the fix is not four fixes. The tree expands into the `link:` lines it stands for —
`Dotfiles::links`, one place — and from there there is nothing tree-shaped left to get wrong.
~40 lines added, one loop deleted, four behaviours gained, and `spec_from_extra` converts a
tree's file and a hand-written line into the same value so they cannot drift apart again.

**The part that should be uncomfortable.** Four documents said the ledger row existed:
`model/dotfiles.rs`'s header (*"one ledger row per file, and that is the cost worth paying"*),
`core/extras_lock.rs` (*"its files ARE keyed here … one ledger row per placed file"*),
`history.md`'s 7n entry, and `plan.md`'s 7n — marked **DONE 2026-07-24**, with the exit condition
*"a file deleted from the tree has its link removed by the same `extras_lock` teardown every
other extra uses."* Every one of those describes the design correctly. None of them was true.
The row was designed, documented four times, ruled, marked built, and never written — and
because the tree's own loop worked for the case anybody tests by hand (place files on a fresh
machine), nothing disagreed with the documents for eleven days. **A stated exit condition is not
a test.** This one is now: `tests/dotfiles_tree_is_a_pile_of_links_tests.rs` runs `dotfiles:` and
`link:` against the same bytes and asserts they answer the same, with the `link:` half as the
control, so the two cannot silently diverge again in either direction.

**And the sibling underneath.** `Dotfiles::plan` answered *did Shall put this here?* with
`is_symlink`. `link:` had already learned that is wrong — a file it placed via the copy fallback
is not a symlink — and the tree's copy of the question never heard. It surfaced the moment the
tree started using the backend: Shall called its own copy a destination it did not create and
refused, by name, to touch the tree. Ownership is now what the ledger recorded, in union with the
old test so that a tree placed before the row existed does not become a fresh `U23` refusal on
the sync after an upgrade.

---

**V.140 — Why the write-ahead log covers packages and scripts, and deliberately not resources.**
*(2026-08-06, `Y10`. Rule in II.19. Raised by `lamdan/whole-repo-2026-08-05.md` as F-0.)*

`readme.md` said *"a write-ahead log records every mutation before it runs."* `JournalAction`
had two variants, both packages, and all nine `apply/` modules referenced the journal **zero**
times. Under the 2026-08-05 ruling that *everything is the product*, that sentence was false for
the majority of what Shall converges.

The review's proposed fix was one variant per phase, and its own steelman refutes it. **A
mutation needs a durable record exactly when the next run cannot recompute it.** A `service:`, a
`setting:`, a `firewall:` rule, a placed `link:` is a read-then-write converge from a
declaration: killed halfway, the next sync reads the machine, sees the line unmet, and finishes
the job. That is not a *worse* recovery than replaying a log, it is a **better** one — it also
corrects drift that happened while the process was dead, which no log could have recorded. Nine
new variants would have bought a slower sync and a bigger file, and would have moved the
authority for "what is true about this machine" from the machine to a log. They stay out, and
the rule says so, so that adding one has to argue with the reason rather than slip past it.

Two mutations are not converges. `exec:` runs code and `@undo=` runs an arbitrary shell command.
Nothing records how far either got, their authors never promised they were safe to run twice,
and there is no declared end state to converge towards. Those get entries.

**What recovery can do with one, and what it must not.** It must not replay it: a package is
finished by installing it again because reaching a state twice is reaching it once, and a script
has no such property — re-running it repeats the half that already ran. So `heal` reports it, by
name, with its content hash and the sentence a user can act on: *the next sync will run it again
from the top; if that script is not safe to run twice, this is the moment to check.* Before
this, a machine killed mid-`exec:` came back **silent** and re-ran the script on the next sync.
Reporting is a smaller thing than repairing, and it is the whole of what is honestly available.

**Then the entry is resolved as a FAILURE**, which is not bookkeeping. Not a success: the
script did not finish, and a log recording `Completed` for an interrupted mutation is the same
dishonest record this whole entry is about. `Failed` is terminal, so recovery stops asking, and
it ages out on the rule every other terminal entry ages out on while carrying the reason. `Q33` measured what an unresolvable
`InProgress` entry costs: it keeps `needs_recovery` true for ever, so every `sync` runs a full
recovery in front of itself — 208 seconds of one `watch --once`. An entry recovery can never
finish is exactly that shape, so "reported" has to be a terminal state.

**One correction to the finding.** It named `apply/extras.rs`'s teardown as the third
irreversible phase. It is not. `reconcile` computes drift from a ledger it writes only after the
loop, so a kill mid-teardown leaves the ledger naming the same drift and the next sync retries
it — a converge, like the rest. Checked, cleared, and recorded here rather than fixed, because a
sibling that turns out not to be one is worth as much as one that is.

**How the shape is held.** `heal` used to match on `JournalAction` in six places, each for its
own reason. Adding a variant that is not package work to that shape would have been six chances
to route a script down a package path. There is now one function — `replay_of` — that turns the
log's vocabulary into the engine's, and past it `heal` speaks `GraphAction` and can only say
things about packages. The write-ahead half is pinned by a test whose instrument is the mutation
itself: **the script under test reads the journal while it is running.** An entry recorded after
the interpreter returns leaves it nothing to find, which is precisely the difference between a
write-ahead log and a write-behind one, and the only witness that can tell them apart.

---

## `Q48` — the drive check that answered "different drive" for every path on earth

**V.141.** `link:` on Windows deployed a copy, never a link — on a machine with one drive, under
a warning reading *"Cross-drive fallback to COPY"*.

`is_same_drive` compared `source.canonicalize()` against the raw target. `canonicalize` returns a
verbatim path, whose prefix Rust models as `VerbatimDisk('C')`; the target's is `Disk('C')`. Same
drive, two spellings, and the comparison was of the spelling. Measured rather than reasoned:

    verbatim: VerbatimDisk(67)
    plain:    Disk(67)
    same_drive = false

67 is `C` in both. So the guard was not merely wrong at the margin — it was wrong for every path
on every machine, and `link:`, the feature whose entire purpose is *one file with two names*,
quietly produced two files that drift apart the moment either is edited.

**The check should not have been repaired, because the thing it checked does not exist.** A
Windows symlink is a reparse point holding the destination as a *string*, resolved when the link
is opened; it crosses volumes fine. It is the *hard* link that cannot. Verified before deleting:
a second drive letter via `subst`, then `symlink_file` from `C:` to `X:`, unelevated — created,
resolved, and read through. So repairing the prefix comparison would have preserved a fallback
guarding against nothing, and still copied for the cross-drive case symlinks handle.

**What does vary is the privilege**, and it is now the only thing branched on.
`ERROR_PRIVILEGE_NOT_HELD` (1314) — and no other error — falls back to a copy; everything else
propagates, so a genuine failure is no longer laundered into a silent copy. The warning names the
privilege, the remedy, and the consequence, because *"fell back to a copy"* is not something a
reader can act on, and the consequence is the one the user meets later: edits stop propagating.

**Why this was not shipped when it was found.** It reached `decisions.md` as `Q48` and sat there,
correctly: turning copies into symlinks can fail a sync that works today, which is behaviour a
user would notice. What was fixed at the time was the ownership predicate — a copy Shall made is
now recognised as Shall's — which is what kept a run from backing up its own copy on every sync
under a summary reading `already up to date`. That made the bug wasteful instead of latent, and
bought the time to have it ruled rather than guessed.

---

**V.142 — Why the second path is always where the safety falls off, and what makes it stop.**

`lamdan/whole-repo-2026-08-05.md` filed F-5 as *"two paths for everything, and the second path
is where the safety falls off."* The finding was accurate and it was incomplete in a way that
matters: it named the direction it had happened to look in.

The mechanism first, because it is not the thing the finding names. `registry.rs` carries an
**argv table** — every backend driven against a mock, the argv it would really have run
recorded, checked on every platform's CI so a typo in `mas`'s verbs is visible on a machine
with no Mac. It is one of the better instruments in this tree. And it recorded

```
Runs("apt install -y -- jq"),
…
Runs("dnf install -y jq"),
Runs("pacman -S --noconfirm --needed jq"),
```

in the same list, in a passing test, for as long as all three existed. **Nothing ever asked
`core::argv` whether the recorded argv was right.** That file is the one place that knows
whether a binary ends its options at `--`; it says `dnf` and `pacman` both do, on the strength
of the parser they link against. The gate recorded the defect *as the expectation*, which is
this repo's signature failure — a check drawn around the artifact under review rather than
around the property — one level below where II.21 had just fixed it.

**Why those two and no others.** Every backend on the data path gets the terminator from
`argv::push_names`, which `generic.rs` calls on the one line every install goes through. Every
hand-written backend called `push_names` too — `brew`, `conda`, `flatpak`, `go`, `link`, `mise`,
`nix`, `snap`, `vscode`, `xbps` all do — except `dnf.rs` and `pacman.rs`, which built a
`Vec<&str>` and pushed names onto it. Twelve of fourteen remembered. The two that forgot are the
two that run as root, which is not a coincidence so much as the point: a rule that has to be
remembered is a rule that will be forgotten somewhere, and where it gets forgotten is not
correlated with where it matters least.

**The finding's frame was one-directional, and the traffic runs both ways.** F-5 read the split
as *data path good, hand-written path lossy*. Scoping the conversions turned up two defects of
the same class running the other way, both live:

- **`clean_cache` existed only on the hand-written path.** `ManagerConfig` had no field for it
  and `GenericUpgradable` did not implement the method, so all forty data backends answered
  `Unsupported`; `handle_clean_cache` filters that out silently and prints *"No backend on this
  machine has a cache to clear."* That sentence was false on every Debian, Alpine, SUSE and
  Node machine Shall has ever run on. Six hand-written modules had the verb. The shared
  machinery — the good path — could not express it at all.
- **The exclusive lock keyed on the program, not the manager.** `run_exclusive(binary(), …)` in
  `generic.rs` against `run_exclusive("xbps", …)` in every hand-written module. OpenBSD installs
  with `pkg_add` and removes with `pkg_delete`, so its install and its removal took two
  different flocks over one package database. `xbps.rs` had this right and would have lost it in
  conversion — which is the honest reason to scope a deletion before doing it.

So the rule is not "prefer the data path". It is **one path, and a capability the machinery
lacks becomes a field rather than a reason to keep a second implementation**. That is what the
ratchet's header already said about the eight conversions of 2026-08-04, and it held for these
three: `CacheClean`, `DependsProbe`, `OutdatedProbe::silence_is_none` and `{name_component}` are
now available to all sixty-two backends, including a user's own row in `adapters/backends.toml`.

**Why the exemptions survived.** `tests/backend_is_data_not_code_tests.rs` is the ratchet, and
it is well built — a list that may only shrink, no stale entries, no "not converted yet". Its
one assertion about the *reasons* was `why.len() > 60`. So:

- `pacman.rs` claimed *"the removal guard needs pacman's own essential/required-by data"*. There
  is no `essential()` impl in the file. `grep -n essential src/backends/pacman.rs` returns
  nothing, and the row for `yay` two hundred lines away carries `essential_args: None` with the
  correct reason written out: pacman has no per-package essential flag, `base` is a convention
  and `HoldPkg` is user config.
- `dnf.rs` claimed *"a second command whose output changes what the first one means"*. It runs
  `rpm -qa` and `dnf repoquery --userinstalled` and reads both with **the same function**. That
  is `ManualListing::Command { format: SameAsInstalled }`, described in prose.
- `xbps.rs` claimed three binaries. Those are `binary`, `remove_binary` and `list_binary`, and
  `generic.rs`'s own doc comment names OpenBSD's `pkg_add`/`pkg_delete` as exactly this case.

Every one of the three was 100–160 characters of accurate-sounding English about a manager, and
all three passed. **Sixty characters of fluent prose is what a check that cannot fail looks
like when the subject is English.** Each entry now carries a `proof`: text that must appear in
the module the reason excuses. pacman's would have been `fn essential`, and it would have failed
on the day it was written. The instrument is self-tested against a planted falsehood first
(IV.1) — the check is run against a module that does *not* contain `fn essential`, so a grep
matching nothing cannot pass for a grep that found something.

**A fourth false reason, found by the gate that was just installed, and what it taught about
the gate.** `brew.rs` claimed *"`brew list --versions` and the search headers need parsing the
generic parsers do not have"* — `LambdaParser` has carried exactly that for every apk and AUR
row since they were written. The first `proof` written for the entry was brew's own install
argv, which is present in the module and establishes nothing; catching that took reading the
file. **A grep cannot decide whether a reason is sound, only whether it is about code that
exists** — what it buys is that writing the entry now requires naming a line, and naming a line
is where the three false claims came apart.

brew's real exemption turned out to be one line further down: `info` reads `brew info --json=v1`
for the keg prefix and whether the formula arrived as a dependency. `PropertyProbe` substitutes
a command's whole stdout into a `{base}` template and cannot reach `installed[0].prefix`, and a
listing cannot answer about a formula that is not installed. The entry says that now, and points
at `installed_as_dependency`. **brew is not converted in this change** — one more field to reach
into a JSON document is a fourth conversion's worth of design, and a sweep that keeps widening
is how a repo ends up with two of everything by a different route.

What did come out of brew and flatpak is the pair F-5 named directly: both had a
whitespace-columns loop inlined in `fetch_installed` that is `parsers::common::parse_simple_list`
verbatim. flatpak's was character-identical; brew's differed in one case — it dropped a line with
a single token, where the shared parser keeps it as a versionless package. The shared behaviour
is the safer of the two in the direction that matters (`Q40`: a listing that silently drops rows
reports a machine as emptier than it is), so the shared one wins and the local copy is gone.

**What was deliberately not changed.** `dnf`, `pacman` and `xbps` still report a package's
dependencies; they are the only three system managers that do, and the test asserting the other
fifteen ask nothing stays as it is. Under `Y9` a dependency is *reported, never planned from*,
so asking costs one subprocess on `shall info` and buys the `Dependencies:` line. Removing it
would be a feature a user would notice, which is not a detail to settle inside a refactor.

---

**V.143 — Why a plan has to say what it was computed over.** *(Rule in II.7.)*

A removal set is `managed − desired`. That subtraction is only as good as `desired`, and nothing
in `ChangePlanner::plan`'s old signature could tell a `desired` that was the machine's whole
declaration set from one that was a shell's four requests.

The signature took `Option<Scope>`. `None` meant two things:

- do not filter `desired` down to a profile or module, and
- every managed package missing from `desired` is drift, on every backend, remove it.

Those are unrelated facts and one of them is a decision about deleting software. Five of the
eight call sites passed `None`; four wanted only the first.

**The site that matters is `app/shell/mod.rs:269`.** `provision_transient_env` builds a desired
map holding **only the packages `shall shell` was asked for** — it is not the config, it was
never meant to be compared against the machine — and planned it as a whole-machine converge.
Every other managed package became a `Remove` node, handed straight to
`engine.sync(…, GuardScope::Sync)`. The test that pins this now prints, under the old code, the
program's own parallel breakdown uninstalling four packages in response to `shall shell fd`.
`max_removals` was the only thing between the plan and the machine, and a ceiling is not a rule —
it is a number that is usually large enough.

**Four hundred lines below that call, the planner describes the bug.** `planner.rs:480`:
*"Removal planning is GLOBAL… When the caller narrows to a single profile/module/group, `desired`
has already been reduced to that scope, so running removal here would delete every package
OUTSIDE the scope."* The comment is correct, it is in the right place, and it did not stop a
caller doing exactly that, because a comment cannot reach the call site and an `Option` gives it
nothing to reach with.

**And `planner.rs:39` recorded why it was an `Option` in the first place** — which is the part
worth keeping:

> *Absence of a scope is `Option::None` rather than a variant: as an enum variant it was an
> implicit spare-everything switch that `matches!` early-returns skipped past, so adding a
> variant produced no compiler error.*

That objection is real and it is why the fix is not simply "put the variant back". A
spare-everything variant is a hazard; an `Option` is the *same* hazard with less to read, because
nothing about typing `None` looks like a decision about deleting software. The answer to both is
a variant that **carries the thing that makes it safe**: `PlanScope::Whole(HostBackends)` cannot
be written without producing the backend list, and the list can only come from the resolver that
read `priority`. There is no spelling of "reap everything" that is shorter than saying so.

**Why the newtype, when a `Vec<String>` would compile the same.** `priority`'s promise is in the
error a new user reads when the file is missing: *"Listed means Shall uses it. Not listed means
Shall does not touch it at all."* A `Vec<String>` can be assembled by anyone from anything; a
`HostBackends` has one constructor and one caller, and `planner_scope_enumeration_tests` fails
the build if a second appears. That is the same rule `priority` itself exists to enforce (V.15) —
one place decides which managers are this host's own — applied to the code that acts on it.

**The gate found a hole in itself, which is the part to keep.** Its first run reported the
canary's file as *stale — it plans nothing any more*, because `upgrade --canary` bound its scope
to a variable above the call and no `PlanScope::` literal appeared in the argument list. The scan
had skipped it in silence: a clean report about a file it could not read. That is precisely the
"check that cannot fail" family the review filed as F-2, reproduced inside a check written to
close it — which is worth recording, because it says the disease is not carelessness. A scan
written to answer "what does each site say" answers nothing for a site that says it elsewhere,
and reads as a pass. An unreadable site now fails by name, and the canary states both variants at
the call.

**What this cost, honestly.** Two of the four broken sites are in `src/verbs/`, which `main.rs`
declares as a private module, so no test binary can reach them and no behavioural test can cover
them. They are covered by a source enumeration instead. That is a workaround for a module
boundary in the wrong place, not a design; it is the second finding this month whose test had to
be written sideways for that reason, and it belongs on the list of things to fix rather than in
the list of things that are fine.

---

**V.144 — Why the order of a sync is a type, and why a statement declares its own phase.**
*(Rule in II.7. Ruled 2026-08-06, `Y13`.)*

The order a sync does things in was a comment, and membership of that order was a chain of `||`.
Neither could be checked, and between them they were wrong four times — the same way, on the same
day each new statement kind shipped.

`verbs/sync.rs` wrote the bill down itself: *"every statement kind added since was missed by one
of them: extras, then `exec:`, then `dotfiles:`, then `firewall:` — four times."* That sentence
was written when the dry-run branch's duplicate phase list was folded into the real one, and it
correctly diagnosed the duplicate list. It did not notice that **three more copies of the same
fact were still standing**: `DesiredState::dependents()` spelled out `Shim | Service | Link |
Setting` in a `matches!`, each `has_*` helper spelled out its own kind, and
`has_non_package_work` was five `||`s naming five of them. A fifth kind added to the grammar
would have compiled against all four.

**A phase is a property of the statement, so it lives on the statement.** `Statement::phase()` is
an exhaustive match: a kind added to the grammar does not compile until somebody has said where
in a sync it belongs, which is the one question every one of the four misses failed to ask. It is
the same move `Statement::key()` already made for identity — that doc records *three* lists of
the same twelve variants, "none of which the compiler could check against the others" — applied
to the other fact the statement owns.

**`Ord` is the run order, and the chain of ors becomes a comparison.** `phase > Phase::Packages`
is "work the package transaction does not cover". Nothing has to be extended for a new kind to be
counted, and the exclusion that *is* deliberate is now expressible: `repo:` is not in that answer
because it is phase 1 and ran before the package plan, not because somebody forgot it. It had
been forgotten — the exit's own comment said so, and worked around it by counting placements
separately.

**Two halves, and only one of them is the compiler's.** `apply_non_package_phases` matches
exhaustively over `Phase`, so the binary will not build with a phase nobody dispatched. But
`src/verbs/` is declared in `main.rs` and is private to the binary, so no test can call it — and
a phase can still be dispatched to *nothing* by being folded into the ignored arm. That half is a
source scan.

**The scan shipped unable to fail, and a mutation caught it.** Its first version searched for
`Phase::Execs =>` as a substring. Folding `Execs` into `Phase::Resolution | Phase::Repositories |
Phase::Packages | Phase::Execs => {}` deletes the dispatch and still contains that substring, so
the check passed over the exact regression it was written to catch. This is F-2's family
reproduced inside a check written for F-2's family, which is now twice in two rulings — `Y12`'s
gate reported a file clean without having read it — and the lesson is the same both times: **a
gate is not a gate until it has been watched to fail.** The scan now requires the arm to open its
line and to have a non-empty body, and the three ways to look dispatched without being dispatched
— a comment, an empty body, the last alternative of an or-pattern — are each a control in its
oracle test.

---

**V.145 — Why K17 has one mechanism, when it had seven.**
*(Rule in II.7a and X.4. Ruled 2026-08-06, `Y13`.)*

K17 ruled that an adapter is a row in a table and the built-ins are rows in it. It was applied
seven times and **implemented** seven times: firewalls, init systems, settings stores, snapshot
providers, bootstrap commands, prereq steps and secret providers each grew their own loader.

Seven row *types* is correct and stays. A firewall's argv is `allow`/`deny`, an init's is
`start`/`stop`, a settings store's is `read`/`write`/`reset`; one schema holding all of them would
be a struct of twenty optional fields, which is the shape that makes a table unreadable. What was
written seven times is everything asked *about* rows — the `os` filter, the floor a row clears to
be acted on, the shipped-then-user merge, the which-row-is-this-machine search, and
`{placeholder}` substitution.

**Four of those five had already diverged, and nobody could have noticed.** Each table was written
by someone who had read the ruling and not the other six implementations:

- **`[[secret]]` had no `os` field at all.** Every one of its six siblings could be confined to a
  platform; the single table whose rows are handed a plaintext secret was the one that could not.
- **Three tables refused a second row claiming a name and three kept it silently**, so whether a
  duplicate was reported depended on which table it was in. In `[[secret]]`, which of two `vault`
  blocks answered was decided by file order and nothing said so.
- **The OS question had two spellings** — `applies_to_this_os()` reading `std::env::consts::OS`,
  and `applies_here(os)` taking it as a parameter. The four copies that read the constant directly
  meant the Windows arm of four tables could only ever be exercised on Windows.
- **The floor was seven near-copies**, agreeing on the first check and diverging after it.

**The built-in snapshot rows did not go through the floor a user's row goes through**, which is
the K17/U1 invariant stated in `snapshot_builtins.toml`'s own header. They pass it — a test says
so directly — so this cost nothing on the day it was found, and would have cost exactly one
shipped row the day one stopped passing.

**The gate is not the ledger.** A ledger of the seven tables catches an eighth being added. It
does not catch the thing that actually happened, which is a table quietly growing its own copy of
machinery that already exists — because each of the seven authors was doing something reasonable
in the file in front of them. So the duplication itself is a build failure:
`the_shared_machinery_is_written_exactly_once` asserts the OS filter and the usability floor
appear in `core/adapter.rs` and nowhere else in `src/`. Reintroducing a hand-written
`applies_to_this_os` fails the suite by file and line.

**`applies_to(os)` takes the OS rather than reading it**, so the platform arm of a table is
testable on a machine that is not that platform. That is not tidiness: it is the difference
between a Windows row that is checked and one that is hoped for.

---

**V.146 — Why the resource backends are not four converge loops, and why `Installable` keeps its
name.** *(No rule — a finding checked and refused. Ruled 2026-08-06, `Y13`.)*

`lamdan/whole-repo-2026-08-05.md` argued that the central abstraction should be named `Converge`,
and that calling it `Installable` is the **cause** of there being four hand-written converge loops
— *"there was no shared noun to hang an engine on, so each noun grew its own"*. It cites
`ZfsInstallable::install` reading existence and creating if absent, `service.rs` enable-then-start,
`setting.rs` read-before-write, `btrfs.rs` create-plus-fstab.

The four bodies are there. The causal claim is wrong, and it is worth writing down why, because
the reading is a natural one and will be made again.

**The convergence decision is already shared, in exactly two places, and neither is in those
bodies.** On the package path, `ChangePlanner` computes `desired − present` and then asks
`is_drifted` — one comparison covering `@quota`, `@size`, `@mount`, `@mount_options`, `@channel`
and `@classic` across zfs, lvm, btrfs and snap, with `limit_drifted`'s three-state reading of a
value the backend could not read (D13). A converged declaration never reaches `Installable` at
all. On the dependent path, `Dependents::apply` asks `apply::extras::in_effect` per resource and
skips anything already in force — the probe that exists because the loop once did not ask it, and
re-copied all three declared links on every Windows sync, leaving `.shall-backup` files that were
backups of Shall's own copies.

So the read inside each `install` body is a **local idempotence guard behind a decision made
upstream**, not a converge loop competing with three others. Merging them would merge four
actuators, not four deciders, and would buy a shared `for spec in specs` — while separating each
backend from the argv it exists to name.

**And the rename is half a vocabulary.** `Installable` is the verb; every value that flows through
it is a `PackageSpec`, produced into a `Package`, recorded in `StateRegistry.packages` and
serialized into `registry.json`. Renaming the verb to `Converge` while the nouns stay
package-shaped leaves the codebase with two vocabularies instead of one, at 159 occurrences, and
buys no property the compiler can check. Renaming the nouns too is a wire-format change to
`registry.json` and `SavedPlan` — a decision about compatibility, not a refactor, and the owner's.

**What the finding was actually pointing at is fixed, and it is not a name.** The reason those
four bodies looked like orphan loops is that nothing said where the decision was made. It says so
now, on the trait, which is where somebody reading one of the four will be.


---

## `F-4` — the log followed the engine, and eight commands walked around the engine

**V.147 — Why the write-ahead record belongs to the mutation and not to the command.**
*(2026-08-06, `Y14`. Rule in II.19. Raised by `lamdan/whole-repo-2026-08-05.md` as F-4.)*

The finding is accurate and its headline is wrong in a way worth keeping: *"`apply` is **the one**
change path `heal` cannot recover."* It was one of eight.

`verbs/plan.rs` contained zero references to `Transaction` or `journal` — true, verified, and the
review found it by reading the file the `plan`/`apply` feature lives in. What no reading finds is
the set of files it did not open. Enumerating every call that reaches `Installable::install` or
`::remove` outside `src/backends/` turns up thirteen files. Eight of them — eleven call sites —
recorded nothing: `apply` (2), `upgrade`, `remove-orphans` and `purge-undeclared` (2 in
`cleanup.rs`), the suspend removal in `packages.rs`, the expired-lease sweep and the suspension
restore (2 in `leases.rs`), `run`'s auto-provision, the shell restore, and the remediation
install in `diagnostics.rs`. **One of them is `purge-undeclared`, which the repo's own prose
calls the most destructive command in the program.**

**The mechanism behind the miss is the same one F-2 named**, and this is the ninth instance of it:
the journal was written for the transaction engine, so it lives in the transaction engine, and
"what the engine schedules is what gets journalled" quietly became the rule. Nothing was ever
decided about the other eight — they were never in the room. A gate drawn around
`core/transaction.rs` would have passed on every one of them.

**So the gate is drawn around the property.** `tests/wal_enumeration_tests.rs` scans `src/` for
package mutations, and every file holding one must appear in a ledger saying what makes an
interruption recoverable: `Transaction`, `Journalled`, or `Recomputed`. The third is not an escape
hatch — it is II.19's line, and stating it per file is what stops "it is a resource" from being
asserted about a call that installs a package. The ledger is checked in both directions, a
`Journalled` claim is checked against the file actually containing the call, and the scan is fed
the exact lines it exists to find before anything trusts it.

**What the record is, and what it is not.** `journalled` is the log without the ceremony: it
writes one entry per action, flushes, awaits the mutation, then closes the entries. A whole
transaction — snapshot, health checks, `after_sync` — is the wrong shape for reclaiming an expired
lease, and demanding one would have been the reason to keep doing nothing. The one property that
had to be exact is that `record_start` runs before the mutation future is polled, which is free:
a future that is passed in has not started. **The test that pins it makes the mutation the
observer** — the body opens a second handle on the same log file and counts interrupted entries,
which is what a fresh process after a crash would see. A wrapper that recorded around the call and
flushed afterwards passes every assertion about the finished state and provides no recovery at
all; only a witness inside the mutation can tell a write-ahead log from a write-behind one.

**And a log that cannot be written stops the mutation.** The transaction engine already made an
unrecordable batch stillborn rather than letting it run; the same rule now holds at every site,
because a manager invoked with nothing recording that it was invoked is exactly the state the
whole subsystem exists to prevent.

**One thing checked and left alone.** `run` and `shell` install packages the user calls temporary,
and the temptation is to read that as "no record needed". Temporary describes the intent, not what
`dpkg` is left holding when the process dies between the flush and the exit. They journal.

---

**V.148 — Why a frozen plan is executed by the engine that executes every other plan.**
*(2026-08-06, `Y14`. Rule in II.7 / the `plan`–`apply` list.)*

`plan`/`apply` is the Terraform story: freeze what `sync` would do, review it, apply exactly that.
It was implemented as two serial `for` loops calling the backend directly, and the steelman for
that is real — re-entering the sync engine sounds like re-planning, which would defeat the freeze.

**It does not, and that is the whole of the argument.** `SyncEngine::sync` takes a `SyncChanges`
and executes it. The planning happens in `ChangePlanner`, which `apply` never calls. So handing
the engine the graph rebuilt from the file preserves the freeze exactly, and everything the loops
were missing arrives with it: the write-ahead log, the transaction, auto-rollback, the prior-state
probe that stops a rollback uninstalling software the user already had, the pre-sync snapshot,
`@health=`, the per-package hooks, the events, and one manager command per wave instead of one per
package — which the engine's own measurement puts at **ten times** the cost (V.115). `apply` was
still paying that after `sync` stopped.

**The failure semantics change, and the change is the point.** The loops warned and continued, so
`shall apply` printed `Applied plan: 6 installed` over a machine where four had failed and exited
0. Through the engine a failed node fails the command, names the declaration it failed for
(`Q34`), and rolls back. A frozen plan is one change to one machine — the same reason
`continue_on_error` is off for `sync` — and half of a reviewed plan is not what was reviewed.

**The second half was hiding in the same function.** Rebuilding the graph used `add_node` in a
loop and wired no edges, so a `@requires` a user wrote survived `plan`, sat in the JSON in the
specs' own `requires` field, and was read back as nothing. Nothing detected it, because an
edgeless graph runs perfectly well in the wrong order and only a package that genuinely needs its
requirement first ever notices. **`rebuild` had the same bug and a worse spelling** — it keyed its
install map by the bare name, while `requires` is written `backend:name`, so the lookup could
never hit however the graph was built.

That is four hand-written copies of "add the nodes, wire the `requires` edges": the planner,
`heal`, `apply` and `rebuild`. Two had edges and two did not. There is one now
(`SyncChanges::add_installs`), and the sibling `add_removal` exists for the same reason at one
remove — the removal tracker is what `declined` consults to answer *is this already scheduled*, so
a removal added to the graph and not to the tracker can be scheduled twice, and four call sites
were maintaining that pair by hand.

**V.149 — Why a manager this machine does not have is skipped rather than failed.**
*(Owner ruling, 2026-08-06. Rule in II.7c, register entry `Y15`.)*

The want that makes Shall worth having is *rebuild a machine, or bring a second one into line,
without a day of remembering* — and the second machine is usually not the same kind of machine.
A configuration that cannot hold `apt:ripgrep` and `winget:ripgrep` at once is a configuration
per machine, which is the pile of shell scripts this program exists to replace.

**What actually happened before this.** `spec_is_missing` asked the registry for the backend and
turned `None` into `Error::BackendNotFound`, from inside the planner's fan-out — so the `?`
carried it out of `plan()` and **the entire sync failed, having planned nothing.** Not the one
line: the whole file. A Windows machine reading a shared `modules/` file with one `apt:` line in
it installed none of the twenty `winget:` lines beside it, and the message named a backend, not a
line.

**And it had already been ruled once, for the other half of the surface.** `Q9` clause 3,
2026-07-28: *"A real backend that cannot run here is a different answer. `flatpak` on a machine
without flatpak is not a typo — it is a fact about the machine — so it says that and exits 0."*
That governs a backend named in a command *argument*, and it is word for word the rule below,
decided nine days earlier. What `Y15` does is carry it from arguments to declarations. The
distinction it draws — typo versus fact about the machine — is the same one, and `install
brew:jq` on a machine without brew has been warning and exiting 0 the whole time that
`brew:jq` in a *file* was failing the entire sync. Two answers to one question, in one program,
because the ruling was applied to the surface that was under review when it was made.

**The correct behaviour already existed, one layer up, and had been written down.** `app/vocab.rs`
folds `priority` into the grammar's backend vocabulary *specifically* so that a manager this OS
does not build still parses; its header says so and names the alternative — "a baffling
unrecognised line" — and `priority_names_a_backend_this_os_does_not_build` has pinned it since it
was written. The grammar had been taught that a config travels. Nothing downstream had. That is
this repository's signature defect stated exactly: **the correct behaviour already exists at a
different site**, named as the headline by three separate grade rounds.

**Why the two kinds of missing are one rule.** `apt` on Windows is absent from the registry
because `create_default_registry` never registered it; `brew` on a Linux box without brew is
registered and answers `is_available() == false`. Two facts about Shall's internals, one fact
about the machine — and the first version of this fix handled one and not the other, which is
how `spec_is_missing` came to raise `BackendNotFound` for one case while planning an install that
could not run for the other. `runs_here` is one predicate so no call site can answer half of it.

**Why the typo does not get the same mercy.** Skipping is only safe because something else is
strict: `brwe:ripgrep` is refused by the grammar, against `Vocab`, before a plan exists. If
skipping applied to unknown names too, a misspelled line would be quietly dropped on every
machine and the config would describe a machine nobody has — a silence far worse than the
failure this rule removes.

**Why absence is skipped and failure is not.** They look alike from inside `sync` and are
opposite from outside it. *This machine has no brew* is true before the run starts, will be true
after it, and is not something the run can act on — reporting it as a failure asks the user to
fix a machine that is not broken. *`apt install nginx` returned 100* is a fact this run produced,
about work the user asked for that did not happen, and a summary that reports success over it is
`AU1`. So absence leaves the plan and failure stops the command, and `--keep-going` exists for
the one caller who genuinely wants best-effort — per-run only, because a machine-wide "never
fail" setting is exactly the destructive default `[remove] purge`'s own comment warns against.

**And a skip is louder than a drop.** Every skipped declaration lands in `SyncChanges::skipped`,
which already carried the rule that made this cheap: *an empty plan with a non-empty `skipped` is
NOT `already up to date`*. `apply` filters before it counts rather than leaving it to the
engine's backstop, because its summary is computed from the graph before the engine runs — and
`Applied plan: 4 installed` over a machine that got two is the same lie by a shorter route.

**V.150 — Why the three hook dialects are one language in three notations, and why a marker is
not part of the script.** *(Owner ruling, 2026-08-07, confirming the 2026-07-20 ruling that all
three dialects stay. Rule in II.12.)*

A hook's first line picks how it runs: a shebang makes it a process, `#rhai` runs it in-process,
anything else is Lua. Three arms, and every one of them was allowed to decide for itself what a
hook is handed — so they had drifted into three different features wearing one name.

**The `#rhai` arm had never executed a script.** The marker line was passed to the engine along
with the body, and `#` is a reserved symbol in Rhai, so every `#rhai` hook ever written failed
with a syntax error on line 1. It was not a rare path or a bad edge case; it was the whole arm,
and the only `#rhai` example that ships — in `examples/preferences.toml` — was one of its
casualties. **A dialect nothing tests is a dialect that does not run**, and this one had no test
because it had no test: the same sentence twice, which is how it survived a rewrite.

That example was wrong a second way. It called `exec("systemctl enable docker")`, and no engine
in this binary has ever registered `exec` — the hook arm registered `print` and stopped. So the
documentation described a function that did not exist, for an arm that could not have run it if
it had. **Two independent faults pointing the same direction is what an untested feature looks
like from outside**, and neither was found by reading, because reading is what produced them.

The rule is therefore three properties, not one fix:

- **The marker is consumed by whatever it selects.** `#rhai` is Shall's word and is stripped
  before the engine sees it; a shebang is the *script's* first instruction and is kept, because
  removing it leaves nothing to name the interpreter the author chose. The stripped line is blanked
  rather than deleted, so a runtime error still names the line the author wrote — a one-line
  offset in an error message is a bug that hides inside the fix for another bug.
- **Every dialect is handed the same four facts** — `PKG_NAME`, `HOOK_TYPE`, `OS`, `ARCH`, and
  `SHALL_`-prefixed for a process. Lua and the shebang arm had all four; Rhai had two, so a
  hook that branched on the platform could not be written in one of the three dialects, for no
  reason anybody had chosen.
- **A `#rhai` hook gets the same standard library as `vars.shall`** (II.6b's clock, shell,
  read-only files, environment, network and `parse_json`), from the same function. II.6b's own
  wording is that `vars.shall` is *"trusted the same as a hook"* — a definition by reference to
  hooks — so a hook having strictly less than the thing defined in its terms was backwards. The
  narrower arm was never a security posture and V.55 already said why: a hook two lines away in
  the same config can open `#!` and run anything, so withholding `sh` from one notation stopped
  nobody. **What gates a hook is II.12's ledger**, which hashes every one of them and refuses an
  unapproved or changed script under `-y` and with no terminal alike.

Not deleted, fixed. The cheap reading of "the Rhai arm is broken and `mlua` costs 28,000 lines
of vendored C per build" is that a dialect should go. But a broken feature is evidence about the
tests, not about the feature: all three are reachable by a user, all three are ruled, and the
defect was that nothing ever ran two of them. The fix is one engine builder, one fact list, one
place that decides a dialect — and a test that executes each arm, which is the part that was
actually missing.

**And then the arm that *did* run turned out not to run here (Y17, owner ruling 2026-08-07).**
Fixing `#rhai` meant putting a real binary in front of all three dialects, which is how the third
one was found dead on Windows. A script file handed to `CreateProcess` comes back *"The specified
executable is not a valid application for this OS platform"* — measured against the OS, not
inferred. The shebang is a **kernel** feature; Windows has no equivalent at any layer, so no
amount of care inside the hook reaches it. What a user saw was `Polyglot execution failed: … (os
error 193)`: a message about their script, for a script that was fine.

Routing it through PowerShell was the obvious repair and is the wrong one. `#!/usr/bin/env
python3` would then run under PowerShell, which treats the line that chose Python as a comment —
**that does not run the script, it runs a different one**, and it converts a clear failure into a
confusing success. A blanket refusal was the honest alternative, and it costs the thing the
product is for: one config, every machine.

So Shall reads the shebang itself. The measurement that made this cheap rather than a
reimplementation of the kernel: **the `#!` line does not have to be removed first.** Every
language a shebang names treats it as a comment, so `python foo.py` runs an unmodified
`#!/usr/bin/env python3` file. What is left is a name lookup.

- **On every platform, not just the broken one.** Three callers shared this file and used two
  different answers, which is the shape of the bug the file was created to prevent. An absolute
  interpreter that exists is used as written, so Unix launches exactly the binary the kernel would
  have — the platforms agree instead of diverging.
- **`/usr/bin/env` is dropped, not launched.** It is a PATH search wearing a path, there is none
  to launch on Windows, and the search now happens here. Leaving it in place is the difference
  between finding `python` and reporting that `/usr/bin/env` is missing.
- **`python3` falls back to `python`, then `py`.** `python3` is the name Unix uses and almost no
  Windows machine has it. This is the case the whole ruling was about, and a fallback list that
  omitted it would have been a feature that works in principle.
- **A candidate with bytes in it is preferred over one without.** On Windows `which python3`
  returns `%LOCALAPPDATA%\Microsoft\WindowsApps\python3.exe`, a zero-length reparse point.
  Configured, it launches Python; unconfigured, it opens the Microsoft Store and runs nothing —
  **and the two are identical to inspect**, since the working one on this host is also zero bytes.
  So the rule is not "detect the dead alias", which cannot be done, but "prefer a candidate that
  is unambiguously a program". `winget`, which has no other form, keeps its alias.
- **`env -S FOO=1 python3` is refused.** `exec:` runs through an executor with no per-command
  environment, so this could be honoured by two callers out of three — and a form that works in
  two places out of three is the two-of-everything disease with a friendlier face.

**The temp file dropped from 0755 to 0600 in the same change.** The execute bit existed because
the kernel was being asked to run the file; an interpreter named on the command line only reads
it. What was left was the author's script sitting world-readable in a shared temp directory for
as long as the hook took.

**The fourth site had the same bug inverted, and shows why one lookup is the point.** A
`vars.<ext>` provider picks its interpreter by *extension* — IX.6's whole point is that a
`vars.py` runs without a shebang or a chmod, and that stays. But the name it produced was
literally `python` on Windows and literally `python3` everywhere else, so the assumption that a
program has one spelling per platform had simply been made twice, in opposite directions. A
Windows box with only `python3` and a Linux box with only `python` each had a provider that could
not run. The extension table is untouched; the name it yields now goes through the same lookup,
which is what carries the fallbacks and the alias-avoidance to it. **It was deliberately not
given shebang parsing** — a second dispatch inside the file whose rule is "no shebang needed"
would be two-of-everything wearing the costume of a fix.

**V.151 — Why a channel change is repaired by whatever the backend's switch actually is, and why
two branches means "unreadable".** *(Rules in II.2. Owner ruling 2026-08-09, `Y23`; D13 is the
parent.)*

D13 ruled the shape — a `@channel` that differs from what a package follows needs a refresh —
and it was built against snap, which has a refresh. The rule then read as though every backend
that publishes channels had one. flatpak does not, and the gap was invisible in both directions
at once: **flatpak's `@channel` really did reach the machine** (`install_ref` builds the ref
`org.gimp.GIMP//beta`) and the installed branch was never read back, because the listing asked
for `application,version` and stopped there. So editing a flatpak's channel installed the new
branch the first time and did nothing for ever after, and *neither half announced itself* — the
declaration was honoured once, which is the most convincing way for a feature to look finished.

**Adding the column would not have been the fix; it would have been the next bug.** flatpak's
`install` calls an already-installed ref an *error* and exits non-zero — the string
`Error: %s%s%s already installed` is in the shipped binary. Making the drift visible without
changing the repair would have turned a channel that did nothing into a sync that failed on
every run, which is the same defect with a louder failure mode. `--or-update` — flatpak's own
*"Update install if already installed"* — is what makes the repair idempotent, and it is applied
to every flatpak install rather than to the channel path, because an adopted package or a
half-applied plan reaches that command holding a ref the machine already has.

**And installing the branch is not switching to it.** flatpak keeps branches side by side; the
launcher goes on running the one it ran yesterday until `make-current` says otherwise. A repair
that stopped at the install would have reported a converged channel over a machine where nothing
a user could see had changed — a plan that lies about what it did, which is the class II.7 exists
to prevent.

**Which leaves what "the installed branch" means when there are two of them, and the answer is
that there is no answer.** `flatpak list --columns=help` offers no current-branch column, and the
binary carries no such word among its option strings — this was measured in a `debian:12`
container against flathub, not inferred. So an app on two branches reports **no** channel, and
D13's existing rule takes it from there: a value the backend could not read is left alone. The
alternative was to pick one of the two rows, which reads as thoroughness and is in fact the exact
failure D13 was written to prevent — a wrong reading schedules the same switch on every sync for
ever, and unlike the silent version, that one edits the machine.

**The two backends that have channels both report one now, and there are exactly two.**
`capability::HAS_CHANNELS` is `["snap", "flatpak"]`, and the sweep that closed this checked the
other end too: every key in every `*_OPTION_KEYS` table has a reader outside the grammar. The
family is closed by enumeration rather than by resemblance.

**V.152 — Why `@source=` is read when the shim runs and not when it is deployed, and why a shim
must never resolve to itself.** *(Rules in II.2 and the option table. Owner ruling 2026-08-09,
closing `Y18`'s third finding.)*

`source` was a legal option on a `shim:` line that nothing read. The imperative `shall shim
--source` had thrown the same value away before it, II.16 converted the command into the line,
and the defect **moved house rather than dying** — accepted by the parser, listed in the option
table, discarded at apply time, which is the "silently ignoring an option the user wrote" shape
II.2 names, sitting inside the repo that names it.

The reason it stayed unbuilt is a false constraint worth recording: a shim is the shall binary
copied under another name, so there is nowhere in the *artefact* to keep the answer, and every
sketch of the fix started by inventing a sidecar file to keep it in. **The record already
exists.** The config that declared the shim is the same config the shim process loads on its way
in — it still says `shim:jq@source=cargo:jq` — so the option is read at run time, from the
declaration, and no second store is created that could disagree with the first. A sidecar would
have been two of everything, invented to solve a problem that had already been solved by the
thing that caused it.

**The mechanism it belongs to had never run, and reading it end to end is what found the rest.**
`exec_shim` had no test caller anywhere in the tree, and the path it takes ends at
`Command::new(name)` — a bare `PATH` lookup, with nothing excluding `[bin_dir]`. That is the
directory the shim was deployed into, ahead of the real binary, deliberately: the search finds
the shim, which re-enters Shall, which searches again. One process per turn. Nothing in the tree
stopped it — no depth counter, no environment marker, no exclusion — and it was reachable by
typing the shimmed name on a machine where the shim worked as designed.

**The exclusion is by identity, not by directory,** and that distinction is the whole reason the
fix is not one line. `web:`, `github:` and `appimage:` deploy real executables into that same
`[bin_dir]`; a runner that skipped the directory would be unable to find the packages three
backends install. So the search asks each candidate whether it is *this binary under another
name* — the ownership test `create_shim` and `remove_shim` already share — and skips only that.
The fallback when a `PATH` holds nothing else is the bare name, so the user gets the error they
would have got by typing it rather than a silent re-entry.

---

**V.153 — Why a gate carries an oracle that drives the gate, and not a sentence about it.**
*(Rule in II.23. Fixed 2026-08-09, `S48`.)*

**Eight grade rounds named "a check that cannot fail" as this repository's signature defect, and
the defect then appeared inside three checks written to close it.** That is not carelessness
twice; it is the same mistake having a shape, and the shape is worth naming: *when you sit down
to prove a scan works, the easiest true sentence to write is one about the scan's vocabulary
rather than one about the scan.*

Here is what all three did. `output_is_sanitized_tests.rs` wanted to prove its scan still
recognised an unsanitized read of a command's stdout. So it declared

```rust
let raw_read = "let s = String::from_utf8_lossy(&out.stdout).trim().to_string();";
assert!(raw_read.contains("from_utf8_lossy(&") && raw_read.contains(".stdout"));
```

Every word of that is true. It is also true of a tree where the scan has been deleted. The
assertion is about the literal on the line above it, and the scan is never called — so the test
passes on the string's contents, which the author chose, rather than on the code's behaviour,
which is the thing under doubt. `ledger_file_rules_tests.rs` did the same with two literals, and
`grader_refusal_exit_code_tests.rs` did it with three, **underneath a doc comment quoting the
standard it was violating**: *"do not test your own oracle by assuming it works"*.

**This was verified rather than argued.** Each predicate was mutated — `is_raw_stdout_read` to
`false`, the ledger checks to `false`, `run_exclusive`'s per-key mutex to one global key — and
in every case the *real* scan stayed green and only the rewritten oracle went red. The old
oracles would have stayed green too, which is the claim, and it is now a measurement.

**The structural cause is where the predicate lived.** In all three the rule was a run of
`continue`s inside the directory walk, and in `grader_refusal_exit_code_tests.rs` the two
helpers were nested inside the test function itself. A predicate with no name has no caller but
the loop it sits in, so the only available way to check it is to read it — and reading it is
precisely what had already been done, twice, by the person who wrote the bug. Giving the rule a
name is most of the fix; the oracle is what a name makes possible.

**The controls matter more than the offender.** A scan that catches the planted offender and
also catches four innocent lines is not a working scan, it is a noisy one that will be
suppressed. Every control in the three rewrites is a shape that a previous version of some scan
in this repo got wrong: a phase folded into an or-pattern (V.144), a wrap that sits below its
call instead of above it, a `stderr` read where the rule is about `stdout`, a comment mentioning
the call. The oracle is the only place those near-misses are written down as *deliberately not
findings*.

**And the floor is the half nobody writes.** A walk with the wrong root, a `read_dir` that
failed and returned early, an extension filter that stopped matching — each produces an empty
result set, and an empty result set is indistinguishable from a clean tree. `Y12`'s gate
reported a file clean without having read it, for exactly this reason. So each scan now asserts
what it *reached* — over a hundred files under `src/`, at least six ledgers, at least ten
refusal sites — before asserting that what it reached was clean.

**Six more tests were not oracles at all; they simply asserted nothing.** The instructive one is
`shell_lifecycle_tests.rs`, which wrote a `shall.txt`, read it back, and then split it using its
own copy of `auto_shell`'s five lines. It asserted that `str::lines` works. It would have passed
against a Shall with no manifest discovery in it. Copying the production body into the test is
the vacuous-check family wearing its most convincing disguise, because the test *looks* like it
knows the implementation — and the fix is the ordinary one this repo keeps arriving at: there
should be one copy, the test should call it, and here that also retired a **fourth**
implementation of the comment rule, so `brew:jq  # a note` now parses and a URL fragment
survives.

**`dag_test.rs` is the other shape worth recording: a test whose mechanism cannot distinguish
the two answers.** It fired one `brew` call and one `cargo` call through `tokio::join!` and
asserted both returned `Ok` — under a doc comment claiming it proved per-backend locking. A
single global mutex returns `Ok` for both as well; it just returns them one after the other. And
mock commands complete instantly, so there was nothing to contend over in either world. The
property needs *time* to be observable at all, which is why `MockExecutor` gained `set_delay`,
and it needs both halves asserted against each other — two backends must overlap, two calls to
one backend must not — because either half alone has a passing explanation that is the opposite
of the intended one.

---

**V.154 — Why the guard scope is passed as the enum, and never as its own label.**
*(Rule in II.10. Fixed 2026-08-09, `S49`.)*

**Two functions, fourteen lines apart in behaviour and two files apart in the tree, spoke
different languages about the same value, and nothing in between could notice.**

`verbs/sync.rs` had `scope_label(GuardScope) -> &'static str`, which answered
`"an unattended watch tick"` for `Watch` and `"sync"` for everything else.
`apply/firewall.rs` had `guard_scope(&str) -> GuardScope`, which matched `"purge-undeclared"`,
`"watch"`, and `_ => Sync`. The first is the only producer. The second is the only consumer.
**Neither of the consumer's named arms could ever be reached**, because the producer never
emitted either word.

The value went in as an enum, came out as an enum, and lost its identity in the middle — which
is the failure this repository keeps finding in new places: *the type stops at a module boundary
and continues on the other side as a string.* `Reaped`, `Phase`, `PlanScope` and `HostBackends`
are all this technique applied successfully. The disease is not that the cure is unknown.

**What made it expensive is `N7`.** Read alone, a dead `"watch"` arm looks like a branch waiting
for a feature. `N7` (ruled 2026-07-24) makes an unattended `watch` tick **revert drift by
default**, reporting instead only when the revert would close the session's own port. So the
unreachable arm was not aspirational: it was the guard scope for a live, ruled path that closes
ports on a machine with nobody in front of it, and it silently resolved to `Sync`. A refusal on
that path named a command the user was not running.

**The test written for this exact promise could not see it, and the reason is worth keeping.**
`a_firewall_teardown_is_a_removal_tests.rs` opens with nineteen lines diagnosing the original
defect better than any review did. It then asserts `GuardScope::Sync.as_str() == "sync"` for
three variants, under this comment:

> *"The mapping is private, so this asserts the property through the public enum instead."*

**The private mapping it declined to test is the broken one.** A getter returning what a
constructor took is true in every world, including the one where the round trip is severed. And
`model/firewall.rs`'s own unit test had been calling `lockout_refusal(22, "an unattended watch
tick")` — passing the producer's output straight to the consumer's caller by hand, so the test
and the producer agreed and neither had ever met the thing that reads it. Making the parameter
an enum turned that into a compile error, which is how the second half was found.

**Why `during()` is written out per variant.** The enum now carries both vocabularies, because
they answer different questions: `as_str` is the command to *retype with a flag on it*, so it
must be what the user typed; `during` is what a reader needs to understand a refusal, and there
the load-bearing fact is whether anybody was watching. A tempting `other => other.as_str()`
catch-all would have been shorter and is precisely the shape that produced the bug — the deleted
label answered `"sync"` for nine of twelve scopes for exactly that reason. Twelve arms, and a
test asserting no two of them collide.

---

**V.155 — Why the lock asks the command, and why `history` is a reader with a lock inside it.**
*(Rule in II.24. Fixed 2026-08-09, `S50`.)*

**A hand-written list of twenty-one strings, seventy lines from the enum it describes, decided
whether a Shall run took a 120-second exclusive lock on everything it knows about the machine.**

The list's own test docstring is the best argument against it, and it was written by whoever
last repaired it: *twelve of its thirty-three entries named commands the program did not have*
— `status`, `doctor`, `unmanaged`, `absent`, `insight`, `show`, `audit`, `outdated`, `log`,
`locate`, `metrics`, `verify`. That was found and fixed. **Two tests were then written to keep
it fixed, and both guarded the same direction**: that every name on the list is a real command.

Invention is the harmless half. A list naming a command that does not exist exempts nothing.
The expensive halves are **omission** — a writer that is not on the list is correctly locked, so
that one is safe — and **misclassification**, where a writer *is* on the list. Nothing checked
that, and it was live in both directions at once:

- **`history` was exempt, and reached `handle_rollback` → `handle_sync`.** That is the entire
  install/remove path, `state.save()` included, running with no lock held. The *same function*
  reached through `Commands::Rollback` was locked. One function, two doors, two locking regimes,
  and which one you got depended on whether you typed the verb or picked it out of a TUI.
- **`fleet` was absent, and touches nothing local.** It drives other machines over SSH and took
  the writer lock for a purely remote report — every other Shall on the box waiting behind a
  command that was never going to write.

**The argument for reading argv was good and had stopped being true.** The doc comment said the
name was taken from argv *"rather than matched out of `Commands`, so a subcommand added later is
locked by default instead of being forgotten by a match arm nobody updated."* That is a real
concern and the right instinct. But `acquire_data_lock` is called at `main.rs:147`, **after clap
has parsed** — the `Commands` was sitting right there — and an exhaustive match does not have
the failure mode the argv read was defending against. A forgotten arm is a compile error. The
defence was against a weakness the chosen design does not have, and it cost the thing it was
protecting.

**Why `history` stays a reader.** The obvious repair is to mark it a writer, and it is wrong.
`AU6` records why: `edit` blocks on `$EDITOR`, and locking it meant one person reading a manifest
in vim stopped every other Shall on the machine for as long as they read. A history browser is
the same shape — a human reading a screen for an unbounded time. So the exemption stays at the
command and the lock is acquired where the mutation begins, which is one arm of one match. The
general rule that falls out: **a command's lock class is about what it does by default; a
mutating action inside a reader takes the lock itself.**

**And `run_user_verb` was locking unconditionally for a reason that also expired.** Its comment
said the verb name is unknown to `acquire_data_lock`, so it locks as the safe default for a
sequence that may install or remove. But it parses each step into a `Cli` already — the sequence
can be asked. It now takes the lock when *any* step writes, once, spanning all of them: a verb
of five readers stops blocking the machine, and a verb whose third step syncs takes the lock
before its first step rather than partway through, which is the case where releasing between
steps would have let another writer in between two commands that have to agree.

**Both replacement tests were watched failing.** With `History` moved to the writer arm, the
reader-set assertion goes red naming the diff, and the clap-driven classification test goes red
naming `history`. The reader set is read out of `Commands::writes` itself rather than restated,
so a variant moving between the arms appears as a diff in this file — which is precisely what a
list living seventy lines away could never do.

---

**V.156 — Why `info` is about the machine, and how three backends came to answer the catalogue.**
*(Rule in II.25. Fixed 2026-08-09, `S53`, `S52`.)*

**`Queryable::info` had no doc comment.** Not a short one — none. Fourteen implementations, one
undocumented signature, and the meaning lived entirely in the call sites: `spec_is_missing` reads
`Ok(None)` as *schedule an install*, `prior_state` reads it as *this was absent before we
started*, and `shall info` prints *"is not installed on this machine"* from it. Those call sites
are heavily commented — `planner.rs` explains at length why `Err` must not be read as absence —
and every one of those comments is on the reading side of the boundary.

Three backends answered the other question:

- **`vscode` POSTed to `marketplace.visualstudio.com`** and returned `Some` for anything
  published, carrying the marketplace's *latest* version. Every plan therefore made one
  rate-limited HTTPS POST per declared extension, and every one of them said "already installed".
- **`brew info --json=v1`** prints a full record for any formula in any tapped repository. The
  code read `installed[0]` for its `prefix` and `installed_as_dependency` — so it *had* the field
  that distinguishes a tap from a machine in its hand — and ignored whether the array was empty.
- **`snap info`** answers from the store. The local helper `installed_state` returns `None` for a
  store-only report, its doc comment says *"`snap info` answers happily for a snap that only
  exists in the store, and reading that as installed would send every first install down the
  refresh path"*, and there is a test pinning it. `info` did not call it for that.

**The failure is the same in all three, and it is silent in the worst direction.** The planner
asks *is it missing?*, hears *no*, and schedules nothing; `shall install vscode:x` prints success
having done nothing at all. Then the version half: a `@version=` pin compares against upstream's
newest published build, so the moment a new one appears the pin is "drifted" and re-installs on
every sync, for ever, without ever converging.

**This was already known.** `mise.rs:183` carries the obituary in a doc comment — *"It used to
ask `mise plugins ls --all`, which lists every plugin mise has ever heard of… `shall install
mise:jq` reported already up to date while installing nothing"* — found by the `tools` container
on 2026-07-24, with an assertion at `:409` that the catalogue is never consulted. **The test that
would have caught vscode, brew and snap already existed, written against the fourth backend.**
That is what a family looks like when only one member is fixed: the diagnosis is perfect, the
prose is quotable, and the bug is still shipping in three other files.

**`appimage` is the same shape one field over** (`S52`). `install` keys its state by the URL,
because for that backend the URL *is* the name; `fetch_installed` reported the basename. `info`
compares the two and never matched, so every declared AppImage read as absent and `sync`
re-downloaded all of them on every run, for ever. `btrfs.rs` carries a test for exactly this,
dated 2026-07-30, whose comment reads *"A name `list` does not return is a package `sync`
believes is absent: it re-creates it on every run, for ever."* Diagnosed, named, tested, and
fixed in one member.

**So the contract is now on the trait method** rather than distributed across its readers, and
the rule it states is the one the four fixes have in common: the answer is about this machine,
and the version is the one on disk.

---

**V.157 — Why the JSON is found rather than assumed, and why that is one function.**
*(Rule in II.26. Fixed 2026-08-09, `S51`.)*

composer prints `Changed current directory to /root/.composer` ahead of every global command
whenever a global config directory exists — which is every machine that has ever run
`composer global`, i.e. every machine that has a composer package to manage. Parsed from byte
zero that banner is a syntax error.

`parse_composer_json` answered a syntax error with `unwrap_or_default()`, which is `Value::Null`,
which answers `None` to every accessor, which yields the empty vector. **So the installed listing
`sync` plans from was empty on every real machine**: every declared PHP package a fresh install,
every removal silently dropped. That is `LX-1`'s failure — *nothing in the chain believed
anything had failed* — reappearing one layer above the place `LX-1` fixed it.

**The comment explaining the banner was already in this repo, two lines from the wiring.**
`registry.rs:1933` describes it exactly, attached to the `outdated` probe, whose parser opens
with `text.find('{')`. The installed reader, sitting on the line above, did not. Two lines apart,
one of them right — which is the argument for a shared function rather than for a second correct
copy: the sibling's fix is now the only implementation, and `parse_composer_outdated`'s bespoke
`find('{')` was deleted rather than left as a second one.

**Stopping at the end of the first value is the other half.** `serde_json::from_str` rejects
trailing bytes, so a manager that prints a summary line *below* its document fails for the mirror
image of the same reason. A stream deserializer reads one value and does not care what follows.

**The fallback is anchored to a line start, not to the next brace.** If the first bracket byte
turns out to be inside the banner — a path with a `{` in it — the retry looks for a line that
*opens* with a bracket. Scanning forward brace by brace would eventually find a nested object
inside a malformed document and return one sub-tree of it, confidently and wrongly, which is
worse than returning nothing.

**And it returns `Option`, not `Value::Null`.** The whole failure above is a caller that could
not tell "I did not understand this" from "there is nothing here". A reader that hands back
`Null` for both makes that distinction unavailable to everyone downstream.

---

**V.158 — Why the shared helper was the one with the weak rule, and six copies had the strong one.**
*(Rule in II.27. Fixed 2026-08-09, `S54`.)*

`LX-1` is the repo's own name for the mistake: *a parser that did not understand the output
answering "the machine is empty"*. The type that fixes it, `Unrecognised`, is one of the
best-documented things in the codebase, and the function that decides which answer to give —
`or_unrecognised` — carries a paragraph explaining that an empty candidate list is a real answer
and a populated one that yielded nothing is not.

**Then a JSON escape hatch was added to it, for a real reason, and it took the check with it.**
`pipx list --json` on an empty machine prints a sentence and then four lines of JSON. Those four
lines are not prose by the general rule and not package rows either, so the line count called
them "unread" and `shall list` warned about pipx on every run of a clean box. Measured on a real
Windows machine, 2026-08-07. The fix was: *if the output contains a parseable JSON document,
return `Ok`.*

Which is correct for the case it was written for and catastrophic one step to the left.
**`Ok(found)` — where `found` is empty — was returned for any output holding a parseable
document, whether or not the reader had extracted a single package from it.** npm renaming
`dependencies`, pip capitalising `name`: still valid JSON, still parses, still reads as a machine
with nothing installed. Five backends reach that arm — npm, pnpm, pip, pipx, composer — and they
were precisely the ones the check no longer protected.

**Six sites elsewhere in the repo already had the right rule.** conda (twice), dotnet, pixi,
winget, scoop and the custom-backend onboarder each wrote `if found.is_empty() &&
!container.is_empty() { … }` by hand, several with excellent comments about why the two answers
must not share a return value. That is the shape of the problem: the correct rule copied six
times, and the shared function everyone else called holding the weak one. The five backends that
did the right thing — reuse — got the worse behaviour.

**And a length alone could not have expressed it.** `Some(0)` and `None` are different answers:
the container was there and empty, or the container was not there. Every one of the six
hand-rolled copies could only ask the first question, because it had already `let Some(arr) = …
else { return unreadable }`-ed its way past the second. Folding them into one helper made the
distinction a parameter, so the case each copy handled separately is now the same case.

**The line-count fallback survives, for the `None` branch only.** A refusal that says "0 line(s)
this parser does not recognise" about a screenful of output reads as a bug in the refusal, so
when there is no entry count to report it falls back to counting the non-blank lines — which is
exactly what all five private `unreadable` helpers did, and the reason they existed.

**What was deleted:** five private `unreadable`/`export_unreadable` builders, seven literal
`found.is_empty() && !container.is_empty()` guards, and the `candidates[start..].join("\n")`
full-output copy the escape hatch made on every pipx and yarn listing. The tests that pinned the
pipx and yarn empty cases still pass — through the container check now, which is the reading that
was meant.

---

**V.159 — Why the count is a value the command owns, and why there are two of them.**
*(Rule in II.28. Ruled 2026-08-09 as `Y20`; fixed as `S55`.)*

**The parameter was right and that is what made it dangerous.** `inspect_removals` took
`also_removing: usize`, and its doc comment argued the case correctly: *"a sync that drops three
packages and three links removes six things, and a limit of five must see six. Checking each
phase's own list separately is how a ceiling gets passed twice by a plan that exceeds it once."*
Exactly so. The design was understood. The problem is that understanding it left every caller
holding a number, and a number three callers assemble is a number one of them assembles wrong.

Three callers, and they diverged in the way this repo's bugs always diverge — the reviewed one
was right and the quiet one was not:

- `verbs/sync.rs` passed `changes.total_remove()`, read *after* the TUI may have filtered the
  plan. Correct, and carefully so.
- `verbs/plan.rs` passed `package_pairs.len()` for its preview. Correct.
- **`apply/firewall.rs` passed `0`.** So four packages and four ports, under a limit of five,
  were invisible to every guard call in the run: the packages saw four, the ports saw four, and
  nothing ever saw eight.

The firewall teardown had been outside the guard entirely until 2026-08-07 (`Y20`), and the
change that wired it in supplied the argument the signature demanded — with the only value
available at that call site, which was nothing. Not carelessness: the count simply was not
reachable from there. **A parameter that a caller cannot answer is a parameter that will be
answered with a zero.**

`Reaping` is the number, owned by the `App` — one invocation, one budget — and written where the
guard clears a set rather than where a caller believes one was cleared. `enforce_extras` no
longer asks how much has gone; it looks. The firewall struct holds the value instead of a
literal, which is the only change that call site needed and the one it could not make while the
count lived in `verbs/sync.rs`.

**Then the second half, which is the owner's ruling and not a repair.** With the phases finally
sharing one number, the number turned out to be the wrong shape: `Q7` had ruled in July that
packages and resources count *together*, and `Y20` had left open whether a closed port belongs in
that count at all. Together, one ceiling of twenty meant a machine with forty ports open and one
`firewall:22/tcp` line could not converge — and, worse, that its perimeter and its package set
competed for the same budget, so tightening a firewall made the machine less able to remove a
package.

Ruled: two counts. **Software leaving a machine and a perimeter tightening are different events
and deserve different tolerances**, and one number makes the stricter of the two govern both.
`max_removals` stays packages-only at twenty; `max_extra_removals` is new, also twenty, and
covers every teardown including the ports. `Q7` clause 2 is amended: *whole command, not each
phase* stands; *together* does not.

**Both answer to the same `--allow-mass-removal`**, because the question the flag answers — *yes,
that many, I meant it* — is one question, and a second flag would be a second thing to remember
in the one moment a user is already being interrupted. And both refusals now name the setting
they hit, because a message reading `[guard] max_removals` over a port closure sends the reader
to a line that will not help.

**The forty-port machine still refuses**, at the new ceiling rather than the old one. That is
the ruling, not an oversight: forty ports closing at once is the shape a ceiling exists to
interrupt, and one flag is the answer.

**One detail worth its line: a refused set is not recorded.** The total is raised only where the
guard says yes. A removal that was refused must not make the next phase's budget smaller — the
command is about to stop anyway, and the alternative is a counter that punishes a plan for what
it was not allowed to do.

**The third ceiling, and the one over all of them** *(ruled by the owner, 2026-08-10 — `N8`)*.
`Y20` split the count in two and left the ports inside the resource half. That was one lump too
few. The run that first declares a perimeter is precisely the run that closes the most ports, and
under `Y20` it spent a budget meant for `link:`/`service:` teardowns to do it — so declaring a
firewall could refuse a dotfile change that had nothing to do with the firewall. Ports get
`max_port_closures`, on the same reasoning that gave resources their own number: **the kinds fail
differently, so they get different tolerances.**

**Why a total on top of that, when three ceilings already exist.** Because three ceilings of
twenty permit fifty-seven changes, and nobody who wrote `20` three times was thinking of
fifty-seven. Per-kind numbers answer *how much of this kind is too much*; none of them answers
*how much is too much*. `max_total_changes` is that number, and it counts what the removal
ceilings never look at — installs and upgrades, resources written, ports opened — because a
total that only counted removals would be the removal ceiling with a longer name. That is also
why `enforce_additions` exists at all: it has no ceiling of its own and refuses nothing on its
own account. It is there so the total is a total.

**Why it is off by default when the other four are not.** The three removal ceilings protect
against a class of accident — a manifest edit that deletes more than you meant — and twenty is a
number a machine can carry without noticing. A total is a statement about how much churn *this
machine* tolerates, which is not a thing Shall knows. Shipping it at any non-zero number would
refuse syncs that ran yesterday, on machines whose owners never asked for it, and the first
thing every one of them would do is turn it off. A default that is turned off before it catches
anything is worse than no default.

**Why both mass flags answer it and no third flag exists.** The total is made of installs and
removals both, so either "yes, that many, I meant it" covers it. A `--allow-mass-change` would be
a third spelling of one sentence, and the moment it exists a user has to remember which of three
flags a refusal wants. What does **not** carry over: `--allow-mass-install` answers the total and
the install count and nothing else. The flag that means *install* that many must never quietly
also mean *remove* that many — that conflation is II.10's original bug, one ceiling up.

**Why a refusal names every ceiling it hit rather than the first.** A set can be over its own
number and over the total at once. Reporting one of them sends the reader to raise it, run again,
and meet the other — twice the interruption for one decision, and the second one arrives looking
like the fix did not work.

---

**V.160 — Why the kind is a type, and what the two catch-alls were quietly doing.**
*(Rule in II.29. Fixed 2026-08-09, `S56`.)*

`Statement::kind()` returned `Option<&'static str>` — the keyword, as text. Its own doc comment
made the right argument for existing: *"A caller that wants to group or filter by kind asks here
rather than re-splitting `key` on `:`, which would read `apt:jq` as the kind `apt`."* Correct,
and it is why the function is there. But a `&str` cannot be matched exhaustively, so both
dispatches over it ended in a branch nobody had to justify, and both branches were wrong in the
direction that leaves no trace.

**The teardown's catch-all reported success.**

```rust
other => {
    warn!("no undo known for extra kind `{}`.", other);
    Ok(())
}
```

`Ok(())` from `undo_extra` means *the undo is done*, and the caller acts on it: the key is not
added to `still_applied`, so the ledger is rewritten without it. **The resource is forgotten
while still in effect, and no later sync will ever look at it again** — because the ledger is
the only record that Shall put it there. The `warn!` is the whole trace, and `warn!` is what
this same file's teardown comment already identifies as too quiet: *"a deletion the user cannot
see coming is the wrong shape, and `info!` is below the default filter, which is why this
teardown could delete five files under a summary reading `already up to date`."*

`firewall:` reached that arm. `extra_key`'s final `_ => stmt.kind().map(|_| stmt.key())` keys
every kind with a keyword, so deleting a `firewall:` line put `firewall:22/tcp` in the ledger and
the next sync's teardown shrugged at it. The port was in fact closed — by `Firewall::apply`,
which diffs the whole perimeter — so nothing broke, and the arm has been correct by luck since
the day `firewall:` was added. That is the worst kind of correct: the exhaustive version now says
`K::Firewall => Ok(())` with the reason beside it, and the reason is a fact about the design
rather than an accident of ordering.

**The probe's catch-all placed for ever.**

`in_effect` answers *is this already true?*, and its own doc comment states the stakes: `None`
means Shall cannot ask, **not** that the answer is yes — and a `None` is placed rather than
guessed. So `_ => None` at the bottom is not a neutral default; it is *re-apply this on every
sync, indefinitely*. Three kinds were answered (`service`, `link`, `shim`) and the rest arrived
there by omission. The comment above the function names exactly the cases where that is the right
answer — an adapter with no read-back, a `@decrypt`ed secret — but nothing distinguished those
from a kind nobody had got to yet.

**And the function already had the type in its hand.** `in_effect` takes `stmt: &Statement` at
its second parameter and then matches on a string it split out of the *key* — the fourth
parameter — to decide which arm to run. The typed answer was one method call away, and the
string was preferred because that is what `split_key` returns.

`ResourceKind` is that method call. Ten variants, `Display` and `FromStr` as the only conversions,
a hand-written `ALL` that a test drives every grammar keyword through so it cannot fall behind.
Both dispatches are exhaustive with no catch-all, and the arms that do nothing say why:

- `K::Setting` — the adapter has no "current value" command.
- `K::Repo` — answerable, but not for free, and deciding what a differing URL means is a ruling
  rather than a refactor. **Left `None` deliberately, and written down here so the next person
  finds a decision instead of a gap.**
- `K::Schedule` — provisioning is idempotent at the OS scheduler and cheap to repeat.
- `K::Firewall` — reconciled as a whole perimeter elsewhere; a per-line probe would be a second
  opinion about the same fact.
- `K::Exec`, `K::Generate`, `K::Dotfiles` — never keyed at all, and listed so the compiler keeps
  that true rather than a comment claiming it.

**One behaviour did change, and it is the safe direction.** A ledger row whose kind this build
does not have — a file written by a newer Shall, an edit by hand — is now kept and reported
instead of being silently dropped. Forgetting a row is the one outcome that cannot be undone: the
resource stays on the machine with nothing recording that Shall owns it.

---

**V.161 — Why the ledger key is a type, and why five of the nine "hand-rolled readers" were fine.**
*(Rule in II.30. Fixed 2026-08-09, `S57`.)*

The review that raised this counted nine places re-splitting `Statement::key()` on `:` and called
them nine instances of one bug. **They are not.** Reading them one at a time turns up two
different key spaces wearing the same punctuation, and that — not the count — is the finding:

- **`kind:subject`**, an extras-ledger key. `service:nginx`, `repo:apt:ppa:x/y`,
  `link:/home/u/.vimrc`. Three sites: `extras_lock::split_key`, `guard::extra_removal_pairs`,
  and `apply/extras::in_effect`.
- **`backend:name`**, a package key. `apt:jq`. Five sites: `core/state::is_held`,
  `model/resolve::same_package`, `verbs/cleanup`'s OS-essential filter, `verbs/plan::scoped_by`
  (a *third* space again — the lock-file ledger's `adapters:backends.toml`), and
  `verbs/packages::handle_info`.

Four of the five package-key sites are correct and stay: they split a key their own module
formatted, in a space where the prefix really is a backend. `state::is_held` even documents why
it compares halves in place instead of formatting a key — one heap allocation per declaration,
inside the planner's fan-out. `scoped_by` is about a different ledger entirely.

The fifth was a real instance of the repo's own rule — *one parser for `backend:name`, and
anything that splits on `:` and trusts the prefix is a bug*. `handle_info`'s "not installed"
message built its `shall search …` hint with `package.rsplit(':').next()`, which for
`web:https://example/x.deb` yields `//example/x.deb`: a suggested command nobody can run. It goes
through the grammar now.

**So the finding is the extras space, and there the type is the fix.** `ExtraKey` is the only
thing that formats a `kind:subject` string and the only thing that parses one. Two properties
fall out that no amount of careful splitting could give:

- **The producer and the reader are one construction.** `link_key` existed already, precisely
  because a `dotfiles:` tree asks the same question from the other end and *"must produce the
  same string or a teardown searches for a row nothing wrote"*. That was one string built in two
  places with a comment holding them together; it is now one constructor.
- **The split point is stated once.** At the **first** colon, because a `repo:` subject carries
  its own. Every hand-rolled reader used `split_once`, so they all agreed — but nothing said why,
  and `handle_info`'s `rsplit` is what the disagreement looks like when it happens.

**`extra_key` no longer builds from `Statement::key()`.** It builds from `kind()` and
`subject()`. `key()` is documented as the display form of a line; that it happens to equal
`kind:subject` for the keyword statements is a coincidence two functions were relying on, in a
file that already documents the one place it deliberately departs from `key()` (a `link:` is
keyed by its destination, not its source).

**The on-disk format did not change**, and the ledger still deserialises as strings rather than
as parsed keys. That is deliberate: parsing at load would make one unreadable row — a file
written by a newer Shall, a hand edit — fail the whole file, and `S56` had just established that
forgetting a row is the one outcome that cannot be undone. Parsing at the point of use lets the
row be reported and kept, and lets the guard go on counting and protecting it, because the guard
does not dispatch on kind and only the kind is unknown.

---

**V.162 — Why a capability matrix could not catch this, and why the fix is not a derivation.**
*(Rule in II.31. Fixed 2026-08-09, `S58`.)*

`winget` carries `upgrade_args: ["upgrade", "--all", "--silent"]` and an `OutdatedProbe` that
runs `winget upgrade` to find out what needs it. `scoop` carries `["update", "*"]` and its own
probe. Neither was registered `.with_upgradable(…)`. `choco` — the third Windows manager, in the
same file, forty lines away — was.

**So `shall upgrade` on Windows upgraded chocolatey packages and skipped the other two in
silence.** There is no error path: `as_upgradable()` returns `None`, and every caller reads
`None` as *this manager has no such concept*, which is the correct answer for `link:` and the
wrong one here.

**The matrix test asserted the loss.** `assert_caps(&reg, "winget", &[…])` listed five
capabilities and `upgradable` was not among them, so the omission was pinned as intended on the
day it was made. That is not a flaw in that test — it does its job, which is to notice a
capability *changing*. It is a demonstration that a check written from the code can only ever
say the code is what the code is. The question it cannot ask is whether the config and the
registration agree, and that question is where the answer was.

**Asking it turned two into three, and cleared two more.** The scan found five registrations
whose config declares an upgrade-all verb and whose builder omits `Upgradable`:

- **`winget`, `scoop`** — the reported pair. Real verbs. Registered.
- **`gem`** — `gem update` upgrades every installed gem. The same loss, unreported. Registered.
- **`pip`** — `pip install --upgrade` takes package names and fails without them. There is no
  upgrade-all to register; those args are the per-package form. **Correct omission.**
- **`bun`** — `bun upgrade` upgrades the bun runtime, not the packages bun installed.
  Registering it would make `shall upgrade` replace the user's toolchain while reporting that it
  had updated their packages. **Correct omission**, and the sharpest argument against the
  obvious fix.

**Which is why this is not a derivation.** The tidy version of this repair — read the capability
set off `ManagerConfig`, the way the custom-backend onboarder already does — would have
registered `pip` and `bun` too, and turned a silent omission into a command that does the wrong
thing loudly. It would also have given `bun` a `Searchable` whose parser is `|_| vec![]`, which
is the *inverse* of the mistake the onboarder's own comment warns about: turning "not
configured" into "no results". A mechanical rule over a field that means two different things
is not an improvement on a hand-written list; it is the same list with the reasoning deleted.

**So the fix is the list, plus the reasons, plus a gate.** Three registrations added, and a scan
that fails on any future config/builder disagreement unless the manager is named in `EXEMPT`
with the sentence explaining it. A second test checks that no exemption has outlived its
subject: an exemption for a manager that has since been registered, or renamed, silences nothing
and reads as if it does.

**Both scans are driven by an oracle** (II.23), over a planted body that declares the verb and
omits the capability, a planted body that does both, a planted body with an empty `vec![]`, and
a planted body using the `cfg.upgrade_args = …` spelling that follows `base_config` — because
the extractor has to read both spellings this file uses and a scan that only reads one would
pass by finding nothing.

---

**V.163 — Why the durability is one function and the preview policy is two.**
*(Rule in II.32. Fixed 2026-08-09, `S59`.)*

`utils/file.rs` opened with a paragraph that is one of the better ones in this repo — *"There
were two of these… A writer that honours the flag is no protection while a writer that ignores
it sits beside it, so there is now one."* It is about a real bug (`--dry-run adopt` recorded 112
packages as managed while correctly not writing the manifest that declares them) and it draws
the right conclusion.

**And the sentence it ends on was false in a way the paragraph made hard to see.** There were
four rename-into-place writers:

| | fsync | preview |
|---|---|---|
| `utils::file::atomic_write` | yes | via `persist` |
| `CommandExecutor::write_atomic` | **no flush, no sync** | via the VFS |
| `CommandExecutor::write_secret` | flush, **no sync** | via the VFS |
| `InstalledListings::save_to_disk` | no, deliberately | none, deliberately |

**A rename is atomic against a concurrent reader and says nothing whatever about power loss.**
The directory entry can reach the disk before the bytes it points at do. So `write_atomic` —
which is what writes a systemd unit, a `link:` target and every backend's state file — could
leave a zero-length file after a crash, while `registry.json` and the WAL went through `persist`
and survived. **That is the worst possible division of durability**: Shall keeps its record of
what it did and loses the thing it did, which is precisely the state `heal` is least able to
reason about.

**The fix is not "make it one writer", because two of the differences are real.** The two
preview policies answer different questions and both are correct:

- The config repo must not be touched by a preview at all. `persist` prints *would write …* and
  returns `false` so the caller can phrase its own message.
- A machine write must be *visible to the rest of the preview*. The executor diverts bytes into
  a dry-run VFS so that a previewed command reading a file a previewed command would have
  written sees it. Printing and stopping there would make every multi-step dry run wrong.

What was never legitimately plural is the **durability**, and that is now `durable_write`: dir,
temp file, write, flush, `prepare`, `sync_all`, rename. `prepare` is the permission hook, and it
runs before the rename because that is the only moment at which a mode change is not a window —
a `chmod` after the rename leaves world-readable plaintext at the target path for however long
it takes, and T5's whole argument is that "however short" is not an argument for a secret.

**`InstalledListings::save_to_disk` stays its own, and the reason is worth keeping.** It is a
cache: a torn file is a miss, an fsync per listing would put a disk barrier on the read path,
and its temp name carries the process id because *the rename is only atomic per writer* — two
`shall` runs sharing one temp path write into each other's file and rename the interleaving,
which is the torn listing the mechanism exists to prevent, arrived at by the mechanism.

**And the comment is replaced by a scan.** A paragraph asserting a singleton is exactly what was
wrong here: it was true when written, and nothing re-derived it. `tests/a_writer_that_reaches_
the_disk_goes_through_one_tests.rs` walks `src/`, ignores test modules, and fails on any
rename-into-place outside the two allowed files — each of which carries the sentence saying why
it is allowed, checked to still be needed. Driven by an oracle over a planted offender, a
planted innocent, and a planted file whose only offence is inside `#[cfg(test)]` (II.23).

**One smaller correction rode along.** `verbs/plan.rs` claimed *"Every ledger these commands
write goes through `utils::file::persist`"* as ground for a dry-run claim. Scoped to those
commands it is true; read as a claim about the program it is not, and the four-writer paragraph
above is why somebody would read it that way. It now says which scope it means.

---

**V.164 — Why the removal arm asks the same question, and why the register had to be told.**
*(Rule in II.33. Amended ruling in `U41`; fixed 2026-08-09, `S60`.)*

`U41` was answered on 2026-07-27 and its answer said both rollback arms compensate. Then `LX-3`
changed one of them. `Prior::Absent` stopped being permission to remove, on the argument — a good
one, written into the code as a comment — that *"was not here before this run" and "is not wanted
now" are different facts, and the manifest holds the second one*. **Nobody told `decisions.md`.**
So the register recorded a closed ruling while the code ran two arms under two rules, and the
only way to find out was to read both arms.

That is the process failure, and it is the reason the review flagged this at all. The code change
was right.

**The asymmetry itself.** The install arm consulted the plan; the removal arm reinstated
unconditionally. Asked plainly: *why can rollback know that an install should stand, but not that
a removal should?* The answer given at the time was that a removal has no equivalent fact — and
that answer is wrong, in the owner's words: *"we could have figured it out the same way we know
to delete it: it's not there."*

**Exactly so, and it is the same set.** A `Remove` node exists because the planner found the
package in `present − desired`. The set that authorised it is the plan's own `Install` nodes —
what this plan intends the machine to end up holding — and the package's absence from that set is
still true when the rollback fires. `false` is an answer. Both arms now call
`plan_intends_present`, which is one function so that the symmetry is a property of the code
rather than of two comments agreeing.

**What the second half costs.** A package the user had, that this run removed, stays removed
after a failed transaction. That is a real loss and it is accepted for a stated reason:
**generations and snapshots are the durable put-it-back**, a pre-sync restore point is taken on
every run, and `shall history` reaches it. Re-installing at whatever version is newest — which is
all the WAL can support today, because it records the removal and not the version — is a
different package wearing the same name. A durable `Prior` is the alternative and it is deferred
rather than rejected; when it exists this ruling is worth reopening, because part of the case for
leaving a removal alone is that putting it back imprecisely is worse than not putting it back.

**And two scopes are exempt, which is where this nearly went wrong.** The first implementation
gated on `declared` being non-empty, which would have been silently correct for `sync` and
silently catastrophic for `rebuild`: a rebuild's *down* phase is a removals-only graph, so its
`Install` set is empty, and "not in the set" would have meant "leave every declared package
deleted". `GuardScope::reconciles()` is the discriminator instead, exhaustive over all twelve
scopes with the two exceptions written out — a rebuild's removal phase is the first half of a
reinstall of declared packages, split in two only so the `Remove` and the `Install` cannot race;
and an `uninstall` was typed by a person rather than derived from a manifest. A scope added later
does not compile until it says which it is, because inheriting `true` here means inheriting *a
failed run may leave your software deleted*.

---

**V.165 — Why the shipping surface accumulated six of these, and what they have in common.**
*(Rule in II.34. Fixed 2026-08-09, `S61`; licence ruled as `Z1`, `S62`; the installers made
installers 2026-08-10, `S79`.)*

**The seventh, found when the first release was actually attempted.** Both installers opened
with *"the 30-second first run"* and then ran `cargo install --git`, which resolves 448 crates
and compiles them under fat LTO. Nobody has ever measured thirty seconds doing that. The header
was not a lie anyone told; it is what happens when the shipping surface is *read* — the sentence
was written for the program that was going to exist, and the release that would have made it
true had never been cut. So the claim aged into a falsehood in a file whose whole audience pipes
it, unread, from a URL.

Two things had to be true at once for the fix, and only one of them was code. The release job
had never run — its only trigger is a `v*` tag — and it would have published four binaries all
named `shall`, because a GitHub asset takes the basename and every target builds that same
basename. Three platforms out of four would have received the wrong architecture from a release
page that looked complete. **An untested release job is not a risk that shows up as a failure;
it is one that shows up as a success serving the wrong file.**

And the ordering, which reads backwards until you see it: the toolchain check moved *after* the
download attempt. Requiring Rust before knowing whether a prebuilt binary was available made
"install this program" mean "install a toolchain first" for every user on a platform that has a
published build — a package manager whose own installation needed a compiler.

None of the six is hard. That is the point of collecting them under one reason: they are all in
the part of the repository that **nothing else in the repository checks**, and each of them
survived exactly as long as it took somebody to read the file rather than run it.

**Two of them are the same variable failure, in both directions.** `install.sh`'s header list is
the entire interface of a script users pipe from a URL — there is no `--help`, no man page, and
nobody clones the repo to read the body. It listed `SHALL_BIN_DIR`, which appeared nowhere else
in the tree, and omitted `SHALL_REF`, which both installers read and both comment at length.
A promise nothing keeps and a feature nobody is told about, in the same eight lines.

`SHALL_BIN_DIR` is **wired in rather than deleted**, and the wiring is the interesting part:
cargo cannot be told "install the binary here", only "install under this root, which means
`$root/bin`". So a directory ending in `bin` becomes a `--root` of its parent, and anything else
gets a staged install and a copy. Computing that is three lines; demanding the user compute it
would be documenting a variable that means something other than what it says, which is where
this started.

**One is a gate that passes hardest when its input is missing.** `grep -q "^$MSRV"` with an empty
`$MSRV` is `grep -q "^"`, which matches every line of any input — so a `Cargo.toml` whose
`rust-version` had been renamed, moved or requoted would make `release-check.sh` report GO after
running `cargo +"" check`. The `.ps1` twin had the explicit emptiness guard. Same file, same
week, two authors, one of them thought about it: the twin-divergence shape `CLAUDE.md` is about,
and the reason both scripts now say *"change one, change the other"* in every block that exists
twice.

**One is a number tracked by hand in six places.** `Cargo.lock` holds 448 crates; four files said
380 and two said 452. This repository contains a 226-line script written because *two* files
tracked one number by hand. The count is prose here rather than an assertion, which is why it
drifted — and correcting it is worth the line only because each of those six sentences is an
argument that depends on the number being roughly right.

**One is a suppression addressed to nobody.** Two files carry `# shellcheck disable=SC2086`. No
shellcheck ran anywhere: not in CI, not in either release script, not in a pre-commit hook. So
those two directives were comments shaped like gates, and eleven shell scripts went unlinted —
including `docker/integration/run.sh` and `harness-logic-test.sh`, which **decide pass or fail
for every backend on every distro**. A quoting bug in a harness is not a broken script; it is a
wrong verdict about the product, reported as a green tick. The linter runs now: hard in CI, soft
in both release scripts so a developer meets it before a red push, at `-S warning` because a gate
switched on over existing code and arriving red on cosmetics is a gate somebody disables in
week one.

**And one is a path whose first execution would have been the thing it exists for.** The
`release` job triggers on `refs/tags/v*`, and no tag has ever been pushed. Every other job in
that file has run hundreds of times; this one has run zero, and it is the single step between a
green build and a stranger downloading a binary. It gained `workflow_dispatch`: the same
download, an assertion that the artifacts are actually there — which the tagged path never makes,
because `action-gh-release` only fails on an empty glob *after* the tag exists and the release is
half-made — and then a stop, publishing nothing. One job, one path, one `if`.

**The licence (`Z1`, `S62`) is not one of the six**; it is a ruling, and it is here because it is
the last thing between this repository and other people having a right to the copy the install
script gives them. Two placeholders went with it, and both were honest while they stood:
`publish = false`, which said *crates.io would refuse this anyway*, and `deny.toml`'s
`private = { ignore = true }`, which existed so a lint config would not answer the owner's legal
question by implication. Neither has anything left to stand for.

---

**V.166 — Why four unrelated one-liners share a reason.**
*(Rule in II.35. Fixed 2026-08-09, `S63`, `S64`.)*

They are all the same failure: **a rewrite that changed something nobody asked it to change, and
a comment that had made peace with it.**

**The CRLF one is the clearest.** `model/edit.rs` rejoined with `"\n"` at three sites, so every
`shall install` on a Windows-authored module rewrote the whole file's line endings. Nobody
noticed because the *content* diff is one line — it is `git diff` that shows four hundred, and
by then the change has been committed. The sharp detail is one file over: the grammar accepts a
leading BOM, and the comment explaining why says *because that is what Notepad writes*. Notepad
writes CRLF as well. Somebody thought carefully about meeting that editor halfway and then
delivered one half of the courtesy.

**The `setting:` scope is the same shape with worse consequences**, and it came with a comment
that stated its own cause: *"Scope is not carried on a removal (only names are), so this resets
the store's default scope — which is where an unscoped declaration wrote, the case that exists
today."* Both clauses are true. The conclusion does not follow: `@scope=system` is a spelling the
grammar accepts and `scope_of` writes a bespoke refusal for, so the case that exists today is not
the only case. Deleting such a line reset the *user* key, left the machine-wide value in place,
and said the line had been undone.

The fix is where the fix for this class always is — **carry the fact to where the decision is
made**. By teardown time the declaration is gone, so the ledger key is the only record left, and
the key now carries the scope. An unscoped key means the store's default, which is exactly what
every row written before this meant, so nothing on disk changes meaning.

**`shall why` re-resolving per match** is the same instinct in the read path: the function that
needed the configuration fetched it itself, which is right until it is called in a loop. Two
backends carrying one name — an ordinary answer — read every file you own twice.

**And `md5`** is a supply-chain line item for one cache-directory name, beside `sha2`, in a tool
whose pitch is being careful about exactly that.

**The fourth is not a bug in the code and is here because it is the most dangerous.** Three
source scans justify themselves with *"`verbs/` is private to the binary"*. It is
`pub mod verbs;`, and `verbs_are_reachable_tests.rs` imports through it. The scans are still the
right technique — the claim is about *every* call site, and calling one proves nothing about the
other forty — but they were resting on a reason that is false, and a check with a false reason is
the one the next careful reader deletes. Fixing the reason is what keeps the check.

---

**V.167 — Why the listing is shared and the lookup is not moved.**
*(Rule in II.36. Fixed 2026-08-09, `S65`.)*

The review measured this one correctly and prescribed the wrong fix, and the difference is worth
writing down because the wrong fix is the obvious one.

**The measurement.** `installed_sets` builds `HashMap<backend, HashSet<name>>` once per backend
for the `absent:` loop, and `identify_needed_actions` then throws it away and calls
`q.info(&spec.name)` per declared spec. Thirteen backends implement `info` as list-then-find, and
`InstalledListings::once` returned an owned `Vec<Package>` — so each of those calls cloned the
whole listing. On a 256-line winget config against a 280-package listing: **~71,680 `Package`
clones and 256 mutex acquisitions, to answer 256 questions**.

**The prescription** was to have `installed_sets` return `HashMap<name, Package>` and have
`spec_is_missing` look packages up in it. That deletes the clones by deleting the call — and with
it, everything the call was doing:

- `generic.rs::info` matches choco, scoop and winget **case-insensitively**, because "choco
  installs `wget` yet lists the Title `Wget`" — its own comment, and its own bug, fixed once
  already. It also accepts winget's bare moniker (`jq`) for a vendor-qualified id (`jqlang.jq`).
- `go.rs::info` matches a module path by its trailing binary segment, so `fzf` finds
  `github.com/junegunn/fzf`.
- `spec_is_missing` reads `properties["channel"]` and `properties["classic"]` for `@channel` and
  `@classic` drift. Both come from `snap info`; `snap list` carries neither.

A map keyed by the listed name answers none of those. Every `winget:jq` declaration would read as
absent, schedule an install, and winget would report it already installed — on every sync, for
ever. That is the exact failure class three separate entries in this file already record.

**So the clone was the cost, not the call.** The memo holds `Arc<Vec<Package>>`, `info` borrows
it, and one `Package` is cloned instead of two hundred and eighty. `list_installed` still returns
an owned `Vec` for the callers that genuinely consume one; `installed_listing` is a new door
rather than a migration, which is why thirteen backends changed by two lines each and no matching
rule moved.

**What was scoped and not taken, with reasons**, so the next reader does not re-derive them:

- **`guard::essential_names` runs twice per removal command, unmemoised**, a subprocess per
  backend, under a comment calling it cheap. The finding is right and the fix is not a memo: the
  essential set is derived from the installed database, so a cache needs the invalidation
  `InstalledListings` already has and a home that respects it. Caching a stale essential list into
  a removal decision is the worst thing in this program to get wrong, and picking the home is a
  design decision rather than a line.
- **`go.rs` and `psresource.rs` spawn one subprocess per item** where the tool takes a list.
  Batching them changes argv, and argv changes in this repo are settled by capturing real output
  in a container, not by reasoning — twice now.
- **`[profile.ci]` with `lto = false`** would stop 99 test binaries each taking a fat-LTO link.
  It also changes the flag both release scripts must pass, which `grade6_gate_parity` compares,
  and buys CI wall-clock rather than anything a user sees.
- **Three sites download an unbounded response body into RAM** (`github.rs`, `web.rs`,
  `appimage.rs`) with no `content-length` check and no cap. Streaming is the same code shape, but
  a cap is a number a user meets as a refusal, and `core/download.rs` has scheme policy and
  checksum policy precisely because those were rulings. **This one is a ruling too**, and it is
  raised rather than picked.

---

**V.168 — Why one test target, and what merging them found.**
*(Rule in II.37. Fixed 2026-08-09, `S67`.)*

**The repository had already diagnosed this, in writing, at one-hundredth of the scale.**
`mock_providers/mod.rs` opens:

> *"Cargo auto-discovers every `tests/*.rs` as its own test target, so at the top level this
> became a 716 KB binary containing **zero tests**, linked with `lto = true` and
> `codegen-units = 1`, and its 312 lines were compiled nineteen times."*

Correct, and fixed — by moving one file into a directory. What nobody then asked is why the same
sentence is not true of the other hundred files, because it is: each is linked against a
100k-line crate and a 448-crate graph, and **36 of them never call the library API at all**. They
spawn `CARGO_BIN_EXE_shall` and read its output; the library they link is dead weight in every
one.

**The number that made it undeniable was not a benchmark.** `target/` reached **194 GB** and
filled a 944 GB disk, and `rustc` died mid-build with `IO failure on output stream: no space on
device`. After the merge it is **19 GB**.

**`autotests = false` rather than moving files into a subdirectory**, which is the other way to
do this. Moving them would rewrite every `include_str!("fixtures/…")` and `include_str!(
"../src/…")` path in the suite and lose the one-file-one-sentence naming at a glance in `ls`.
Turning off auto-discovery costs one line in `Cargo.toml` and one `mod` per file, and every file
stays exactly where it was.

**The cost is real and is why the guard exists.** With auto-discovery off, a test file nobody adds
to `tests/main.rs` is a file that never runs, and it looks exactly like a file that passes.
`every_test_file_is_in_the_suite` reads the directory, reads the module list, and fails on the
difference. That check is the whole justification for the arrangement — this repository has
S-entries for four separate ways of losing a check silently, and adding a fifth to save link time
would be a poor trade.

**Two things had to change, and both are improvements.**

*The inner attributes.* `grader_shim_exit_code_tests.rs` and `pty_tests.rs` open with
`#![cfg(windows)]` and `#![cfg(target_os = "linux")]`, which cannot survive a file becoming a
module. On the `mod` line the gate is strictly stronger: the module is not compiled at all
off-platform, where before it was compiled to nothing.

*The two lines that only compiled by accident.* `dry_run_every_verb_tests.rs:541` and
`grade6_gate_parity_sees_whole_jobs_tests.rs:53` used `String + &String` and
`String + &Cow<str>`. The first is ordinarily legal via deref coercion — **unless another crate
in the compilation unit adds a second `Add` impl for `String`, which makes the target type
ambiguous and stops coercion being attempted.** `rhai` depends on `smartstring`, which does
exactly that. As their own binaries those two files never referenced anything that pulled
`smartstring`'s metadata in, so the impl was never loaded and the coercion went through. In one
unit it is always loaded. Both lines say `format!` now, which is what they meant; the point worth
keeping is that **a per-file binary can compile code that the same code cannot compile beside its
neighbours**, and that is a fragility the merge removed rather than one it introduced.

---

**V.169 — Why sixteen copies is worse than the lines suggest, and what the merge nearly cost.**
*(Rule in II.38. Fixed 2026-08-09, `S68`.)*

Twenty-five lines written sixteen times is four hundred lines, which is the cheap way to describe
it and the wrong one. **The expensive part is that no two copies agreed**, and the disagreements
were all in the direction of *less isolation*:

- **`current_dir`: 3 of 16.** The other thirteen ran `shall` with the test harness's working
  directory — the repository root. Shall reads `shall.txt` from the working directory as a
  project-local shell manifest, so those thirteen were one stray file away from behaving
  differently, and would have blamed the product.
- **`HOME`/`USERPROFILE`: 3 of 16.** The other thirteen let `~` resolve to the machine's real home
  directory. A test that declares `link:… @target=~/.vimrc` and runs `sync` was writing to the
  developer's actual dotfiles. It passed anyway — because the assertion was about what Shall did,
  not about where — and on CI it passed or failed depending on whether the checkout happened to
  sit under `$HOME`.
- **`cfg()`: 11 of 16.** The five without it wrote `root.join("config")` at each call site, which
  is not a bug and is how the other two started.

The union is not a compromise here: **every one of the three is correct in the strong direction,
and the copies that had it are the ones that were right.** A test process that can see the
developer's home directory is a test whose result is about that machine.

**What the merge nearly cost, and how it was caught.** The first conversion dropped each file's
`new` on the grounds that it was shared boilerplate. Five of them were not: they created `tree/`,
`src/` and `dest/` directories, appended `use extras` to the generated profile, and seeded link
declarations before the first assertion. Deleting that would have left five files whose tests
still compiled, still ran, and no longer tested what they were named after — **the exact failure
this change exists to prevent, committed by the change itself**. It was caught by diffing every
removed `new` against the shared one before running anything, and those five keep a free
`setup(name)` built on `Fixture::new`.

The lesson is the one the repository keeps relearning and is worth stating as a rule of the
technique: **a de-duplication is only safe when the things being merged have been shown to be
identical, one at a time.** Sixteen functions with the same name and shape are not sixteen copies
of one function until somebody has read all sixteen.

**V.170 — Why the four assertions had to stop being nine copies, and what stayed behind.**

The bug this prevents is `S48` seen from the supply side. Three gates asserted over a walk whose
predicate had stopped matching, found nothing, and reported clean — and the reason all three
were written that way is that the floor is the one assertion of the four you can omit without the
gate looking wrong. Unexplained-sites and stale-entries both have an obvious failure mode a
reader imagines while writing them. *The walk read enough files to mean anything* has none: it
guards a state the author has never seen, so it is the line that does not get typed on copy nine.

Concentrating the four is what makes the omission impossible rather than unlikely, and it is why
`Ledger::audit` panics on a floor of zero instead of treating it as *no floor requested*. A
permissive shared helper would be strictly worse than nine copies: it fails nine gates at once,
and it fails them silently, because a delegated assertion looks identical to a working one at the
call site. So the helper has an oracle of its own, and each of the four is driven over a planted
input built to violate it and watched to panic.

**What was deliberately left alone matters as much.** Four tables have the same `&[(&str, &str)]`
shape and are not exemption tables: one accounts for every mutation site rather than excusing
any, one lists patterns that must appear exactly once, one is a classification with no scan
behind it, and one pins its entries to file and line. Converting those would have produced a
larger diff and a smaller amount of truth. The rule is that a helper is for the assertions that
are the same, not for the tables that look the same.

**And the assertions a shared helper cannot make stay at the site.** `os_native_argv`'s check
that an exemption does not contradict a row three lines below it is the `helm` case — exempt
on the grounds that a row *“would pass on the remove alone”*, which stopped being true
the moment rows could carry options, with nothing to say so. The ledger's stale check would catch
it, and would report it as *“no longer has no row”*, which sends the reader to the
wrong place. So it runs first, with a line saying why. That ordering is the general form: where a
site knows more than the helper, the site asserts first and the helper cleans up behind it.

**V.171 — Why one field kept thirty others in Rust, and what a row can now get wrong.**

Twenty-three backends were already data and nobody could act on it. `fn register_yarn` opened
with `ManagerConfig { name: "yarn".into(), install_args: vec!["global".into(), "add".into()], … }`
and closed forty lines later having said nothing a `[[backend]]` row could not say — except in
one field. `parser` takes a `ParserSpec`, which *describes* a shape; the listing yarn prints
needs `ws_name_version`, which is a *function*. A TOML row had no way to write a function's name,
so the parser field forced the whole registration into Rust, and once it was in Rust the argv
went with it. Multiply by twenty-three and that is 782 lines of struct literal justified by one
missing indirection.

`src/parsers/named.rs` is that indirection: a name-to-function-pointer resolver, one per call
site in `build_capabilities`, over readers that already existed and were already tested. Nothing
about the parsers changed. What changed is that a row can now say which one it wants.

**A row can be wrong in ways a function could not, and that is the real cost of the trade.** The
compiler checked `parser: Arc::new(ws_name_version)`; it cannot check `reads = "ws_name_version"`.
Worse, the field is an `Option` whose `None` has a meaning — *use the described parser* — so a
misspelling is not a load error, not a warning, and not a panic. It is a backend that registers,
advertises `Queryable`, runs its listing command, parses it with one-bare-name-per-line, and
reports a machine where every installed package is either missing or misnamed. That is Q40's
class exactly, arriving through a door Q40's fix did not cover, and it is why the gate for it
holds a floor on the number of readers it found: a scan that stopped seeing the fields would pass
as loudly as a correct one.

The same shape repeats three more times, each with a `None` that means something reasonable and
therefore hides something wrong. `searches` absent means *this manager has no catalogue*, which
is true for `krew` and a silent lie for anything with `search_args`. `parser` absent means *one
name per line*, which is a real shape and the wrong answer for every manager that prints a
version. `install_source_option` absent means *installs by name only*, which the grammar
independently believes or disbelieves in `capability::install_source_key` — and when the two
disagree the user either gets a refusal for a line the backend could have run, or an accepted
line whose source is dropped on the floor. Four `None`s, four gates, each watched to fail on a
planted defect before it was believed.

**Registration order is a decision, not an accident.** Rows go in first so that a hand-written
registration overwrites one, which is the safe direction: the Rust is the more specific thing and
the thing more likely to exist for a reason. But *safe direction* is not the same as *fine*, and
a name held in both places is two of everything by definition. The gate refuses it rather than
letting the order of two calls quietly pick a winner.

**V.172 — Why a fixture is a required column, and what asking the containers cost.**

The claim under `ws_name_version` was that cabal, spack, pub, krew, helm, guix, luarocks and uv
print the same shape. It had one piece of evidence: `NAME VERSION` / `foo 1.2.3` /
`bar 0.1.0 some-desc`, three lines somebody typed, labelled `helm`. Seven managers were being
read through a parser nobody had shown their output to, and the tests were green the whole time,
because a test written from the same imagination as the parser agrees with it by construction.

An afternoon of containers produced four defects. None of them is subtle once you have the bytes,
and none of them is findable without: `uv` prints its executables under each tool, `cabal` writes
configuration chatter to stdout, `nimble 2` changed its version record, and a header-only listing
was refused rather than answered. That ratio — four defects, zero found by reading the code
first — is the argument for the column. A parser is a claim about a program somebody else wrote,
and the only evidence that settles it is what that program printed.

**Why `source` and why a ratchet.** The failure mode this replaces is not *no fixture*, it is
*a fixture that looks like evidence*. Bytes reconstructed from a README are the same characters
as bytes captured from a tool and carry none of the authority, so the difference has to be
written down or it does not exist. Seven of the twenty-two are reconstructions today. The ceiling
is not a budget for more; it is the number that has to go down, and there is nowhere in the gate
to record *"I raised it"* that does not read as what it is.

**Why the fixture runs through `parser_for` and not through its own resolution.** A check that
re-derived which reader a row uses would be testing the re-derivation. `build_capabilities` calls
`parser_for`; so does the fixture gate; the object the bytes go through is the object the machine
goes through.

**Why the header fix went into `is_noise_line`.** Ten readers each wrote `if is_header_token(name)
{ return None }` inside their `filter_map`, which removes the header from the *packages* and
leaves it in the *candidates* — and `or_unrecognised`'s whole rule is that candidates yielding
nothing is a refusal. So ten readers agreed that a header is not a package and all ten still
refused a listing that was only a header. The judgement had to move to where the candidates are
chosen, which is the one place all ten pass through. Fixing it in `ws_name_version` alone would
have fixed helm and left the other nine.

**Why the two dead helpers were wired rather than deleted.** `parsers/utils.rs` had no callers,
and both of its functions had hand-rolled equivalents elsewhere — which is not dead code, it is
*two of everything with one copy asleep*. `split_columns` became the `quoted` option on
`ParserSpec::Columns`, a shape a user's row genuinely could not express for a Windows manager
that prints `"7.3.4 (x64)"`; `extract_version_bracketed` replaced `trim_matches('(' | ')')` in
mas and gem. Both callers kept a fallback to their old behaviour on a line with no brackets,
deliberately: extracting returns `None` there, `None` would drop the package, and a dropped
package is a removal while a wrong version is only drift.

**V.173 — Why the three downloaders' removal had to be one, and why the wording gained a clause.**

The duplication was not suspected, it was *documented*: `appimage.rs`'s test header describes
its own removal as `web.rs`'s with the D5 handoff taken out, same state file, same two deployed
paths, same re-insert-on-failure rule. A comment that accurate about a copy is a comment that has
given up on removing it.

What makes this worth merging rather than tolerating is the last of those four steps. **Putting
the record back when a delete fails is the non-obvious one**, and it is the one whose absence is
invisible: the removal reports failure, the state file says the thing is gone, and every
subsequent run agrees with the state file. All three had it. Three copies of a subtle rule is
three chances for the fourth downloader to be written without it, which is the shape of every
family bug in this repo.

**The cache ordering was a real difference, not a cosmetic one.** Two of the three cleaned the
cache inside the success branch and one had the branch nested differently; the shared function
makes *success first* structural. A cache dropped beside a file that would not delete costs a
re-download and buys nothing.

**Why the sentence says both halves.** `github:` and `appimage:` said *still installed*; `web:`
said *still on disk*. Choosing one would have silently downgraded a message a user reads at
exactly the moment something went wrong — and neither is complete: a `.deb` handed to dpkg is in
a package database *and* on the filesystem. Saying both costs four words and loses nothing that
any of the three used to report. Two web tests pinned the old wording and were updated; that they
existed is why the change is a considered one rather than an accident.

**Why the identity rule needed a gate and not a comment.** It already had a comment — a good one,
naming `btrfs:` and `web:` as the same shape and explaining exactly what went wrong. The comment
did not stop the bug from being possible again, because nothing runs a comment. What the gate
adds is that it was watched to fail: the basename bug was planted back into `fetch_installed` and
the test caught it, which is the only evidence that a green run means anything.

**Why PyPI became a variant.** `PipSearchable` and `node_registry` are the same idea — this
backend's search is an HTTP call — and one of them had a `SearchSource` variant while the other
was a struct bolted onto one registration. That asymmetry is what U2 is about: a user's row could
say `search_source = "npm_registry"` and could not say `"pypi"`, for no reason except which of
the two got written second. The parser did not change; it stopped hard-coding `pip` as the tag,
so the backend that asked is the backend the answer is labelled with.

**V.174 — Why one confirm, and why the third answer is an argument.**

The three steps were not worth merging on their own. Six copies of *check a flag, check a
terminal, call dialoguer* is ten lines of duplication and nothing more. What made it worth doing
is that the six were not the same: **each had decided, separately, what happens when there is no
terminal**, and two of the answers were opposites.

Refusing is right for a rebuild — a run that would have asked before removing software and
cannot ask must stop. Declining is right for bootstrap — *there is no package manager and nobody
to approve installing one* should not fail the sync, it should skip the offer. Both are correct;
neither is a default; and a copy that has not thought about it produces `dialoguer`'s bare
`IO error: not a terminal` attached to a run that was about to change the machine, which is what
`snapshot_restore`'s gallery did. Making the answer an argument means the question cannot be
skipped: `Unattended` has no default variant, so a seventh call site has to say which it is.

**Why the sentences stay at the call site.** A shared refusal message would have to be generic,
and a generic refusal is the failure mode this repo has a rule about: it names no verb, no flag
and no way forward. The shared part is the machinery; the sentence is the site's.

**Why the prompt gate's floor came down instead of the gate being deleted.** It scans for
`.interact()` and demands an `is_terminal` above it, and merging six sites into one took its
count below its own floor — the exact shape of failure it was built to catch, arriving from the
good direction. Lowering the floor without replacing the coverage would be losing the check to a
refactor, so `only_one_place_asks_for_a_yes_or_no` came with it. *There is one confirm* is
strictly stronger than *every confirm is guarded*: the first makes the second true by
construction, and it also catches the copy that is guarded correctly and still decides the third
answer for itself.

**Why the refusal gate learned a second constructor rather than being worked around.** After the
merge, the six sites write `Unattended::Refuse("Refusing to …")` and none of them constructs an
`Error::Refused` within the gate's eight-line window. Rewording the messages to dodge the scan
would have been a lie; teaching it that `Unattended::Refuse` becomes `Error::Refused` is true,
and the module carries a test that proves it by matching the variant rather than the text.

**V.175 — Why the reader is a type, and why an absent flag stopped meaning false.**

**The bug the `Output` type prevents.** Two `--json` defects are already recorded in this file:
`sync --dry-run --json` printing `already up to date` in English on a converged machine, and
`check --json` prefixed with a plain-text note. Both were written under a `!json` guard, and
that is not a coincidence. `!json` reads as *not machine-readable*, so an early return placed
under it feels correct — the human path is the one you are thinking about. `out.is_human()`
reads as *a person is here*, and the same early return under it is visibly a claim about who is
watching, which is the claim that was wrong. The type does not stop anyone writing the branch;
it stops the branch reading as though it were about something else.

**Why the bool was also unsafe on its way in.** Four call sites wrote `handle_sync(app, false,
false, false)`. Three adjacent booleans have six orderings and one meaning, the compiler cannot
tell them apart, and `locked` versus `upgrade` is the difference between converging to the
lockfile and taking whatever the managers offer today — the exact decision Z2 exists about. The
fix is not vigilance; it is that `SyncMode { locked: false, upgrade: false }` cannot be
transposed.

**Why `Some(cli.dry_run)` was worse than it looks.** An `Option<bool>` parameter says *the caller
may have no opinion*, and every caller of `merge_cli_overrides` had an opinion it did not have.
A clap `bool` flag is false both when the user said nothing and when the user could not have said
anything, and there is no third state to pass along; wrapping it in `Some` turned "no opinion"
into "off, definitively", and definitively-off wins over the file. The result is five documented
config keys that parse, validate and then evaporate — the worst kind of dead setting, because
`Config::from_file` accepts it, `deny_unknown_fields` blesses it, and the run behaves as though
it were never written. `main` even carried a comment four lines below the call saying a
`dry_run = true` in `preferences.toml` "counts too". It did not.

**Why `|=` rather than restoring the `Option`.** An `Option<bool>` would be honest only if some
caller could produce `Some(false)` meaningfully, and none can: there is no `--no-dry-run`, no
`--no-yes`, no `--no-allow-mass-removal`. Keeping the `Option` would preserve the shape of a
choice nobody can make, and the next caller would fill it the same wrong way. `|=` says the only
thing the command line is able to say.

**Why the tests walk five fields instead of pinning `dry_run`.** The reported symptom would have
been `dry_run`, and a test for `dry_run` alone would pass over the four siblings — including
`yes`, which decides whether the machine asks before it changes, and `allow_mass_removal`, which
decides whether the guard has a ceiling. The table in `flag_pairs` is the unit under test: a
sixth flag added tomorrow has to join it, because `a_flag_that_was_passed_turns_the_setting_on`
is written over the table rather than over any one name.

**V.176 — Why the second copy is the bug, not the duplication.**

**Two predicates is not redundancy, it is a fork.** `prunes()` and `is_active()` differed by one
clause and were called from different places — the caller in `verbs::mod` used the first to
decide whether to prune at all, and `select_deletions` used the second to decide what to delete.
So the safe reading and the destructive reading of the same policy both existed, and which one
you got depended on how the pruning was reached. That is worse than either being wrong on its
own: a keep-only policy was harmless through `sync` and destructive through the history prune,
and no amount of reading one function tells you that.

**Why the symlink clause is the function.** `path.is_dir()` is a question about the *target*.
Asking it in order to decide how to delete the *link* is a category error that reads as
completely ordinary code, and its consequence — `remove_dir_all` on the target — is silent, total
and outside anything Shall declared. `link:` had already been written twice with the clause,
which is the tell: the code that meets real symlinks knew, and the shared helper written away
from it never learned.

**Why the Windows arm arrived with the fix and not before.** `symlink_metadata` on a directory
symlink reports `is_symlink()` true and `is_dir()` false, so the corrected branch sent it to
`remove_file`, which Windows refuses. The bug and its repair have opposite platform signatures —
Unix would have passed the whole way through — and only a test that makes a real directory
symlink and deletes it sees either. That test is why `remove_deployed_path`, which `link:` was
just routed through, did not ship broken on Windows.

**Why a writer and a reader of the same file must share the header parser.** `active` is read by
`gated.rs` and rewritten by `profiles.rs`. The reader refuses a non-`when` block header; the
writer treated one as a `when` with an empty predicate, which evaluates false, which means the
names inside it are off. The two are only reachable together — you read the file, then you edit
it — so the disagreement is not theoretical, it is one command away, and it fails in the
direction where a user's declarations quietly stop applying.

**Why "wire it in" beat "delete it" for the three dead helpers.** Each was the *better* version
of something the tree already did longhand: `ensure_dir` had the error message the two dozen
inline `create_dir_all`s lacked, `force_remove` had the already-gone-is-fine rule five call sites
each re-derived, `read_lines_filtered` had the comment rule that `.gitignore` handling was
open-coding against the raw text. Deleting them would have removed the answer and left the
question in twenty-odd places. Giving them the one improvement each needed — a path in the error,
a symlink clause, a pure half — and then routing the longhand through them removes the question
instead.

**Why `Journal::new` was kept and given a caller.** It is not a spare constructor; it is where
the rule *the WAL lives beside the registry* belongs. That rule was a comment above a caller
which derived both paths by hand, and the last time it was left to a caller the registry got
isolated for tests and the WAL did not — 733 KB of test noise appended to a developer's real
journal, and then a format change made the file unparseable and bricked every test at bootstrap.
A rule enforced by a comment above one call site is not enforced.

**V.177 — Why shell was the wrong language for six of these, and why one of them stayed put.**

**A shell pipeline fails silently in the direction of "pass".** `grep -c` prints `0` and exits 1
when it matches nothing, so `COUNT=$(grep -c … || echo 0)` captures the two-line string `0\n0`,
both numeric guards die with *integer expected*, `[` returning an error takes the else branch of
each `if`, and the script reaches its success message. That is not a hypothetical: it is written
in this repo's own comments, about this repo's own mutation gate, in exactly the total-collapse
case the guards exist to catch. Rust does not have a comparison operator that treats a parse
failure as *the good branch*.

**And a shell pipeline can be blind to the byte it is looking for.** MSYS grep opens a file in
text mode and normalises CRLF before matching, so the CRLF gate never fired on Windows — the one
platform where a developer's editor writes CRLF into the working tree. The shell version needed a
self-test that plants a CRLF file and checks its own detector before believing a `no`. Reading
the bytes needs no such ceremony.

**Timing is the other half.** These predicates ran at the end of a release script or in CI, which
means the feedback arrives after the work is done, from a log. In `cargo test` they run beside
the twenty-seven other gates that read `ci.yml`, `target-state.md` and `src/` — the same second,
the same command, the same failure format.

**Why gate parity stayed where it was.** It had already been ported, when the shell predicate was
caught comparing basenames while CI ran the mutation gate against two different harnesses. The
Rust successor keys on the whole invocation. Writing a second Rust version because the shell one
was on the list would have produced three implementations of one question inside the change whose
entire subject is that pattern — and it would have looked like progress, because the count of
shell lines would have gone down.

**Why the floors are not optional here.** Every one of these gates reads a *list*: the scripts in
a directory, the mounts in a workflow, the Dockerfiles in a folder, the functions in a harness. A
list that comes back empty makes every one of them pass. That is II.23's shape, and it is the
specific way the predicates being replaced had failed before.

**Why the register's arithmetic was wrong and how that was found.** Running the trimmed script
end to end — which nothing had done during the compaction — reported that `decisions.md` and
`SPEC.md` each stated 174 ANSWERED, 3 BUILT NEVER RULED and 1 OPEN, while the register itself
held 176, 2 and 0, and the index still listed `Z1` as OPEN four days after it was ruled. The
number is counted, not typed, and the check for it exists; what did not happen is anybody running
it. A gate is only as good as the last time it was allowed to speak.

**V.178 — Why the surfaces needed a front door rather than a ninth surface.**

**The mechanism was complete and the discoverability was zero.** Nothing here adds a capability:
every one of the eight already loaded, already went through the ledger, already had its
behaviour tested. What did not exist was any way — from inside the program or from the command
line — to enumerate them. That is a real defect and not a documentation one, because *the list
itself* was the thing being maintained by hand in three places at once: the `Layout` accessors,
the readme's prose, and whatever a reader happened to remember. Three hand-maintained copies of
one list is the shape this repo has a rule about, and the fix is the same as it always is: one
table, and gates that fail when something disagrees with it.

**Why rows-in-force is the number that matters.** The failure mode a plugin system has that a
built-in does not is *the extension that silently does nothing*. Present, approved, valid TOML,
and read by nobody, because the array key is `backends` and the reader wants `backend`. Every
signal a user has says fine — the file is there, `shall lock` approved it, no warning appears —
and the symptom shows up later, somewhere else, as a line that names an unknown backend. A
standing of `no rows` is the only report that describes what is actually happening.

**Why `firewall:` proves the table has to be the definition.** It was the one surface with no
`Layout` method, because its reader joined the path inline. Nothing was broken by that; the
firewall adapters worked. What it cost is that any list derived from the accessors — which is
the obvious way to build one — would have been seven surfaces, and every gate over that list
would have passed. The table is written out by hand *and* checked against the source, in that
order, because the source is the thing that can quietly grow a ninth.

**Why one voice for the failures.** The eight `warn!("ignoring adapters/x.toml: {e}")` lines were
each individually reasonable and collectively useless: a serde error names a line number in a
file the user may not have known Shall reads, offers no example of a correct row, and does not
say that the rest of the file is inert. Worse, the eight were subtly different — one said
*"Ignoring malformed"*, one said *"ignoring the settings adapters in"* — so grepping your own
terminal for the word you half-remember finds seven of eight.

**Why a malformed adapter only warns, and where it is loud instead** *(ruled by the owner,
2026-08-10)*. Making it fatal is defensible: a file the user wrote and Shall cannot read is a
declaration that is not happening. It lost on where it would fire — a `sync` on a working
machine, where a typo in an optional extension file would stop you installing a package. The
degradation stays.

What the ruling adds is the surface where being loud is free. `check adapters` is a section of
its own and exits non-zero on any file that is written and not in use, so the fact lives
somewhere a person or a CI job can ask about it, rather than only in a warning that scrolls past
once per sync. A sibling was fixed with it: `check_approvals` carried a sentence claiming
adapters *"block a sync loudly"* and used that as the reason event hooks needed the section to
themselves. They do not block; they warn and skip, exactly like a hook. The comment had been
arguing for the section on a fact that was not true, and the true version of that fact is an
argument for one more section rather than for eight readers staying quiet.

**And what a skipped adapter actually costs, which is easy to get backwards.** Not that the
surface stops working — the built-in adapters still ship, so `ufw` and `firewalld` are still
driven. It is that a row *overriding* a built-in silently stops overriding it, and the machine
quietly returns to stock behaviour. An outage announces itself. This does not.

**V.179 — Why `pip:` had no answer on the distros most people run.** *(Rule in II.49. Ruled by
the owner 2026-08-10 as `Q49`.)*

PEP 668 is four years old and Shall had never met it, because nothing in this project had run a
real `pip install` on a current Ubuntu. The integration images had, once — the lifecycle ratchet
recorded 7 for ubuntu on 2026-07-30 — and then the images moved and the number fell to 6 and no
CI run happened again for eleven days. The gap was not discovered by reasoning about Python
packaging. It was discovered by a coverage number falling.

**Why the refusal is kept rather than routed around.** The marker means the distro's package
manager owns that site-packages, and it is not a formality: apt and pip both writing there is
the failure mode that ends with a python that cannot import its own stdlib, on a machine where
python is what `apt` is written in. A tool whose pitch is *be careful what gets installed on
your machine* does not get to be the one that overrides it by default.

**Why `pipx:` is the thing pointed at rather than a venv Shall manages.** pipx already does it,
Shall already drives pipx, and it lifecycles on every one of these images. Building a second
implementation of per-application environments — Shall owning a venv, its path, its cleanup —
to avoid naming a backend that ships in the same binary is the shape this repo keeps deleting.

**Why the flag is per line and splits the batch.** `--break-system-packages` is not a
preference, it is permission to write into something someone else owns, and permission that
leaks is not permission. The batch split is the same mechanism `@unverified` uses (V.104) with a
worse blast radius: sharing an opt-out weakens one check on one package, sharing this one
installs packages nobody said it about into the system interpreter.

**V.180 — Why a lock file outlives the process that took it, and what that costs.**
*(Rule in II.50. Ruled by the owner 2026-08-10 as `Q50`.)*

Kill a sync — Ctrl-C, a lost battery, a container torn down — and pacman dies holding
`/var/lib/pacman/db.lck`. The file is the lock: pacman creates it on start and deletes it on
exit, and there is no kernel involvement to clean up after it. So every Shall run afterwards
fails, with pacman's own advice about removing a file the user has never heard of, and the
machine's package manager is simply broken until somebody reads that sentence.

Shall was already *good* at this failure. The retry classifier notices the error does not change
between attempts and says so — *"this is not the transient failure its output looks like"* —
which is more than pacman manages. Being articulate about a wedged machine is not the same as
unwedging it, and `heal` exists for exactly the difference.

**Why apt and dpkg are excluded, and why the exclusion is data.** Their locks are `flock(2)` on
files that exist permanently. The presence of `/var/lib/dpkg/lock-frontend` says nothing at all
— it is there on every Debian ever booted — and the kernel releases the lock the moment the
holder dies, so there is nothing stale to clear. Deleting it deletes what the next `apt` expects
to lock. The distinction between *this file being here means a run is in progress* and *this
file is always here* is the whole safety argument, and a table that merely omitted apt would
re-admit it the first time someone extended the list by pattern-matching on the word "lock".

**Why staleness is proved rather than assumed.** The failure mode of getting this wrong is not a
failed command; it is a package database with two writers. So a pid file is judged by its own
pid, a lock with no pid is judged by whether that manager is running at all, and a pid file with
nothing readable in it — half-written by a process that died between `create` and `write` — is
evidence of nothing and is left alone. Not proved is not stale.

**V.181 — Why Shall waits for another package manager instead of failing at it.**
*(Rule in II.51. Ruled by the owner 2026-08-10 as `Q51`.)*

V.180 is about a lock nothing holds. This is about the far commoner case — a lock something
**does** hold — and Shall treated the two as one failure.

The retry loop gave a taken lock four attempts over about three and a half seconds, and then
printed the sentence V.180 praises: *"tried 4 times; the failure did not change, so a further
retry will not help — this is not the transient failure its output looks like"*. Against an
`apt upgrade` running in the next terminal, every clause of that is false. It **is** the
transient failure it looks like. A further retry is exactly what helps, once the holder is
finished. And the reason the failure did not change in three and a half seconds is that three
and a half seconds is not how long a package manager takes.

**The words cannot tell you which case it is; the machine can.** `pacman` says *"unable to lock
database"* whether its lock is held by a live transaction or by a corpse, and it says so
precisely because it does not know — its lock file carries no pid, which is why its own advice
begins *"if you're sure a package manager is not already running"*. `/proc` is sure. So the
verdict is taken from the machine and only the *trigger* from the message, which also keeps the
scan off every successful install: nothing is asked of `/proc` until a manager has already used
its own phrasing for a taken lock.

**Why waiting is bounded by the holder's work rather than by Shall's patience.** Five minutes is
not a guess about how long a user will tolerate a pause; it is a guess about how long a large
`dnf upgrade` takes. A bound sized the other way expires in the middle of the ordinary case and
produces the same failure with a delay in front of it, which is worse than either honest answer.

**Why the wait says who it is waiting for, immediately.** Shall's own data-directory lock has
announced its holder since S27, for the reason recorded there: a silent wait is indistinguishable
from a hang, and a hang gets killed. Killing Shall mid-sync is what leaves an orphaned manager
holding a lock — so a silent wait here would manufacture, on the next run, exactly the condition
it was waiting out.

**Why `heal` waits before it judges, and why the filesystem is asked before the process list.**
Both of these are the same mistake caught twice, a few hours apart.

`heal` surveyed once at the top, found a live `pacman` holding `db.lck`, and correctly left it —
then went on to a recovery during which that `pacman`, an orphan of the run `heal` had been
called about, exited. The lock became stale after the only step that could clear it had already
happened, and the run ended by advising the user to run the command they were running. A snapshot
is not a state; the holder has to be waited out before the question means anything.

And `held_for` asked *is a manager running* before *is there a lock at all*, which reads fine
until you notice that `ProcFs::any_named` answers **yes** on a machine with no `/proc`. That
answer is deliberate and right for the clearing path — "I cannot tell" must never become "go
ahead and delete it". For the *waiting* path it is exactly backwards: on Windows, where none of
these four files exists, every row reported as held, and `heal`'s new settle step would have
waited the full budget on each of them. Twenty minutes of a command doing nothing. A running
`pacman` with no `db.lck` holds nothing, on any platform, and that is the cheaper question
besides.

**Why backends that share a manager share a lock.** `pacman` and `yay` in one config is an
ordinary Arch machine: the repositories from one, the AUR from the other, both writing
`/var/lib/pacman/`. Keyed by their own names, Shall ran them concurrently and let pacman's
`db.lck` arbitrate — which it does by failing whichever lost. That is Shall contending with
itself, and no amount of waiting is the right fix for a race it created. The family table is the
same one V.180 reads, because *which backends share a manager lock* and *which lock is left
behind when one is killed* are one fact, and a second copy is the copy that goes stale.

**V.182 — Why a process Shall starts is a process Shall owns.**
*(Rule in II.52. Ruled by the owner 2026-08-10 as `Q52`.)*

Asking where the orphaned `pacman` in V.181 came from turned up two failures pointing in
opposite directions, both from the same missing idea.

**Shall killed what it should have asked.** `kill_on_drop(true)` and `start_kill()` are SIGKILL.
It cannot be caught, so nothing runs: no rollback, no unlink, no cleanup. A package manager
stopped that way leaves dpkg's database mid-write and pacman's `db.lck` on disk — Shall was
*manufacturing* the wedged machine of V.180, on its own idle timeout, and then diagnosing it
beautifully on the next run. SIGTERM is caught, and every one of these managers handles it.

And the child is usually not the manager. Shall runs `sudo pacman …`, so SIGKILL kills `sudo`
and the manager it launched survives as an orphan owned by init — still writing, still holding
the lock, with nothing left that could wait for it or report it. The orphan V.181 was taught to
wait for was, in the container, Shall's own doing.

**Shall abandoned what it should have owned.** Dropping a future that awaits `Command::output()`
does not kill the process; tokio detaches it and returns. Seventeen sites did that. The sharpest
is `backends/link.rs`, where a `tokio::time::timeout` around a secret decrypt carried the comment
*"rather than leaving the process (and this sync) hung forever"* — and freed the sync while
leaving `gpg` running against a prompt for as long as the machine stayed up. The comment
described the intent exactly and the code did half of it.

**Why a gate rather than a sweep.** The hand search for these found seven. The gate found
seventeen on its first run, including a `git` invocation, a `zfs list`, a `--help` probe, and the
`generate:` commands that run on every sync — none of which the search had reached, because it
had been capped at twenty results by whoever ran it. A list of sites fixed is a fact about one
afternoon; a predicate that fails the build is a fact about every afternoon after it.

**Why three doors and not one.** The hazards are genuinely different and one door would lie about
two of them. A `tokio` child's problem is detachment: it must be stopped when abandoned. A child
that owns the terminal must *not* be bounded, because a person typing into an editor is not a
silence to time out. A `std::process::Command` cannot be abandoned at all — its problem is the
opposite, that it holds a runtime worker until the child exits, which is why `git commit` after
every sync was parking a thread. Collapsing those into one mechanism is the same mistake as
collapsing V.181's three lock states into one failure.

---

**V.183 — Why a version is recorded everywhere and replayed only where it can be.**
*(Rule in II.53. Ruled by the owner 2026-08-10 as `Q53`; the bug is `S85`.)*

**The failure, measured on the macOS nightly leg.** A sync installs `brew:tokei`. `lock` records
`14.0.0` — correctly; that is tokei's version. A later sync reads it back as `@version=14.0.0`,
`brew.rs` builds `tokei@14.0.0`, and brew answers *No available formula with the name
"tokei@14.0.0"*. The sync dies, and it dies for ever, on a pin the user never typed. Homebrew's
`name@version` is a **different formula's name** — versioned formulae exist for a handful of
packages and carry a series (`python@3.12`, `openssl@3`), never a full semver.

On fourteen other backends the same declaration was dropped and the install reported success at
whatever version the manager picked. Ten could not do otherwise. Four could and had simply never
been built. That second group is the lie class: a command that did not do what it was asked and
said nothing.

**The chiluk that makes it tractable is that a lockfile has two jobs, and they are not the same
job.** Reproduce needs the manager to *accept* a version. Detect drift needs it only to *report*
one. Job two works everywhere. Conflating them is precisely what killed the run: a record kept
for job two was fed back as an install argument for job one. Once they are separated, nothing has
to be given up — the recording stays universal, and only the replay is narrowed to where replay
is possible.

**But separating them makes a report necessary that did not exist.** Before this, the only thing
that ever compared a recorded version against an installed one was the *planner*, by way of the
version it had injected into the spec — so the moment injection stopped, version drift on every
cannot-pin manager would have become invisible, and the ruling's own claim that "detect drift
keeps working" would have been false in the same commit that wrote it down. The comparison moved
to `check`, where it belongs: it is a question about two records, not about what a sync would do,
and asking it there is what makes it answerable for Homebrew and pacman at all.

**Why the refusal needs no provenance flag.** The obvious design carries a bit on each spec
saying whether a person typed the pin or `lock` recorded it, and a bit like that is one more
thing to set wrong. It is unnecessary: once `apply_locks` stops injecting a recorded version into
a spec whose backend cannot replay one, **a version reaching the planner on such a backend can
only have come from a line somebody wrote.** The distinction falls out of the mechanism instead
of being carried alongside it.

**Why `pins_version` defaults to false.** A new backend that answers nothing refuses a pin it
might have been able to honour — a message, and a wrong one, but a *visible* one. The other
default installs the wrong version and reports success. When a default has to be wrong in one
direction, it goes in the direction somebody notices.

**Why the ledger sits in the program and not in the test.** The refusal quotes it. A reason table
kept beside the scan would be a list that agrees with the messages until the day it does not, and
"cannot be met" with no *why* is a puzzle rather than a message (V.42).

**What the fix turned up, which is the part worth remembering.** The gate that claimed to cover
"every backend pins a version or says why" scanned `registry.rs`'s registrars and
`builtin_backends.toml`'s rows — and `brew` is neither. The one backend that did not merely drop
a pin but *invented* one was structurally invisible to the instrument built to find exactly that,
and eleven more hand-written backends sat in the same blind spot. It is the same shape as `S83`,
where a registry walk audited whichever backends happened to register on the host: an instrument
that enumerates one representation of a thing reports cleanly on every representation it cannot
see.

---

**V.184 — Why nothing outside Shall may ask a question Shall cannot answer.**
*(Rule in II.54. The bug is `S88`.)*

**A closed stdin is not a closed mouth.** Every wrong theory about this bug came from one false
premise: that a child with `Stdio::null()` cannot ask for anything. `sudo` opens `/dev/tty`
directly and reads the password there, and so does git's credential prompt. A process with a
controlling terminal — a CI job under a pty, a session nobody is sitting at — waits at that
prompt indefinitely, and because Shall captures the child's streams, **the question is never
displayed**. There is no output, no error and no end: exactly the signature of a slow package
manager.

Two hard failures, both in the `tools` leg, both nightly, both for at least six nights: *"a wrong
password left Shall waiting 900s instead of reporting a failure"* and *"a terminal with nobody at
it wedged Shall for 900s"*. 900 is the harness's own timeout, so both were hangs without end, and
they were most of why that job took 48 minutes. It sat unread for the same reason `S84` did: a
red nightly job nobody opens.

**Why priming once rather than bounding the prompt.** The bound is the obvious fix and it is the
weaker one: it makes the hang cheaper without making it impossible, and every escalated command
keeps its own chance to sit at a prompt. Priming the credential once and running every command
with `-n` moves the asking to a single place that can be bounded properly, and leaves every other
invocation structurally unable to wait — `sudo -n` can refuse, and cannot block. The keepalive
already worked this way and already carried the reasoning; the foreground path had simply never
been brought in line with it.

**Why ssh is left alone.** `-o BatchMode=yes` would close the last hole, and the only way to set
it from here is `GIT_SSH_COMMAND`, which overrides a user's `core.sshCommand`. Silencing a prompt
on a misconfigured remote is not worth breaking every working custom transport. A passphrase with
no agent is the user's setup to fix; an unprompted credential is ours.

**Why this belongs to `Q52`'s family from the other end.** `Q52` was about children Shall
abandons. This is a child Shall waits on for ever — and it was the last place where a program
outside Shall could stop this one indefinitely with no message.

---

**V.185 — Why a download is bounded before it fills the disk.**
*(Rule in II.55.)*

`web:`, `appimage:` and `github:` each read a whole response body into memory with `.bytes()` and
then wrote it out. Two problems in one line: the memory is unbounded regardless of what the file
turns out to be, and there was no ceiling on the file either. A redirect to something enormous —
or a server that simply never stops sending — is answered by allocating until something dies.

Streaming fixes the larger half by itself, and the ceiling is what makes the refusal legible.
`Content-Length` is checked first and trusted for nothing: when a server declares a size over the
ceiling the transfer is refused before a byte moves, which turns a two-gigabyte wait into an
immediate message, and when it declares nothing — a chunked response declares nothing — the
running count catches it anyway. One of those is a courtesy; the other is the actual bound.

**Why a ceiling and not a refusal.** AppImages are legitimately large, so a number here is a
guess about somebody else's artifact. It is set generously, it is in the config, and `0` removes
it. The failure it exists to stop is not a large download; it is an unbounded one.

**Why the partial file is deleted.** A half-downloaded artifact left on disk is one a later run
can find and treat as complete — and the checksum that would have caught that is exactly the one
`@unverified` is allowed to switch off.

**V.186 — Why the manifest owns what the registry forgot, and why a removal that removed nothing says so.**

*(Rule in II.56. Ruling `Q54`. Bug `S87`.)*

A cleanup uninstall reported success over a package it did not remove, on the `void` leg, after a
deliberate SIGKILL. It did not reproduce, and the register's account of it was wrong in a way
that made it look like a race: it read `SIGKILL left 2 newly-opened operation(s)` as "one
finished normally", when that number is a delta, and it concluded that the packages `heal`
recovered were the ones that would not uninstall. It is the other way round. The packages `heal`
recovers come back **owned**, because recovery records ownership; the ones nobody owns are the
ones `heal` never looks at, because their entries were already closed.

Underneath there is no race at all, only a record that is not durable. The registry is one
serialisation at the end of a run, and only if the whole transaction succeeded. Kill the process
between an install landing and that final write and the package is on the machine with nothing
claiming it — permanently, because every mechanism that could fix it is looking somewhere else.
The sync after the crash converges (the package is installed). The preview after that plans
nothing (there is nothing to plan). `heal` has nothing to replay (nothing is open). The damage
appears only when somebody tries to remove the package, which is the one moment ownership is
consulted, and the answer then is a command that reports success and takes nothing away.

**Why the harness could not produce it.** Both crash iterations poll the filesystem — one kills
as the log opens an entry, the other once a canary reaches disk. A canary reaches disk well
before its entry closes, so both always killed too early: twelve iterations, zero occurrences.
Polling the *log* for a newly closed operation hit it on the first attempt. That is why there is
now a third iteration, and why it is measured by what the kill *closed* rather than what it left
open — the state it aims at is one where nothing is open at all.

**Why recovery, and not a durable write per install.** Writing the registry as each package
lands would close the window at the cost of one serialisation of the whole file per package, on
the hot path of every sync, for a failure that needs a kill inside a window of microseconds. The
registry is a materialised view, and recomputing a view is cheaper than making it durable. It
costs nothing when there is nothing to repair, and it also repairs the non-crash sibling — a
transaction that fails after some installs succeeded never reaches the final write either.

**Why the manifest is what the view is recomputed from, and not the log** (owner ruling,
2026-08-11). The first version replayed the write-ahead log: it recorded every install per
operation, so it held what the registry had lost. It worked, and it was too narrow in one
direction and expiring in another.

Too narrow, because the log answers *did Shall install this* and the question ownership actually
turns on is *does this machine declare it*. A package installed by hand and declared afterwards
is never registered at all — an already-present package schedules no install, so no `state.add`
ever runs for it — and that is the common case, not the crash. Expiring, because finished log
entries are purged after seven days, so a machine left alone for a week kept its orphan for ever
with no mechanism left that could see it. The ruling is that declaring a package you already
have makes it Shall's, and once that is the rule the manifest is simply the better record: it is
the one the user wrote, it does not expire, and it is already resolved on every sync.

The declaration is also written *before* the install, by the same `install` that P1 defines as a
file edit. So it survives every kill the registry does not, and the crash orphan the log was
introduced for is covered by the manifest as a special case rather than needing its own
mechanism. Nothing is left that only the log can see: `requires` wires edges inside the declared
set rather than pulling in undeclared packages, and the dependent phase installs shims, services
and links — not packages. `completed_installs` was deleted rather than kept as a second source,
because two records of one relationship is how this repo got into trouble.

**What this gives up, stated plainly.** Undeclare a package by hand and then try to uninstall it
and Shall cannot tell it was ever managed — there is no record left saying so. Ruled acceptable:
`uninstall --absent` removes it regardless of ownership, which is the same end state through a
flag the user types.

**And what it takes on.** Declaring a package you already had now means Shall removes it when
that declaration goes. That is a real transfer of blast radius, which is why the repair
**announces** what it claimed instead of doing it quietly — a machine that silently adopts
software the user installed by hand is deciding something on their behalf. The boundary is that
declaration is the whole of it: an undeclared package on the machine is never claimed, however
it got there, because an installed set is not a manifest.

**Why `unmanage` no longer has to be told.** It writes the registry and the manifest, and
ownership is now read from the manifest — so dropping the line *is* the forgetting, and there is
no third record left to keep in step. The old repair read the log, which still said Shall had
installed the package, so `unmanage` had to clear those entries to stop the next sync taking it
back and then removing it as undeclared drift. That clearing was deleted with the reader it
defended against; leaving it would have been a defence against nothing, and it cost the evidence
of past work for no reader's benefit.

**Why `uninstall` fails rather than warns.** The failure this fixes is precisely that a script
could not see it. `shall uninstall x && rm -rf ~/.config/x` proceeded on a package that was
still installed. A warning on stderr under exit 0 is the same bug with more text.

**Why `--absent` is a flag, and why it writes a declaration rather than removing directly.**
The failure above is honest but it is not always what the user wants: a package Shall does not
own is one `adopt` away from removable, and requiring the two steps to remove software you can
see on your own machine is ceremony. What made this worth a flag rather than a default is that
the two cases are not the same size. Reporting honestly costs nothing if the reading is wrong.
Removing by default would make `uninstall` a verb that takes away software Shall never
installed, and a flag is where a decision about blast radius belongs — it is typed, per run, by
someone who meant it.

It writes an `absent:` line because that line already exists and already means this: *remove
this whether or not you manage it, because I named it* (V.7). A direct removal here would be a
second removal path — one that skipped the guard, the plan and the counts that every other
removal goes through, and that had to grow its own preview to say what it would do. The line
also outlasts the run, which is the point rather than a side effect: ownership is the record an
unowned removal has no equivalent of, and a declaration is a record. A package removed by
`--absent` stays removed when a module elsewhere asks for it back, because II.7 rule 6 says the
absent line wins — and that is why `--absent` prints no "still declared in an inactive module"
warning, while a plain `uninstall` must.

**Why the survivor check stays on that path, with different words.** `--absent` claims to
remove every name it was given, so a package still installed afterwards is a removal that
failed, not one that was refused — the guard declining a protected package reaches exit 0 with
nothing removed, which is `S87` again in a new command. The advice has to differ from the plain
path's: telling someone to `adopt` a package that just survived an explicit `absent:` line
answers a question they did not ask.

## `H1` — the two commands that did not agree about one machine

**The bug.** On a stock Ubuntu container with `sudo` installed, three declarations, and nothing
unusual done to the environment:

```
$ sudo -n env SHALL_CONFIG_DIR=… shall sync --yes ; echo $?
 WARN `ripgrep` is declared for `cargo`, which is not on this machine — skipping it.
 WARN `left-pad` is declared for `bun`, which is not on this machine — skipping it.
 WARN `cowsay` is declared for `uv`, which is not on this machine — skipping it.
0
$ sudo -n env SHALL_CONFIG_DIR=… shall check ; echo $?
->  drift  3 package(s) ...
2
```

Three packages asked for, zero installed, no transaction summary, and the exit code named
`Exit::Converged`. Alternating the two commands repeated it exactly: 0, 2, 0, 2.

**The trigger is not exotic.** `sudo` ships `secure_path` set to
`/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/snap/bin`, and `cargo`, `bun` and
`uv` install to `~/.cargo/bin`, `~/.bun/bin` and `~/.local/bin`. Every one of them is invisible to
anything run through `sudo`, sitting exactly where its own installer put it. `shall schedule` and
`shall fleet` exist, so an unattended sync is a supported use — and an unattended run is precisely
where `PATH` is not the user's. The three warnings are useful to a person watching. Nothing is
watching. What the pipeline reads is the 0.

**Why the rule is what it is.** The distinction was already ruled one command over: §Q2 defines
**critical** as *"it is installed, or `priority` names it, and it cannot work"* — a
`priority`-named manager that cannot be reached is a broken machine, not a line that does not
apply here. `check` was told. `sync` was not, and the two therefore described one machine in
opposite terms at one moment.

**Exit 1 rather than 2**, from `U21`'s own table: 2 is *a read-only command looked and found work
to do*, and `sync` is not read-only; 1 is *Shall could not carry the command out*, which is what
happened. It is already the code a failed install returns, and a declaration that never reached
its manager is the same fact about the run as one the manager refused.

**The trap this had to avoid, and the reason `SkipKind` exists.** `SyncChanges::skipped` carried
two opposite kinds of row in one list: a *declined removal* (installed, undeclared, and it stays)
and a *skipped install* (declared, not installed, and it does not arrive). Failing a run over the
first would fail every adopted machine on earth, because a guard declining a protected package is
the ordinary case. The two kinds are now distinct in the type, so this rule can be stated about
one of them instead of inferred from a sentence — which is the same move `Declined::reported`
already made for removals and which the install path never went through.

**The partial case is the dangerous one.** Three of three is the reproduction; three of four is
ordinary and worse, because something did install and the summary reads like a successful
transaction. Hence a per-declaration count rather than a whole-run boolean.

## `H2` — the preview a script reaches for, always saying "converged"

**The bug.** `target-state.md` says exit 2 *"means a read-only command found work to do"*, and
only `check` ever built `Error::Differences`. Measured: `shall plan` printed *"1 install(s), 0
removal(s)"* and exited **0**.

**Why it matters more for `plan` than for anything else.** `plan` is the command that writes the
machine-readable artifact — it exists to be consumed by a script. A pipeline that wants to know
"is there work to do" will reach for the command that hands it a document, and that command told
it "no" every single time, including while printing the work on the line above.

**Why `list --outdated` is deliberately excluded.** A listing's subject is inventory, not a
verdict. Every script that has ever piped a listing expects 0 for a non-empty list, and a
`list` that exited non-zero for having contents would break them for a distinction they did not
ask about. The two commands are not the same kind of thing, and `U21`'s table is about commands
that render a verdict.

**The condition is `check`'s condition, deliberately.** Not "roughly the same" — the same
quantities in the same combination. The single most expensive recurring defect in this tree is
two readings of one machine disagreeing, and a `plan` whose threshold drifted from `check`'s
would be that defect with a new pair of names.

## `H4` — the confinement that was not one, and the knob that was not wired

**The bug, in a container with no `bwrap` and stock configuration:**

```
$ shall run -p apt:bash@sandbox -- sh -c 'id -u; touch /srv/escaped'
IN-SANDBOX-UID=0
$ ls /srv/escaped
/srv/escaped          # on the host filesystem. rc=0. Nothing on stdout or stderr said so.
```

**Three mechanisms in a row, each of which alone would have been enough.**

1. `SandboxSettings::fallback_allowed` defaults to `true`.
2. `Sandbox::is_available` was `bwrap_available() || settings.fallback_allowed`. It did not
   answer *"is a sandbox available"* — it answered *"is a sandbox available, **or** are we
   permitted to skip it"*, which under the default is a constant `true`. Every caller read it as
   the first question.
3. `wrap_linux` then found no `bwrap`, logged *"Falling back to PATH isolation"* at `debug!`, and
   returned `Command::new(cmd).args(args)` — a bare command with an unmodified environment.
   **There was no PATH isolation.** The sentence named a boundary the code did not build, which
   is word for word the bug `hooks.rs` records having already caught once: *"It was called
   `setup_lua_sandbox`, which claimed a boundary this does not build."*

**The one user-visible warning was unreachable.** `run.rs` read `else if settings.fallback_allowed
{ warn!("Sandbox requested but unavailable…") }`. Reaching it needed `can_sandbox == false &&
fallback_allowed == true`, and step 2 makes that combination impossible. The `warn!` was dead for
every input; the live path logged at `debug!`.

**And the documented remedy did nothing.** `require_bwrap` was declared, documented — *"On Linux,
if true, Shall will fail if 'bwrap' is missing"* — defaulted and serialised, and **read by
nothing**. Its Windows twin `windows_require_sandbox` was wired and raised exactly the refusal its
doc promised. So an administrator who read the configuration reference, decided silent unconfined
execution was unacceptable on their fleet, and wrote `require_bwrap = true` got byte-for-byte the
same unconfined run. A scan of all 76 `pub` fields in the configuration schema found **exactly
one** dead setting, and it was this one — no false positives, so the instrument was naming a
singular case rather than a pattern of noise.

**Why the default stays `true`, which is the part that will look wrong at a glance.** The obvious
fix is to flip it, so an explicit `@sandbox` refuses on a host that cannot honour it. That was the
recommendation put to the owner and it was ruled against, on a principle worth writing down: a
feature in this codebase is **built fully, and not deferred or withdrawn because it is hard or
potentially insecure** — within reason, people are smart. Refusing to run is Shall deciding, on a
user's behalf, that they may not proceed on their own machine. What the user is owed is **the
fact**, and what was wrong here was never the permission — it was that the permission could be
exercised in silence.

So the escape hatch stays open and stays the default, and all three mechanisms above are closed:
one decision point (`Sandbox::decide`), a verdict carried as a value (`Confinement`) so no caller
can claim a boundary it was not handed, a `warn!` on every unconfined run that reaches the person
before the command does, the false "PATH isolation" sentence deleted, and `require_bwrap` wired
and outranking `fallback_allowed` — which is what makes the ruling safe: the knob whose entire
purpose is to close the hole is now a knob.

**Windows keeps its low-integrity launch, reported as `Confinement::None`.** It does lower the
token, so deleting it would remove a real reduction; it is not a sandbox, so calling it one would
be the exact claim this type exists to prevent.

## `H6` — the verb that upgraded everything except the things that are not packages

`shall upgrade` moved managed packages and nothing else. That is defensible for a *package*
manager right up until you notice what a machine actually needs brought forward: firmware, an
editor's plugin manager, a tracked git repository, `rustup` and `gcloud` components. None of
those is a package, none of them is expressible as one, and `topgrade` — the tool a user leaves
to come here — does all of them.

**The mechanism was already built. Only the verb was missing.** `exec:PATH @runs=always` is a
declared line that runs a command on every sync: approval-gated by `shall lock` so nothing runs
that a human has not read, journalled write-ahead like every other mutation, and undone by
`@undo=` when the line goes. Three grading documents recorded this gap as a subsystem to build.
Measured on 2026-08-13, it was one fact: `src/verbs/upgrade.rs` never touched extras, so a
firmware step correctly written and correctly approved was run by `sync` and never by `upgrade`.

**Why the fix is an option and not a widening.** The obvious cure is to run every `exec:` from
`upgrade` too, and it is wrong for a reason the approval ledger cannot fix. `locks/hooks.toml`
records *what content is allowed to run here*; it has never recorded *which verb may run it*. So
a blanket widening takes every `exec:` line in every manifest that already exists — written,
reviewed and approved by people who were consenting to `sync` running them — and hands them to a
verb that has never executed a user script in its life. The approval on file would still be
valid. It would just be answering a question nobody had asked yet.

`@on=` therefore names the verb per step: `sync` (the default), `upgrade`, or `both`. A manifest
that says nothing means exactly what it meant yesterday, and a step that wants the new behaviour
says so on the line where a reader will see it next to the script's own name.

**Three values rather than a list, and that is `F3` speaking.** Options are separated by commas,
so `@on=sync,upgrade` parses as `on=sync` plus a second option called `upgrade` — the same
boundary confusion `F3` was, invited in through the value grammar instead of the key grammar.
`both` costs one word and removes the ambiguity entirely.

**And the unknown value reads as the default, not as "everywhere".** The grammar refuses anything
outside the three by name, so `Verb::claims` cannot be reached with a fourth — but if it ever is,
an unrecognised word must not be what widens which verb runs a script.
