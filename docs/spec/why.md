# Part V — Why

*[LiNix v7](../SPEC.md) — the map is there; this is one part of it.*

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
LiNix decides things. `include:` strictly superseded it.

**V.5 — Why conflicts are errors.** Files were read in filesystem order and first
declaration won. `a.txt: jq@1.6` vs `b.txt: jq@1.7` was decided by the disk. Sorting the
read order only makes the wrong answer deterministic.

**V.6 — Why `keep.txt` died.** It lived in the groups folder and ended in `.txt`, so the
resolver ate it: *"never remove firefox"* also silently meant *"install firefox"*. It was
held back by a hardcoded one-element denylist. **Separate by location, not by denylist** —
and `forget` gives people the thing they actually wanted, which was a way to make LiNix let
go.

**V.7 — Why `absent:` is the one exception to "only removes what it manages".** Because you
named it. Everything else LiNix touches, it owns. `absent:` is you reaching outside that,
deliberately, by name. It stays a line rather than a file because a file can't be turned off
per profile, can't be shared, and puts LiNix's bookkeeping back in a folder you author.

**V.7b — Why a name no line can hold is protected, and why the escape hatch does not open
it.** `winget list` answers for Add/Remove-Programs entries with pseudo-IDs like
`ARP\Machine\X64\Android Studio`. A package name is one word (II.2), so no module line can
hold that: `adopt` cannot take it, nothing can declare it, and it is therefore **unmanaged
forever** — which made it a standing `purge-unmanaged` candidate that `linix adopt` could
never clear. The documented safe sequence, adopt-then-purge, proposed deleting Android Studio.

Removing what you could never have been asked to keep is the inverse of "LiNix only removes
what it manages" (V.34), so it is a protection rather than a warning. It is checked **before**
`unprotected_packages`, which is otherwise absolute (V.35): that hatch means *"I manage this
one myself"*, and you cannot manage what you cannot write down — there is nothing for it to
release. Asked through the one grammar, not a second copy of the naming rule.

*Found by the live Windows sweep, where `adopt` wrote those IDs into `modules/adopted.txt` and
every later command — `rollback` included — died parsing the file LiNix had just generated.*

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
named `jq`, and failed at install — and had that crate shipped a binary, LiNix would have
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
so LiNix keeps it forever; remove vim and it stays, because apt says "orphan" and your file
says "I want this" and the file wins. Monday's bug was claiming ownership of a set that was
never LiNix's.

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

**V.16 — Why unpinned names get locked, per machine.** LiNix *probes* — "does apt have
ripgrep?" So `ripgrep` lands on cargo today, Ubuntu adds it tomorrow, and the same unchanged
line resolves to apt: LiNix uninstalls from cargo and installs from apt because a repo you
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
would make LiNix guess which backend, and the honest ways to guess are all wrong: a
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

**V.21 — Why `purge-unmanaged` is a command and not a mode.** **Sync is then never
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
*purely* to stop drift-pruning deleting `linix install`-ed packages, because they lived in
`local.txt`, which `-g` could move out from under the registry. **Someone met Monday's bug,
understood the symptom exactly, and patched it with a flag.** Not one behaviour was deleted;
they moved to where they were always trying to be.

**V.26 — Why protection is a refusal, not a declaration.** Everything else is a statement of
intent ("I want this"). Protection is "I will not do that, and there is no flag." It doesn't
care whether the package is managed, declared, adopted, or predates LiNix. That's why it
lives in preferences and not in a module — and why deleting a declared `apt:python3` line
makes LiNix refuse until you unprotect it.

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

**V.30 — Why git is the history.** **LiNix commits only on a successful sync, so every
commit is a state your machine actually reached** — not one you asked for. `git log` is
where your machine has been; `git diff` and `linix plan` are the same question; rollback can
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

**V.32b — Why the check reads git's verdict and does not compute one.** LiNix runs `git log
--pretty=%G?` and carries the letter it gets back. It does not decide what a key is worth,
because that is the twenty-year-old tool's job and re-deciding it is how the previous signing
scheme ended up with `sha256(key + "|" + text)`. The same reasoning splits `Good` from
`Unverified`: git distinguishes a signature it trusts from one made by an untrusted, expired or
revoked key, and folding the second into the first would restore exactly the appearance-without-
protection V.32 is about. **And why the refusal is off by default:** a rule that fires on every
rollback in a repo nobody signs is a rule that gets turned off, at which point the signed case
is unprotected too.

**V.33 — Why `clone` died.** It copied **the installed set, not the intent** — you got a
machine with the same packages and no idea why. `git clone && linix sync` gives the intent,
the history, the pins, and the ability to change it afterwards.

**V.34 — Why `prune` and `orphans` died.** sync removes drift by definition, so `prune` is
sync with the install half amputated. "Prune" meant four unrelated things; deleting the
command leaves exactly one meaning ("delete old history") for the first time. `orphans`
shows what sync would remove, which is `plan` — and its message named two commands and
described neither.

**V.35 — Why `--backend` is refused on removals.** A scoped removal is Monday's exact shape:
**you narrow what LiNix looks at without narrowing what it owns**, so everything outside the
scope looks like drift.

**V.36 — Why `clean` survives.** It's apt's housekeeping, not LiNix's drift, and only apt
knows about it. It goes through the guard because `autoremove` is a mass removal LiNix
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
`modules/hooks.txt` and see exactly what got in behind LiNix's back. One `local.txt` mixes
them and forgets which was which.

**V.41 — Why "detected, not configured".** LiNix should not be *told* you have btrfs; it
should look. Not told you have four cores. Almost every "local fact" in `config.toml` is
something LiNix could work out in a second and instead asks you to maintain by hand, forever,
on every machine. **That is not configuration, it's homework.**

**The `max_parallel` exception (owner ruling, 2026-07-17).** This rule's first draft called
`max_parallel` homework too — and noted it was overwritten at `sync/mod.rs:296` anyway, "so the
setting is already a lie." Both halves are now dead: the overwrite is gone (`sync/mod.rs:293-297`
reads it as *"the user's knob"* and honours `self.config.max_parallel.max(1)`), and the owner has
ruled to **keep** it. The distinction that saves the rule: the core count is a *fact* (detected),
but *how many of those cores to use* is a *preference* — you may want to cap it to keep the
machine responsive while a big sync runs. A preference LiNix cannot look up is not homework. So
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
character. The empty list is the one refusal: `linix activate $PROFILE` with `$PROFILE`
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
that LiNix never edits a block — a block is something you wrote — so `deactivate Travel` would
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
learned only that LiNix is fussy. **A warning carries the same information and does not stand
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
removal means "LiNix stops asserting this", and for a setting the honest meaning of that is "the
desktop's own default applies again", not "whatever this machine happened to hold before LiNix
first ran". Restoring a prior value would demand a per-machine store of pre-LiNix state — the
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
over two namespaces, and the day LiNix learns to detect `distro` or `init`, every file that
named a variable `distro` silently changes meaning. **A detected-fact namespace that can never
grow is a worse cost than one character.** With the sigil, facts can be added forever and no
user file is touched, and a reader can tell at a glance which half of the condition they
decided and which half the machine reported.

**V.53 — Why a provider is chosen by filename and ambiguity is refused.** *(Adopted 2026-07-20
from Part IX, IX.6; owner ruling.)* Two bugs, one entry. First, a **silent precedence guess**:
if `vars` and `vars.py` both sit in a repo and LiNix picks one by directory order or a built-in
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
owner ruling the same day.)* II.6b handed `vars.linix` the shell, the filesystem, the
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
direct twin of a fixed one: the Windows script greps `linix` against `git log` where the
container greps `linix:`, and the config directory is named `linix-it-win-config`, so it matches
on every run forever. Another asserts that the build artifact is still on disk and calls it
*"linix survives an uninstall attempt."* One fixed, siblings live, in the sibling file — the
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
`linix --version`, which was answering `6.0.0` — a number describing the model this rewrite
exists to delete. Nothing has ever been released: the branch sat 219 commits ahead of the
remote, no tag was ever pushed, and the tag-triggered release job in CI has never fired. So the
number was not a version, it was a counter of internal rewrites, and it was **counting up while
the thing it named was being thrown away**. `0.1.0` is what it means to have shipped nothing
yet, and going down is the only honest direction from a number nobody was ever given. The
rewrite keeps its name — "v7" is what Part VII and the CHANGELOG call this work — because a
codename and a version answer different questions.

The install scripts are the same fault at the other end. Both fetched from
`github.com/OWNER/linix`, a placeholder that was never substituted, and both finished by running
`linix migrate` — a command **II.17 has listed as deleted since the rename to `adopt`**. So the
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
steps. Worst is `purge-unmanaged`, which prints *"Snapshot taken: X. That is your undo"* — the
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

That would be a latent race in most tools. Here it is a live one, because **LiNix installs
package-manager hooks**: `DPkg::Post-Invoke` and its dnf/zypper/apk/xbps/portage siblings spawn
`linix hook-reconcile` on every ordinary `apt install`. So the second writer is not another
LiNix the user ran — it is `apt`, run by someone who does not know LiNix is involved, at a moment
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

**V.63 — Why `sync` is additive and `purge-unmanaged` is exclusive, for every backend.**
*(Owner ruling 2026-07-23, N1; the rule was always true and had never been written.)*

The firewall proposal asked whether a declared perimeter is exclusive — whether a rule LiNix
never declared counts as drift. It is a reasonable question and it should not have been askable:
**the model answered it years of decisions ago, for every backend at once, and nobody had put
the sentence anywhere a reader could find it.**

The split is what makes LiNix safe to point at a machine that already has software on it. `sync`
only ever removes what the ledger says LiNix put there, which is why running it on an unadopted
box does not empty it. `purge-unmanaged` removes what LiNix did not declare, which is why it
carries a ratio guard, a full listing and a snapshot — it is the one command whose whole job is
acting on things LiNix does not own.

**The bug this prevents is a second `purge-unmanaged` per backend.** A backend that ships its own
exclusive mode has re-implemented that command with none of its protections: no ratio check
noticing you have not adopted the machine, no listing, no snapshot, and a different opt-in for
the user to learn. It would also make the answer to *"will this delete something I made by
hand?"* depend on which backend the line happened to name — which is the two-of-everything
failure at the level of a promise rather than a function.

So: **a backend does not decide its own exclusivity.** If a new backend seems to need an
exclusive mode, the thing it needs is `purge-unmanaged` to learn about its resources.

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
carries `@undo=`, that is what runs. If it does not, LiNix **drops the record and does nothing
else**, and `plan` says so in those words. The alternative — inventing an inverse for a script
whose author did not write one — is LiNix claiming to undo something it cannot, which is the
same class of lie as printing "rolled back" on the strength of an exit code (V.60).

**V.67 — Why a dotfiles tree links files and never directories.** *(U22–U25, ruled 2026-07-24.)*

A dotfiles tree is the one declaration whose *layout is the statement*: `dotfiles:./home` stands
for as many declarations as the folder holds, and adding a file to it is how you declare a new
one. The temptation is to symlink the directory — one link instead of two hundred, and new files
appear for free.

**The bug that closes it is that a symlinked directory is a directory the application writes
into.** Link `~/.config/nvim` and every plugin nvim installs, every lockfile it generates, every
piece of session state it caches, lands **inside the git-tracked repo**. The repo stops being a
declaration and becomes a mirror of runtime state; `linix diff` fills with noise; and `bundle` —
whose whole promise is that the archive is safe to hand to someone — hands over whatever the
application happened to leave there. Per-file links cost more link calls and are the only form
where what LiNix manages is what the user wrote down.

**A destination that already holds the user's own file is refused by name, not replaced** (U23).
The tree has no place to write a per-line option, so there is no `@force` for it and there
cannot be one — which is the same structural reason it **never decrypts** (U24): a `.age` file in
the tree is a file, and there is nowhere to say otherwise. A secret that needs decrypting is a
`link:` line, where the option can be written and read.

**V.68 — Why `firewall:` is built in, and why the lockout check comes first.** *(N1–N7, ruled
2026-07-23/24.)*

A firewall looks exactly like something the onboarder should cover: `ufw` is a command with
subcommands, and XIII.2 exists so a user can add a manager LiNix never heard of. **It is not,
and the reason is the one thing a `[[backend]]` naming `ufw` could never give:
`firewall:22/tcp` must mean the same thing on the Debian laptop and the Windows workstation.**
A per-machine adapter definition makes the *declaration* per-machine, and a declaration that
means something different on two machines is not a declaration. So the statement is built in and
the **adapters** are rows (K17) — `ufw`, `firewalld`, `windows-defender` shipped as data, a
fourth added without a release.

**The lockout check is this feature's precondition, not one of its features.** LiNix detects the
port carrying the controlling connection and refuses any plan that would deny it — from `sync`,
from `purge-unmanaged`, and from an unattended `watch` tick. The tick is the dangerous one:
nobody is there to read the refusal, and the machine that locks you out is the machine you can no
longer reach to fix it. **Building the backend before the check is building the lockout**, which
is why the check sits at the bottom of the module and everything above it is written against it.

**V.69 — Why `@scope` exists on exactly three statements, and why writing the default is
allowed.** *(U19, ruled 2026-07-24.)*

LiNix used to act, implicitly, as whoever typed the command. The Linux backends mostly agree
with that by accident. **The Windows registry cannot**: `HKCU` and `HKLM` are a real choice with
no default that is right for both, and picking one silently means a config that reads identically
on two machines configures the account on one and the machine on the other.

So the question is asked where it can vary — `setting:`, `link:`, `shim:` — and **nowhere else**.
A `service:` is the init system's business and a `repo:` is the manager's; putting `@scope` on
them would be a key that means nothing, and a key that means nothing is a key someone writes and
LiNix silently ignores, which II.2 closes with in exactly those words.

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

**V.72 — Why `linix lock` approves the whole `adapters/` folder, not a named list.** The
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
which one is the safety net must be the user's decision, stated, not LiNix's guess. Choosing "the
one that claims live restore" would let a newly-added, less-trusted provider silently displace a
proven one the moment it declared a capability; choosing by registration order would make the
answer depend on an implementation detail nobody wrote down. `snapshot_priority` is the
`priority`-file shape reused (V.15's reasoning): the first *available* provider in the declared
list wins, an empty list keeps the historical registration order, and a name that matches nothing
present falls back rather than leaving the machine with no net it could have had.

**V.76 — Why APFS is declared create-only.** macOS ships APFS on every machine and `tmutil
localsnapshot` takes one with no configuration, so the second platform LiNix supports finally has
a safety net — but an APFS *restore* needs a reboot into the recovery environment, which LiNix
cannot drive on a running system. Claiming `Live` would be V.60 exactly: an undo offered where it
cannot be kept. So APFS snapshots and refuses the rollback with the manual steps. And because
`tmutil` does not record which snapshots LiNix made, retention never reaps an APFS snapshot
(`is_linix_owned` is false for them) — the safe direction: LiNix never deletes a restore point it
cannot prove it created (S3).

**V.77 — Why a user verb may only compose built-in verbs.** A `[verbs]` entry is `defun` over
the command surface — `refresh = sync, then upgrade` — and it is safe precisely because it
sequences operations LiNix has already audited, producing nothing the guard, the plan and the
ledger did not already see. The moment a verb can run arbitrary argv it is `exec:` wearing a
command's clothes (U4's settled question), so a step that names anything but a built-in is refused
and pointed at U33's off-by-default key. A verb also takes no arguments of its own: threading
`linix refresh --dry-run` into some steps and not others is the surprise a closed vocabulary
exists to avoid, and a verb never shadows a built-in, so a shorthand can never mask a real
command. `linix repl` sits under the same principle from the read side (the U20 rule): it is a
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
The expansion is ordinary declarations, visible in `linix eval` and the removal preview before
anything runs — a macro that could produce an action you cannot see is the one thing U32 must not
be, which is why generated declarations are U33's separate, off-by-default question.

**V.79 — Why `generate:` is off by default, and how it stays on the safe side of the line.**
`generate:` (U33) runs a command and treats its stdout as declarations — the one surface where
the config *computes* its state instead of stating it, which is the property XIII.32 says openness
must not cross. The owner ruled it in anyway, so the whole weight falls on four rules, none
waived: (1) **off by default** — `allow_generators` unset makes a `generate:` line a refusal
naming the key, so the computing-config surface is dormant unless deliberately turned on; (2) **the
ledger gates it** — it is approved by `linix lock` content-addressed like `exec:`, and an
unapproved or changed command stops resolution, `-y` cannot approve; (3) **a failure is a failed
resolution, never an empty set** — a non-zero exit is an error, because "the generator broke" read
as "nothing is declared" is a mass-removal input, VI.0's whole family; (4) **the output is shown,
not trusted** — it is spliced into the statement stream *before* bare-name probing and collection,
so a generated line passes the same conflict check, guard and removal preview as a typed one, and
a generated `apt:foo` reconciles with a typed one rather than doubling it. The approval is scanned
from the files, and scanned *first* in `linix lock`, because resolving the model now runs
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
not say `stdout_only = true` does not load, because LiNix will not hand a secret to a command
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
`plugin.yaml`. LiNix declared the URL, because that is the string install needed, and the install
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

**V.84 — Why LiNix reads every command's output, and why a child never gets the screen (U40,
S42/S43).** *(Found by the production-readiness review, 2026-07-27; ruled and built the same
day.)* `RawExecutor::execute` asked one question — is *LiNix's* stdin a terminal? — and used the
answer to decide all three of the child's handles. When it was, the child inherited stdout, so
`output.stdout` came back empty and all 79 `run_output` call sites parsed an empty string. `linix
list -b apt` reported **609 packages piped and 1 under a terminal, on the same machine, from the
same command.** The failure is worse than a wrong answer because it does not look like one: what
reaches the screen is `dpkg-query`'s own output, which reads like a package list to anyone who
is not comparing formats.

**The rule is therefore about *who* decides, not about which way it goes.** Capture belongs to
the call — a read parses, so a read captures; a mutation may need a password, so a mutation may
share stdin and nothing else. Making it ambient made LiNix behave one way for the machines that
test it and another way for the machines that run it, and only the first kind reports back.

**The same inheritance turned a read-only command into a hang.** With stdout inherited,
`systemctl` concluded a human was watching and piped itself into a pager; `linix status` waited
for a keypress and had to be killed, and across three identical runs printed 80, 640 and 83
lines. So the pager suppression is not a second fix for the same bug — capturing removes the
usual trigger, but `$PAGER` and `$SYSTEMD_PAGER` force one regardless, and a forced pager puts
`lines 1-16/16 (END)` and a screenful of escapes into the text a parser is about to read. It is
set on the env map every spawn inherits, because a suppression applied at some call sites is the
`command -v` case again: the sibling that was missed is the one that runs.

**Mirroring exists so the fix does not cost what it fixes.** Inheriting the handles was the wrong
mechanism for a real requirement — a five-minute `apt install` that prints nothing is a tool that
looks wedged. The bytes now go both places, and the mirror is stderr because stdout is where
LiNix's own answer goes and interleaving the two makes both unreadable to whoever piped us.

**What this cost, and why the test matters more than the fix.** 1,324 tests, four container
lifecycles and three OS builds were green throughout, and not one of them could have observed
any of it: every gate in the repo runs with pipes on every handle. A green suite was not evidence
against the finding — it was the reason the finding survived. `tests/pty_tests.rs` closes the
gap with `script -qec` and a stub manager on `PATH`, asserting that what LiNix printed is what
LiNix parsed, and it was watched failing against the old behaviour before it was made to pass.

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
was assembled by reading, and `linix --help` would have taken ten seconds.

**So the first rule here is about rulings, not commands: a decision to remove a feature is
checked against the running program before it is made.** A consolidation argued from a wrong
inventory removes real capabilities to fix an overlap that was never there.

The removal verbs are not synonyms, and the proof is that no two of them can be swapped:
`uninstall` takes a package away; `remove-orphans` takes away what the *manager* considers
orphaned; `purge-unmanaged` takes away everything LiNix does not manage; `unmanage` takes away
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
on every ordinary run was LiNix narrating its own startup — `No state file found at …`, printed
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
and the line stayed forever even though it could never succeed. A bare `linix install typo` was
withdrawn correctly the whole time, which is why it went unnoticed.

The missing fact was already computed. Every failure carries a `Retryability`, filled in by the
backend's own `ExitPolicy`, and scoop's policy already marked this exact failure `Permanent`.
LiNix classified it as impossible and then kept it anyway.

**Three limits on the widened rule, and each is load-bearing:**

1. **Permanence is read off `CommandFailed`, not off `Error::retryability()`.** That method also
   returns `Permanent` for `Refused`, `Cancelled`, `Config`, `Validation`, `Permission` and five
   more. Every one of those is permanent in the retry sense and none of them means the name was
   wrong. Withdrawing on `retryability()` would delete the line a user just asked for because
   they answered "no" to a prompt — a worse bug than the wedge, and a silent one.
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

**V.91 — Why "not installed" is not "critical".** *(Owner ruling, 2026-07-27 — Q2.)* `check
health` opened with `Backends: 25 OK, 0 degraded, 23 critical (of 48 total)` on an ordinary
Windows box with nothing wrong. The 23 were apt, brew, pacman and the rest of Linux — managers
that machine will never have. It is the first thing a new user sees.

This is fail-loud pointed at something that did not fail. The principle exists so a thing that
did not work cannot pass silently; a manager nobody asked for did not fail to do anything.
Spending the word "critical" on it is worse than cosmetic — it makes the real criticals
unreadable, which is the exact cost of an alarm that is always on.

The tell was that LiNix already disagreed with itself: the `check` rollup printed `ok health 25
backend(s) ready` while `check health` called the same machine 23-critical. **Two counts of one
machine, and no rule said which was right.** So `Absent` is a state rather than a filter, and
both views read one tally — a second way to count is a second answer.

**V.92 — Why a typo is exit 1 and not exit 2.** *(Owner ruling, 2026-07-27 — Q3.)* The readme
publishes four exit codes and says "the same four everywhere, so a script can branch on them".
Code `2` means *a read-only command looked and found work to do* — it exists so `linix check` in
CI can report drift without failing the job. Measured: `linix nosuchcommand`, `linix
--nosuchflag` and `linix sync --badflag` all exited **2**, because that is clap's convention for
a usage error and clap exits before LiNix's own mapping runs.

So the one code whose entire purpose is unattended scripting was ambiguous in exactly the
unattended case: **a CI job following the published table reads a mistyped command name as "the
machine has drifted"** and acts on it. A fifth code would have fixed the collision and broken
the property the table is for. `1` is already published, already means "something went wrong",
and is true — LiNix did not do what was asked.

Ruled alongside it because it is the same contract: **a refusal that exits 1 is a broken
promise, not a rounding error.** `purge-unmanaged`'s ratio refusal used `anyhow::bail!` rather
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
LiNix does not go to production until every registered backend has been thoroughly tested and
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
2026-07-28 — Q5.)* The flag was written for the three backends where LiNix itself fetches a URL,
makes the result executable and puts it on `PATH`, and it read as a relaxation of *LiNix's*
checksum rule. That framing turned out to be one case of a wider one: the thing being relaxed is
not "LiNix checked the bytes", it is **"something checked the bytes"**, and a manager can be that
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
   pair. helm's plain-HTTP switch addresses OCI registries LiNix does not reach, so `@allow_http`
   on `helm:` is still a line that does nothing, and still refused.
2. **The opt-out stays per line.** A batch whose specs disagree becomes two commands, because a
   flag on a shared command hands one line's decision to a line that never made it.
3. **It stays visible afterwards.** `status` lists what skipped a check for as long as it is
   installed — and the heading no longer says "downloaded", because for helm LiNix downloaded
   nothing.

The refusal text is the last piece. helm's own advice names `--verify=false`, an argv no
declaration can write; LiNix now appends the flag a user can actually put on the line.

**V.95 — Why a config file may take `apt` away from the built-in, and why it must say so.**
*(Owner ruling, 2026-07-28 — Q6.)* The onboarder's rule was absolute: custom backends register
last and a name already in use is skipped, "so a stray config can't hijack `apt` or `brew`". The
security half of that is right and stays. The absolute half was wrong, and the reason is the one
this codebase keeps meeting from the other direction: **the built-in is a snapshot of someone
else's CLI, and it goes stale.** helm v4 started refusing unsigned plugin sources; pixi renamed
`global upgrade-all`; nimble's `--` stopped meaning what it meant. Each of those was a day, a
week or a release where LiNix was simply wrong about a manager and the person in front of the
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
to run `linix unmanage` — was written in that story's vocabulary. So when the resource teardown
was added (S20), it was built as the extras' own business and never met the guard, because
nothing in the guard's vocabulary suggested it was about a symlink.

Measured on 2026-07-28: five `link:` lines deleted from a module, `[guard] max_removals = 1` and
`protected_packages = ["f3"]` both configured and both confirmed effective by `linix protected`.
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
value is that `1` and `3` answer two different questions: "something went wrong" and "LiNix
decided not to". For the entire SEC/T series it answered the first when the truth was the
second, so a CI job could not distinguish "the download was refused because it was plain HTTP"
from "the network was down" — and those want opposite responses. One is a config to fix, the
other is a retry.

**How it happened is the whole lesson.** Nobody chose `Validation` over `Refused` for these.
`Validation` is what you reach for when you are writing a check about a URL, and each of the
nine was written on its own day by someone thinking about that check and not about the exit
table three files away. E25 found one of them, in `purge-unmanaged`, and fixed that one. The
family was never swept, because there was a sentence saying it did not need to be.

**The sentence is the defect.** `main.rs` asserted that the `Error::Refused` arm was the one
point every refusal in the program passed through. It was true of every refusal the author had
in mind and false of nine they did not, and because it was written as a guarantee, the next
person to add a security check had a documented reason not to check. A comment that quantifies
over paths is a claim with no test attached, and `history.md` already records that shape as
costing more than the rest combined. It is now a test that enumerates the paths, which is the
only form of that sentence that stays true.

**The hook is worse than the code.** `on_guard_refusal` exists so a person can be told, without
watching, that LiNix said no. It fired for a mass removal — which is loud anyway, because the
command that triggered it is one somebody typed — and stayed silent for an unapproved hook, an
unverified binary and a secret written where nothing protects it, every one of which happens on
an unattended run. The hook was loudest where it was least needed.

**Two decisions inside the sweep, both the non-obvious way.** `rehearsal`'s "no container
runtime" stays a refusal rather than becoming a failure, because 7h's exit condition says it
refuses and names what is missing: the alternative is rehearsing on the host, which answers a
different question and calls it the same one. And a refused declaration is **kept** rather than
withdrawn, unlike E1's unresolvable name — LiNix refused the line as written, the refusal names
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
information, in LiNix's own voice.

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

---

**V.100 — Why a failure that survived its retries stops calling itself transient.**
`Retryability::Transient` is a claim: *a second attempt could differ*. The container harness
proves that claim the only way it can be proved — it retries once and calls a repeat a defect.
The product asserted it from a substring and nothing ever asked whether the substring was
right.

Measured: `luarocks install luafilesystem` on a machine where `https://luarocks.org/manifest-5.5`
returns 200 but the `wget` first on PATH is a scoop shim that rejects the flags luarocks passes.
The output contains "failed downloading", `exit_policy::luarocks` lists that as transient, so
LiNix kept the declaration and told the user `sync` would try it again. It fails identically
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
