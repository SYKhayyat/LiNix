# Part VII — Where the work stands

*[LiNix v7](../SPEC.md) — the map is there; this is one part of it.*

**Living section. It is the one place that records progress — Part III stays the plan, this
says how far it got (P4).** Update it at the end of every session. Everything below was
verified against the tree at the commit that last touched this section, not recalled.

> **This file stopped being living for 52 commits, and that is the finding of the 2026-07-26
> audit below.** It was last written at `c590036`; two sessions and the whole back half of Phase
> 7 landed after it. Because Part III's build list was derived *from this file* rather than from
> the tree, the plan then listed five shipped features as work still to do. **A living document
> that is not updated is not merely out of date — it is a source other documents copy from**, and
> the copy is what gets read. The rule at the top of this section is the fix and it was already
> written: *update it at the end of every session.*

## Session 2026-07-31 (fourth) — the coverage round: exemptions read against `Q4`

**`Q23` first, because CI was red on it on two platforms.** A package name beginning with `@` —
every scoped npm package — could not be written at all: `@` opens an option, so `npm:@bazel/bazelisk`
was read as an empty name followed by nonsense and refused with *"is not a list of `key=value`
options"*. `npm ls -g` prints those names and `linix list` reports them, so LiNix was refusing
names it had just printed. **RULED (owner): the leading `@` is part of the name**, positionally —
only the first character, so `npm:@angular/cli@version=17.3.0` still pins. Rule in II.2, reason in
**V.113**, which also names the shape all three of the day's grammar defects share: *a manager
prints a name LiNix's own grammar cannot take back* (`\` as set math, a byte-order mark, `@`).
Where the two disagree the grammar gives way, because the manager's names are facts and the
grammar is a choice.

**Then the coverage, and the first finding is the one that matters: `nix` was never installed.**
The tools image ran the nixos.org script as root, which refuses — `warning: installing Nix as root
is not supported by this script!` — and `|| echo "SKIP nix install"` swallowed it in step 12 of a
26-step build. For months the ledger recorded `nix` as *no path to a real lifecycle anywhere*: an
impossibility that was a broken line nobody read. It installs now (Determinate, `--init none`, the
container case), with **no `|| true` and a build-time assertion**.

**And installing it found a product bug in the first minute, which is the entire argument.**
`linix -y install nix:hello` succeeded, `nix profile list` showed hello, and
`linix list --backend nix` printed **nothing**. `nix profile list --json` returns `elements` as
an OBJECT KEYED BY NAME from schema v3 (Nix >= 2.20); LiNix asked `as_array()`, got `None`, and
reported an empty profile. That is `E6`'s class — a `list` blind to an installed package is
permanent phantom drift, LiNix reinstalling for ever while `check` reports a problem that is not
there — and it sat on the one backend no image had ever installed, so nothing could have caught
it. Both shapes are read now, from one extracted parser, against a fixture captured from the
running tool. **Verified in the image**: `linix list --backend nix` reports
`determinate-nix figlet nss-cacert`.

**The canary had to move too, and the harness said so itself.** `nix`'s canary was `hello` and so
is `cabal`'s; cabal has no uninstall verb, so its copy sits on PATH for the rest of the run and
the first nix lifecycle in this project's history reported *"nix: hello resolves to
/root/.cabal/bin/hello, which was already there before this install"* — G-3's rule, working, on a
collision built into the table. `figlet` now, because nothing else in the image installs it.

**The class is closed too.** The image writes what it actually ships to
`/etc/linix-image-managers` and the coverage audit reads it, so a manager that failed to install
is *missing and named* rather than impossible. Soft rather than hard on purpose: an architecture
can legitimately take one of thirty away, and a red run there teaches people to delete the line
rather than fix the install — which is how these lists got long.

**Four Windows exemptions were host-RESPECT, not impossibility.** LiNix can install a VS Code
extension, an Emacs package, a system-Python module or a mise shim perfectly well; the sweep
declined so a developer's machine would not be changed. **On a runner there is nobody to
inconvenience**, and `Q4` is explicit that an exemption is something the harness cannot do rather
than something it would rather not. `disposable_host` — `CI`, detected and never assumed — opens
`pip`, `vscode`, `emacs`, `mise` and `asdf` where the machine is thrown away afterwards.

**And `web:` had no lifecycle anywhere because of "no stable public canary"** — a search nobody
had run. A pinned GitHub release asset is exactly a stable public URL. `appimage` likewise: it had
been excused as *needing FUSE, which a plain container does not have*, when installing one is a
download and a symlink; its Windows reason is corrected to the true one, that Windows cannot run
an AppImage at all.

**The numbers, measured on this machine rather than predicted.**

| | before | after |
|---|---|---|
| `tools` image real lifecycles | 25 | **26** (`pass=359 fail=0 soft=11`) |
| Windows native real lifecycles | 5 | **6** |
| container gap ceiling | 11 | **10** |
| `nix` | *no path to a real lifecycle anywhere* | install → list → PATH → remove → gone |

**Three canary collisions were hiding in plain sight, and G-3 is what exposed them.** `cowsay`
was the canary for npm, pnpm, yarn AND bun; `pycowsay` for pipx and uv; `hello` for cabal and
nix. Whichever installed first owned the name, so every other backend's PATH check passed on
somebody else's binary — four checks where one was meaningful. One binary per backend now, each
verified from the registry rather than assumed.

**And the pattern repeated on `choco`, which is the third backend to break the moment it was
covered.** Chocolatey is skipped unless the shell is elevated, and GitHub's Windows runners are —
so CI 30684191791 is its first real lifecycle anywhere:

```text
  PASS  choco installed bat for real
  FAIL  choco: list shows bat (output missing /bat/)
  FAIL  choco: bat is not on PATH and nothing said where it went
  FAIL  choco: uninstall bat (rc=1)
        | removed as packages have dependencies for a reason...
```

**Answered 2026-08-01, by running it.** "The shell is not elevated" was a property of the
shell, not of the machine: one UAC consent raises an Administrator account's filtered token, and
the experiment then takes minutes. **All three lines are one defect, and it is LiNix's.**

`bat` is not the dependency-free package the harness claimed. **Measured**: an elevated `choco
install -y bat` pulls **eleven** packages — three chocolatey extensions, five KB hotfixes,
`less` and `vcredist140` — and `bat` installs last, after all ten. So there are ten ways for
this row to end with no `bat`, and the harness comment asserting there were none was never run.
**Which of the ten failed on the runner is still unmeasured** — all eleven install clean here.

Chocolatey raises its own exit code to 1 for a failed package **only when nothing has set one
already** ([`ChocolateyPackageService.cs`][cps]: `if (Failures != 0 && Environment.ExitCode == 0)`).
`usePackageExitCodes` is on by default, so a dependency that asks for a reboot leaves **3010**
standing over an install that installed nothing — and 3010 is on LiNix's `benign_exits` for
choco. LiNix read the code, called it success, and the harness printed `choco installed bat for
real` about a `bat` that was never there. Everything after follows: `list` cannot show it,
PATH cannot resolve it, and `choco uninstall bat` exits 1 with `bat is not installed. Cannot
uninstall a non-existent package.`

**The dependency reading was the report's own diagnostic hiding the evidence.** Chocolatey
prints six lines of generic troubleshooting advice after *any* uninstall failure, and one
paragraph of it happens to discuss dependencies. `detail_for_user` picks lines by the manager's
declared vocabulary and falls back to the **tail** when a policy declares none — and choco
declared none. So every choco failure ever reported was those six lines, with the sentence that
said what actually went wrong inside the `(13 more line(s))`. Reproduced verbatim on this box,
unelevated, by uninstalling a package that was not installed.

**The family is exactly two.** `choco` and `winget` are the only policies that forgive a
non-zero exit, and they were the only two with no way to say "and it still failed". winget
returns `-1978335212` for opposite events: `uninstall` of something already gone is the outcome
that was wanted, and `install` of a name that does not exist is not. Measured here, only its
wording separates them — `No installed package found` against `No package found` — so
`linix install winget:<typo>` reported success and left the bad line in the user's files.

Both now carry the failure vocabulary their exit codes cannot express. **The canary stays
`bat`**: a dependency-free package would pass this row without ever exercising the path that
broke, which is the green board `Q4` forbids.

[cps]: https://github.com/chocolatey/choco/blob/develop/src/chocolatey/infrastructure.app/services/ChocolateyPackageService.cs

**What stays a cost, said plainly:** `stack` (a Haskell package builds from source, and baking the
toolchain does not change that) and `flatpak` (the smallest runtime is multi-GB). Those are real
prices, not impossibilities, and they are written as prices.

## Session 2026-07-31 (third) — the round-6 grade, worked: eight findings and the blocker that came back

**Nothing here was reported by a user and nothing was found by running the suite.** The
independent grader graded `a510728` at **B−** and left `docs/GRADE-2026-07-31.md` plus four red
tests. All eight findings are closed, plus `E3`, which had *re-reproduced* — `cargo fmt --check`
failed on 12 diffs across 4 files, every one of them a file that session's own commits touched.
The gate was fixed in round 1 and rated HARD in both release scripts; it simply was not run. That
is E3 and E4's lesson arriving in the other order, and nothing in the tree stops the next one:
**a pre-commit hook is the obvious answer and is a change to how this repo is worked**, so it is
named here rather than made.

**`@shim` and `@sandbox` were write-once (G-1), and the fix deletes an engine rather than
repairing it.** Q19/Q20 closed five option keys that morning; these two were in the same table,
on the same `sync`, and failed one layer further in — read from the frozen `StateRegistry`, which
only an install writes. A present package line carrying either now resolves to a `shim:NAME`
extra, the identical noun a `shim:` statement declares, so the ledger, the guard, `--dry-run`,
`plan` and `check` all answer for it without a line of new machinery. `reconcile_all_shims` is
gone. Rule in II.2, reason in **V.111**. The removal-guard ledger is stronger for it: the shim
teardown was accounted for by *inheritance* and is now counted by `guard::enforce_extras`.

**A backslash in a package name was read as set math (G-2).** `winget:ARP\Machine\X64\Firefox`
— which is what **185 of the 278 names `winget list` reports on this box** look like — was
refused with "a module cannot use a set expression". The shield that protects `link:C:\…` covers
`<known backend>:` now, and set math between qualified packages still parses, because an operator
stands apart from its operands and a name never does.

**The two message findings are the same lesson twice: the machine was protected and the human was
not.** A failed backend command pasted its whole output at the user, escapes intact — ~110 lines
of scoop bucket commits with the sentence that mattered fourth (G-5) — while `ExitPolicy::opening`
had been stripping those same escapes so *detection* would work. The manager's own vocabulary now
picks the lines that explain the failure, the rest is counted and reachable at `-v`, and the
sanitiser is `validator::printable`, which W38 wrote for the grammar's refusals. And
`linix-failure-class: permanent` (G-6) — a good contract, printed unconditionally — was the first
line of the first command a new user runs; it is written only to a pipe now, which is exactly
where both harnesses read it.

**Three of the eight were about checks that could not fail.** `assert_binary_reachable` scored
`go: hello is on PATH` against *cabal's* binary on the tools image, in a run where the harness
printed the collision two lines earlier and handed the value that would have caught it to the
*other* assertion (G-3). The gate-parity checker compares script names while the claim is about
gates, so three asymmetries were live behind an `ok`: `--no-fail-fast`, the `storage` job, and
opensuse/void (G-4). And `nok` accepted any non-zero exit as "correctly refused" while LiNix
publishes exit 3 for refusing on purpose — sixteen of seventeen checks surviving a
fail-everything stub were refusal checks (G-8). That stub is now part of the mutation gate, with
its own ratchet per harness, so the class is measured on every push rather than by a grader once.

**One product defect fell out of G-8's measurement**, and it is round 2's `W22` family alive:
`restore` into a non-empty config directory refused with `Error::Other`, exiting 1 while its own
message said *"it refuses unless you pass --force"*. The round-2 sweep matched the phrase
*"refusing to"* and this site says *"refuses unless"*. Fixed, and the vocabulary widened with the
reason written down — including why `refuses to` is deliberately **not** in it: that is what
LiNix says about somebody *else* refusing (`Windows Task Scheduler refuses to register one`, an
`Error::Permission`), and including it produced exactly that false offender.

**`tests/fixtures/winget/` existed and was empty (G-7)**, on the platform this repo is developed
on. The orphan gate walks files, so a directory with none contributed nothing to count and stayed
green. Four cases captured from the real tool, the parser's own unit test switched off its
hand-built table — a synthesised fixture cannot disagree with the parser that reads it — and the
gate now fails on an empty directory, self-tested by planting one.

**Four things were built past the eight findings, each because the finding pointed at a class
rather than a line.** A negative check now asserts LiNix's *sentence* and not only a non-zero
code (`nok_saying`, five call sites per harness, patterns captured from the running binary):
that is what G-8 was really about, and it took the fail-everything survivors from 12 to 7 on
Windows and 17 to 12 in the container, with both ratchets lowered a step behind the measurement.
Every grammar refusal is drawn through `validator::printable`, so **a config saved by Notepad**
— a UTF-8 BOM is what Windows writes by default — no longer produces `` `cargo` is not a backend
LiNix uses; add `cargo` to your priority file ``, naming two strings that look identical. A
failed command's output is capped by width as well as by line count, because a stream with no
newlines in it is one line to `lines()` and winget draws its spinner with bare carriage returns.
And the grammar's punctuation family is pinned as a family: every mark `looks_like_expression`
fires on is asserted against a qualified line, so the next one needs no third discovery.

**The BOM was raised, and ruled the same day — `Q22`.** The message fix came first and the
behaviour was left for the owner, because reading a file LiNix used to refuse is visible from
outside the program. **RULED: read it.** A leading mark is stripped where config text enters a
parser — `parse_document` for the line files, `gated::read_inner` for `priority` and `active`,
and both TOML readers — never where a file is *read*, because `edit.rs` reads the same files in
order to append to them and II.16 forbids rewriting a user's file, encoding included. Only at
the start: a U+FEFF mid-line is still refused by name, and that is the control test. Rule in
II.1, reason in **V.112**.

Two things worth keeping from building it. **The preferences case was already green** — the
`toml` crate tolerates a BOM — so that assertion is a pin rather than a repair, and the test says
so instead of counting itself among the fixes. And **one of the new oracles was wrong**: the
preferences test first asked `linix policy`, which reports install/change rules and would never
have printed the setting it claimed to check. It failed, which is the only reason it was caught —
a check that cannot see the thing it names is the round-6 lesson arriving in the round-8 fix for
it.

**Pushed, and CI found two things the local runs could not.** Both are about the *machine*, which
is the half a developer's box cannot answer:

- **A budget measured here and enforced there.** The fail-everything stub leaves 12 survivors on
  this box and **14** on a clean ubuntu runner — two checks that fail here have nothing to act on
  there (`git log shows a linix commit`, `rebuild wrote no git commit`) — so the gate added this
  session went red in CI for being wrong about the machine rather than about the checks. The
  container budget is the runner's number now; the Windows harness went the other way (7 here, 6
  there) and its budget **tightened**. The sweep itself was identical on both: `pass=285 fail=0`.
- **`Integration (gentoo)` fails on `not one of LiNix's protected packages is installed here`,
  and it is the image rather than the code.** Ubuntu found `bash` the same hour with the same
  binary, so `linix protected` and the intersection work. `Dockerfile.gentoo` builds `FROM
  gentoo/stage3:latest` — a moving base — and today's pull has none of the protected names in
  emerge's database. The consequence is real and worth keeping red: **the guard proof on that
  image examined nothing**, and the subcommand audit failed behind it because `uninstall` is only
  ever executed by the guard victim on a SMOKE_ONLY image. The fix is in the image (record a
  protected package) or in `default_protected_packages`; it is not a harness bug and must not be
  softened into one.

**Two shell lessons worth keeping.** There are no locals in a POSIX shell: `harness-logic-test.sh`
lifts function bodies and runs them against globals of its own, so naming a parameter `_rlog`
inside `assert_binary_reachable` clobbered the test's own `$_rlog` and broke three predicates that
had nothing to do with the change. And a check written to catch the collision passed on the
unfixed tree, because it asked whether the word "shim" appeared anywhere in a JSON document that
always contains it — the first draft of an assertion is where the free green lives.

## Session 2026-07-31 (second) — `Q19` and `Q20` ruled, and `D15` was never really parked

**`Q20` was not reported by anyone — it was found by asking the rest of the tree the question
`Q19` had just answered.** *What else is applied once at install and never again?* `@classic` on
`snap:` came back: read in exactly one place, when the install argv is built, so a snap that
gained the option after it was installed stayed strictly confined for ever with `sync` reporting
nothing to do. **RULED (owner): the same answer** — an edited `@classic` converges.

The two directions came out asymmetric because snapd is: `snap refresh --classic` relaxes
confinement in place and nothing narrows it back, so `@classic=false` on a classic snap is
refused by name with the by-hand path spelled out (V.107's shrink refusal, one backend over).
**Omitting `@classic` manages nothing**, which is what keeps that refusal off configs nobody
edited. Rule in II.2, reason in V.108. **Not run** — no container can host snapd, so it is
argv- and unit-tested against real `snap info` output and named as unexecuted, like `zfs:`.

**And the sibling was inside the sibling.** `@channel`'s drift check `return`ed from the
function — the same fault `Q19` had just fixed for `@mount`, in the branch immediately above it —
so a snap carrying a channel *and* `@classic` had only the channel looked at. The argv had the
matching bug: the refresh was built from `@channel` alone and would have dropped one of two
requested changes. Both fixed; one refresh now carries both switches. `snap info` is also read
once instead of twice (`is_installed` asked `snap list` for a fact printed on the same page).

**`V.109` — the register's checker now verifies parked *conditions*, not just totals.** It
counted entries and failed CI on any wrong number, and it passed every day of the week D15 was
mis-filed: the arithmetic was right the whole time the register was wrong. A parked entry's
`Status:` line must now carry `waits on <what>`; the check fails if the clause is missing or
names a decision that has since been ANSWERED. D16 was caught immediately — it had never said
what it waits on. Self-tested by pointing it at an answered decision and watching it fire.

**Q19 asked what a *changed* `@quota` or `@size` meant, and the code's answer was "nothing".**
Q18 had made the keys writable and applied them at creation; the volume then exists under its
name, so there is no drift to act on, so editing the number left `sync` reporting success over a
declaration it had stopped applying. **RULED (owner): it edits — and where editing can lose data,
the line has to say so.** The builder had recommended refusing to shrink outright; the owner
overrode that half and required a flag instead.

**Built:** `@quota` re-applies on `btrfs:`/`zfs:`; `@size` grows via `lvextend --resizefs` and
shrinks only behind `@allow_shrink=true`, refusing otherwise with both sizes named.
`--resizefs` runs in both directions — that is what makes the flag a permission to resize rather
than to truncate, and it is why xfs (which cannot shrink) fails before the volume is touched.
`@allow_shrink` without `@size` is a parse error, the same one level in `@mount_options` gets.
Rule in II.2 and XIII, reason in V.107.

**The comparison is in bytes and only the declared side is parsed.** Every tool is asked for raw
counts (`zfs list -p`, `lvs --units b --nosuffix`, `btrfs qgroup show --raw`) through one
`core::size`, so `@quota=10240M` against a reported `10737418240` is not a change. D13's failure
mode — a change reported on every sync for ever — was the thing being designed against.

**Two siblings, found by looking rather than by being reported.** `@mount`'s drift check
*returned* from the function, so a line carrying both a mount and a quota had only the mount
looked at; the facets are OR-ed now. And **`@mount_options` was dead the same way** — the fstab
entry is rewritten on every install, but no install was ever scheduled, so a changed option field
survived every sync and every reboot. Checked and **not** affected: `@shim`/`@sandbox`, which have
their own reconcile pass over every managed package (`sync/mod.rs::reconcile_all_shims`) and
converge already. **Still live, and raised rather than built: `@classic` on `snap:`** is read only
when the install argv is built, so flipping it does nothing — the same class, but honouring it
means reinstalling a package when a flag changes, which is a ruling and not a detail.

**Proven on a real logical volume, not argued** (§14b, new; storage image `pass=274 fail=0` →
**`pass=279 fail=0 soft=7`**, the ratchet holding at 5 ≥ 5): grow `67108864 -> 134217728`, a
converged re-sync that left it alone, a smaller `@size` refused by name with the volume untouched,
and `@allow_shrink` shrinking it back. The edit goes into the module file and is applied by
`sync` — `install` would prove the argv and skip the planner, which is the half that was broken.
The converged re-sync is the assertion nothing else could give: D13's failure mode is invisible
to a harness that syncs once. `btrfs:`/`zfs:` did not run — this kernel has neither module — so
the `@quota` re-apply is unit-tested and unexecuted, and is named that way rather than counted.

**`Q21` generalised both into a rule, and `D15` was re-parked on a measurement.** Two options
found dead in one afternoon by looking is a class, so the owner ruled the class: **changing an
option changes the machine, or the line is refused with a reason** — "nothing happens" is not an
outcome, absence manages nothing, and **the proof is per option, not per backend**, because
`snap:` had a real lifecycle for months and it is precisely what `@classic` survived. Rule in
II.2, reason in V.110, sweep in `plan.md` Tier 0 (`0c-bis`) and in `GRADER.md` §3.5 — the grader
carries the check, the register carries the obligation. **Five options done, the rest unaudited**,
and that is written down rather than implied.

`D15` was reopened and **re-parked the same day, deliberately, on a different condition**: what
snapd actually does to a sideloaded snap. The two possible answers fail in opposite directions
and need opposite fixes, and nobody here has observed which is real — so it waits on twenty
minutes of experiment rather than on an argument. The register is at **zero open**.

`GRADER.md` gained the two sections this session earned: **§3.5**, the option-change sweep no
lifecycle performs, and **§2.2b**, the checker that verifies the arithmetic and not the claim —
`decision-count.sh` was green every day of the week `D15` was mis-filed, because it checked the
totals and the totals were never the claim.

**`D15` was reopened.** It had said *PARKED until D5 is answered* since the register was written;
D5 was ruled 2026-07-24 and built 2026-07-26, and the entry went on saying PARKED — filed in the
section for questions that need nothing. **A parking condition is a claim that has to be
re-checked, and nothing re-reads those conditions when the thing they wait on arrives.** The
register's status table now says so, and the Q-series got its own heading: nineteen entries had
been sitting under *Parked or closed* because that was the last heading in the file when they
were appended.

## Session 2026-07-31 — `Q18` ruled: five keys the code read and the grammar refused

**The question was which half of Part II was wrong, and the owner ruled it was the option table**
— with a direction attached that shaped everything below: *broaden until everything the code can
do can be written; never restrict the code to fit the table.* So the narrowed form the builder had
offered (land `@size` and `@quota`, hold `@mount` back until the fstab path had been proven) was
declined, and the fstab path was fixed and exercised instead.

**`lvm:` is writable for the first time since it was merged.** `@size` on `lvm:`, `@quota` and
`@mount` on `btrfs:` and `zfs:`, `@mount_options` on `btrfs:` — legal on the backends that read
them, refused by name everywhere else, in the shape `@url` (U39) already used. `@options` was
respelled `@mount_options`: no line could ever carry the old spelling, so there was nothing to
migrate, and a key called `options` in a flat namespace is a collision waiting for the next
feature.

**The audit found three more keys in the identical state, and the third one is the lesson.**
Every `.get("…")` against an options map was read: `snap`'s `@classic` had never reached its
`--classic` branch, and `@shim` / `@sandbox` — which `sync` reads to decide whether a tool gets
a PATH stand-in — were refused by the parser. **R3 deleted the imperative `shim` command in July
and pointed at `@shim=true` on the line as the declarative way, and the change that closed the
option table into a whitelist did not contain `shim` — so the ruling pointed at the one form that
did not parse.** Shims were not unmakeable: a standalone `shim:NAME` statement still parsed and
is still reconciled, which reading `app/apply/dependents.rs` settled after the first draft of this
entry claimed otherwise. Two defensible changes, and between them a documented form no file could
contain, every test green. The two lists are now
one: `backends/capability.rs` (moved up out of `artifact/`, since it now answers for storage and
confinement too) holds the table, the grammar and the install path both read it, and
`every_scoped_option_is_a_legal_option_key` asserts the join. `validate_artifact_options` is
`validate_backend_options` for the same reason — it stopped being about artifacts three keys ago.

**Making `@mount` reachable exposed what the fstab code had been doing, unexecuted, since it was
written.** It dropped every fstab line *containing* the mount point as a substring — declaring
`/mnt` would have deleted `/mnt/data`, `/mnt/home` and any comment naming the path. It wrote
`subvol=` as the declared path rather than the path from the filesystem root, which is the same
offset bug `list` had been fixed for the day before, mirrored, and produces an entry that mounts
nothing on any machine whose root is a subvolume — which is Fedora, openSUSE and Garuda by
default. Removal left the entry behind, and an fstab line naming a subvolume that no longer
exists stops the next boot in the initramfs. All three fixed, with the logic in pure functions so
the tests need no root: field-wise matching, the `subvol_arg` inverse of `subvolume_paths`, and a
removal that takes the entry and unmounts *before* deleting, because a mounted subvolume cannot
be deleted.

**Running it found three more, and none of them was findable by reading.** The first real
`@mount=` on a real filesystem failed on `Could not determine BTRFS UUID`, twice, for two
different reasons stacked on each other: the parser wanted a line *starting* `uuid:` when the
report reads `Label: none  uuid: …`, and once that was fixed it was still asking
`btrfs filesystem show` about the **subvolume**, which answers `not a valid btrfs filesystem` —
it takes a filesystem, so the question can only be put at the mount point. `subvol_arg` already
knew the mount point and was throwing it away. Third: `info()` — which is what the *planner*
asks, not `list_installed` — answered `Path::exists`, so any directory was an installed
subvolume and the `Package` it built carried no properties at all. It goes through the listing
now, like `zfs:` and `lvm:` always did.

**A half-applied storage declaration used to report itself as satisfied for ever.** The failed
mount left the subvolume created; the name was therefore present; `sync` said *already up to
date* over a declaration it had never finished. So a declared `@mount=` that does not match what
the machine reports is drift now, in `spec_is_missing` beside `@version` and `@channel` (D13).
**Mounted nowhere is a state, not an unknown** — the first draft copied D13's leave-unreadable-
alone rule and put the whole bug straight back, because "not mounted" is exactly what an absent
mountpoint means. Re-applying is idempotent, so being wrong costs a repeated no-op; the other
reading costs a mount that never happens.

**And `@mount` gives one subvolume a second name, which `list` now collapses.** A subvolume
mounted elsewhere is reachable by two paths and the second is undeclared — `remove-orphans` would
have offered to destroy the volume the user had just declared, under its other path. Identity is
the device plus the path from that filesystem's root; the name reported is the one reached through
the mount closest to the filesystem root. **Not the shortest name**, which was the first rule
written and was wrong: `/srv` is shorter than `/mnt/fs/data`, and answering it would have left the
declaration looking unfulfilled and `sync` re-creating the subvolume every run — the 2026-07-30
bug from the opposite side, caught by writing the harness canary before believing the rule.

**`lvm:` then failed for a reason that was not LiNix, and proving that was the point.** The
lifecycle refused with `/dev/linixvg/canary: not found: device not cleared` — and a hand-run
`lvcreate` with the identical argv failed the same way, which is how it was ruled out as a
product defect in one command. The image had *meant* to handle this: it `sed`-ed three udev keys
in `/etc/lvm/lvm.conf` from `= 1` to `= 0`, but Ubuntu ships them **commented out**, so the sed
matched nothing — and the `grep` that was supposed to prove it had worked matched the comments
and passed. **A build-time check that cannot fail.** The settings now go in `lvmlocal.conf`,
which is read after `lvm.conf` and cannot be silently outvoted, and 13c probes with a real
volume made and destroyed before it claims lvm has a canary, so a container that cannot make one
says so in its own words instead of looking like a LiNix bug (Q17's rule).

**Measured, on real devices: `pass=274 fail=0 soft=7`**, and the storage image's real-lifecycle
floor goes 3 → 5 — apt, github, btrfs, cargo, lvm. `zfs` is still not among them, because this
kernel has no ZFS module, which `Q4` counts as a release blocker rather than an exemption.

**The deliberately-red job is green and gates every push.** `lvm:VG/LV@size=64M` was written the
way Part II said it should be and failed by name every run, on purpose, for a day; `btrfs`'s
canary now carries `@quota` and `@mount` so the fstab code runs on a real filesystem rather than
being legal on paper. The storage job moved out of the nightly schedule, which is what its own
comment said to do the moment this was ruled.

**One thing was raised rather than decided: `Q19`.** `@mount` converges, and `@quota` / `@size`
do not — editing a declared size changes nothing on the next sync. Re-applying a quota is a safe
idempotent property write; honouring a changed `@size` on `lvm:` means `lvextend` to grow, and
its mirror image **destroys data**. A declaration that may resize a filesystem, and whether
shrinking is refused by name, is the owner's call and not a comparison to bolt on, so it is in
the register with the recommendation and not in the code.

Ruled in `decisions.md` `Q18`, `target-state.md` II.2 and the storage paragraph, `why.md` V.106,
all in the commit that carries the code.

## Session 2026-07-30 (third) — the round-6 orders: a typo that installs software, and three checks that were wrong before the code was

`docs/GRADE-2026-07-30-round-5.md` graded the tree **B−** for entirely different reasons than the
round before it, and `docs/BUILDER.md` turned its findings into `W33`–`W43`. All eleven are
built. Every one of the grader's red tests is green without an assertion being lowered, and each
fix is at the mechanism rather than the instance — which is also why four of them found a second
defect the grade had not seen.

**W33 / Q16 — a bare grammar keyword was a package name.** A module containing the single word
`link` declared `cargo:link`, `--dry-run sync` planned the install, and `check` recommended it.
Ruled: refuse the bare word; `list:NAME` was already what a bare `NAME` meant (II.2), so the
escape hatch needed no new grammar and V.10's rejection of quoting stands. **Built for
twenty-two words, not the thirteen measured** — `exec`, `dotfiles`, `firewall`, `generate`,
`exclude`, `intersect`, `module`, `use` and `param` reach the package parser by the identical
route, and refusing thirteen of twenty-two would have been the reported symptom fixed and the
class left live. Three copies of the prefix list were collapsed into one, and the disagreement
between them was a live bug of its own, measured rather than argued: a `generate:` line whose
payload is a Windows path parsed as a **set expression**, because the copy deciding whether a
backslash meant set math had never heard of `generate:`. A first draft of the why entry claimed
`setting:` had the same fault; the test written to prove it disproved it, and V.103 says so.

**W34 — `adopt` re-declared what the manifest already named.** `discover()` asked the
managed-state registry and never read the manifests, so a package written by hand and not yet
synced was offered again and written into `adopted.txt` — after which *"Deleting a line
UNINSTALLS that package"* was false for exactly the lines a user wrote themselves. Now
subtracted from the resolved model, excluding `adopt`'s own manifest (II.9: adopting twice
answers *the machine as it is*). The summary's *"(listed in the manifest)"* — a reason wrong for
100% of its items, always — is replaced by the per-item reasons each `Skipped` already carried.
Sibling fixed: `--dry-run unhold` said *"would release 1 hold"* and then *"0 hold(s) were not
recorded"* about the same hold.

**W35 / W36 — the classification was computed and then not consulted, in three places.** A rate
limit is `Transient` in `error.rs` and `why_kept` had no `Transient` branch, so a user was told
*"Nothing classified the failure above"* about a failure classified three lines away. `heal`
printed `Some(CommandFailed { retry: Permanent, absent_name: true })` at the user, advised
re-running `sync`, and **exited 0** after saying an operation could not be recovered. Both now
read `retryability()` and `says_a_name_is_absent()`. The behaviour underneath `heal` — leaving a
failed recovery's entry `InProgress` rather than closing it — was already right and is now pinned
by a test, on a fixture the grader recorded as impossible to automate: a planted **Remove** for
`cargo`, which fails locally, deterministically, with no network, on every runner that builds
LiNix.

**W37 — and the reason it looked like a Mac-only defect.** `list` reported
`service:com.apple.SafariHistoryServiceAgent` and `info` denied it, because `service`, `link` and
`setting` are each both a grammar prefix and a registered backend: a string copied out of a
listing parses as a typed resource statement, and everything downstream understood only packages.
`Statement::listed_as` reads the pair off the variant rather than splitting on `:` a second time.
**It was invisible on Windows because `sc query type= service state= active` is rejected by `sc`
— which exits 0 while printing `ERROR:`, so `linix list -b service` returned an empty list on
every Windows machine since that row was written.** With the argv corrected the contradiction
reproduces here, which is how it was fixed rather than guessed at from a CI log.

**W38 — a refusal about a character nobody can see, that named no file and echoed itself.** The
character validator's refusals now carry the same `Origin` the grammar's do, and `printable()`
names a codepoint (`<U+202E>`) instead of emitting it. `rustc` refuses to compile a doc comment
containing that character; LiNix printed it. The family sweep — five hostile characters through
five commands, asserted on raw bytes — found **sixteen** leaks against the grader's two.

**W39, W40, W41, W42, W43 — the gates.** A floor under `CAUGHT` in the mutation gate, which could
not tell *the checks got stronger* from *the checks were deleted*. The orphan fixture wired up,
and a check that no fixture goes unread again. A budget per command class, with the number in the
output when it is crossed. The flag-drift gate re-aimed to branch on **whether the tool documents
verification at all**, which is the one question `accepts_flag` cannot answer — and without which
the obvious two-directional rewrite is green on a drifted table, which the first draft was.
`bundle` no longer writes its nine files under `--dry-run`.

**Three of the eleven found their own instrument at fault before the product.** The orphan-fixture
check reported twenty orphans and nineteen were its own — it never looked in `src/`, where half
the fixtures are read. The flag-drift rewrite passed a planted drift. The `setting:` claim was
disproved by the test written to prove it. Each is recorded where it happened rather than quietly
corrected, because *a check that examines the wrong thing and reports a finding* is the same
defect as one that reports success, and this document's whole subject is the second.

**Two corrections to the grade, which is the specification.** R-9 says *"nothing measures
latency"* — `tests/latency_budget_tests.rs` has existed since `02a4ec9`, with two ceilings and a
call-counting property test. What was missing is a budget per command *class* and a report to the
user, and that is what was built. Q15's `export` half was ruled with `bundle` on the reasoning
and never measured; measured now, **`export` already honoured the flag**, so W43's code change is
`bundle` alone.

**And the order that must not be built.** `BUILDER.md` carries W18 — split the backends into
*supported* and *experimental*, say so in `check health`, in `priority` and in the readme — as
"the highest-value change in the document", marked ⚠️ *needs a ruling*, and repeats in two more
places that it is still untouched. **It had the ruling, and the ruling was no.** `Q4`, owner,
**2026-07-27**, three days before round 6's section was written: *"Are unverified backends
labelled experimental? NO. They are tested instead, and nothing ships until they are."* The
reason is a rule about the project (`V.93`): *this codebase does things; it does not cover for
not doing them* — a label converts an unfinished job into a permanent disclaimer, and the gap
would read as handled rather than open.

I nearly built it, and the only reason I did not is that I went to the register to find out
which ⚠️ rows were still open instead of trusting the document that listed them. **The register
is the authority and the map was stale** — which is `E31`'s own finding, turning up in the
document that reported it. `BUILDER.md` is corrected in four places, and this paragraph
originally repeated the same claim before the register contradicted it.

**Untouched, and now the one thing that would move the grade most:** a real lifecycle for the 24
of 56 backends that have never had one — which under `Q4` is a release blocker rather than
something a caption makes acceptable. Nothing here touched the safety core, which came through
round 5's adversarial pass intact.

## Session 2026-07-30 (second) — the round-4 grade: a preview that armed a removal, and four gates that examined the wrong thing

`docs/GRADE-2026-07-30.md` graded the tree **B−** and named one blocker and four defects. All five
are closed, each at the mechanism rather than at the instance, and the round's own thirteen red
tests — committed by the grader in five files — are green without their assertions being lowered.
The one-line diagnosis of the round was **"every finding is a gate that examines the wrong thing
while the product does the wrong thing behind it"**, so each entry below says what the gate now
examines as well as what the product now does.

**B-1 — `--dry-run adopt` recorded 112 packages as managed and wrote no manifest.** Managed and
undeclared is the state the model reads as *the user deleted every line*: `linix check` then said
`112 to remove … run linix sync`. The cause was **two writers**. `utils::file::write_config`
honoured the flag exactly as II.8b describes; `atomic_write`, the primitive under it, was public,
shorter, and what every `save()` had always called — the registry, the WAL, and the `github:`/
`web:` backends' own state files. A writer that honours the flag is no protection while a writer
that ignores it sits beside it. There is one now, `utils::file::persist`, returning **whether the
bytes landed**; `atomic_write` is private. `hold`, `unhold`, `adopt` and `path --set` phrase their
output from that answer, so `Held 1 package(s).` and a write that never happened can no longer
appear together. Seventeen `save()` sites inherit it without being touched, including the three
the grade could not measure (`shell` exit, the lease sweep, the pm-hook recorders).
Measured after: `--dry-run adopt -y` on a fresh config finds 112 packages, writes nothing (`data/`
holds only the lock file), and `check` says *the machine matches your files*.

**The gate that could not see it, and the exemptions that hid it.**
`tests/dry_run_every_verb_tests.rs` snapshotted the **config** directory only and excused three
verbs *because they wrote to the other one* — "holds live in the data dir, not the config dir".
That is not a reason; it is the finding, written down and believed for two rounds. Both snapshots
now walk the whole fixture (config, data, and the working directory, with the binary run there),
and `hold`, `unhold`, `heal` and `adopt` are driven — `heal` against a hand-built WAL entry, which
is the only fixture that gives it work. A host that cannot supply the work is **skipped by name and
counted**. `hooks` and `path` kept their exemptions with true reasons instead of false ones
(`hooks install` writes to system hook dirs; `path --set` writes LiNix's own settings file, which
no fixture can relocate) — and `path --set` is driven anyway, by a test that restores the real
settings file in both directions if the fix ever regresses. **Mutation-checked**: with the flag
test in `persist` replaced by `if false`, the gate goes red on five cases and the grader's
`dry_run_hold` test goes red with it; restored, both are green.

**The past tense that outlived it, on the two verbs the grade recorded as still reproducing.**
`--dry-run lock` printed `Lock: pinned 0 package version(s) to …` beside its own `[DRY-RUN] would
write …` for the same file, and up to seven more `Lock: approved …` lines; `--dry-run heal`
printed `repaired: reconciled locks/versions.json`. Both now read their tense off what
`persist` answered — `[DRY-RUN] Lock: would pin`, `heal: would reconcile` — and `heal`'s label
itself changed, because `repaired:` was the last past-tense word in that output and no writer's
answer could reach it.

**P-3 — `linix protected <bare name>` was wrong for every name**, including `jq` (its own help's
example) and `sudo`, which *is* protected and got the wrong reason. `protection_of` was handed
`""` for the backend and opened with `is_declarable("", name)`, which builds the line `:jq` — a
line no grammar accepts — so the declarability test fired before a rule was read. **An unknown
backend is now `Option::None`, not an empty string**, in both `is_declarable` and
`protection_of`: a bare name is checked against the config rules, and the reason says the OS's
essential list was not consulted because it is keyed by backend. The inspector and the enforcer
answer the same question again. `tests/unknown_backend_family_tests.rs` exempted this verb as
taking "nothing — it lists the protected set"; it takes `[PACKAGES]...`, the exemption is deleted,
and `protected` now refuses an unknown prefix the way the rest of that family does (N-3).

**P-1 — `plan` predicted a refusal `apply` does not make.** It merged resources into the package
removal list and asked `RemovalKind::Package`, so every `link:` came back `Undeclarable` and the
user was told a dotfile teardown would be refused *because its manager reports a name no package
line can hold*. `apply` then performed it at rc=0. The guard carries a unit test asserting this
cannot happen — it exercises `RemovalKind::Extra`, and the product called the other function.
`plan` now inspects each list as its own kind, each counting the other against the same ceiling,
which is the split `sync` already used.

**P-2 — `sync` re-placed every declared resource on every run**, and the second run left
`.linix-backup` files beside the user's dotfiles: backups of the copies LiNix itself had made,
under a summary reading `already up to date`. `check` and `plan` used the `in_effect` probe;
the loop that does the work asked nothing. The probe is now one function all three call, it is
compared **by content** rather than by existence (the destination existing is what the ledger
already knew, and on Windows the deploy falls back to a copy so `read_link` answers nothing), and
the `link:` backend skips a destination that already matches its source. A resource nothing can
verify — a `setting:`, a `service:`, a decrypted secret — is still placed, and still named
`unverifiable` rather than assumed converged.

**P-4 — pixi's `list` parser invented a package.** `pixi global list` prints each environment's
properties as indented children, and `exposes: rg` was read as an installed package whose version
was `rg`. Depth is the tool's own structure, so the parser reads it rather than blocklisting the
property names pixi prints today. **The family, not the instance**: `parse_bun_list` and
`parse_yarn_list` trimmed the tree glyphs off before reading too — `bun pm ls -g --all` nests four
levels, so that parser was one flag away from reporting forty dependencies as global installs.
Both now take the top level only, asserted against **bun's own output, captured on this machine**
in the flat and nested forms.

**And the breadth axis, which four rounds of grades say has never moved.** Fixtures captured from
the tools themselves — `scoop list`, `choco list -r`, `pipx list --json`, `helm plugin list`,
`npm`/`pnpm ls -g --json`, `bun pm ls -g` flat and nested — each with a
`tests/parser_fixture_tests.rs` expectation written by reading the file rather than the parser.
That is 8 backends with a fixture → **13 of 48**.

**The first batch found a defect on its first run, which is the argument for the batch.** `pipx
list --json` reports a version under `main_package.package_version`; `parse_pipx_json` read
`main_package.version`, a key that schema does not have, and fell back to the string `unknown` —
for **every pipx package on every machine, silently**. `linix list -b pipx` printed `unknown`, and
`lock` skips a version reading `unknown`, so `linix lock` pinned no pipx package and said nothing
about it. Now `pycowsay 0.0.0.2`. Nothing in 1,500 tests could see it, because the parser's own
test wrote its own JSON: a hand-written fixture is the one thing a schema bug cannot fail. It is
still the axis with the most left on it — no backend meets the four-case rule
(empty/single/not-found/error), and these are single-case `list` captures. `.gitattributes` now pins `tests/fixtures/**` to LF, for
`scripts/lifecycle-floor.txt`'s reason one directory over: a parser fixture's line endings are the
content under test, and the pixi fixture was committed with git warning it would rewrite them.

**Left for the owner, untouched:** **Q14** (the flag half of the argv-drift gate, still OPEN), and
`E15`/`E33`. The `tools`, fedora, arch, alpine and gentoo images were not built this session; the
destructive effectors, the SIGKILL/`heal` loop and real `sudo` remain unrun, as in every round.

## Session 2026-07-30 — what CI said about the previous session, and the gate that was blind where it mattered

The branch was pushed and CI dispatched (run **30503630610**, `workflow_dispatch` because that is
the only event that also runs the nightly jobs). **11 of 16 jobs green**, including every leg the
last session listed as unproven: the `tools` image, gentoo, and the macOS and Windows native
sweeps. Five failed, and all five were this branch's own work.

**Round 3's N-5 was worse than the grade described, and last session's fix did not touch it.** The
grade said the ratchet was absent from the container legs; mounting the floor file made it
*present*. It was still in force nowhere. The floor recorded exactly one host class — a
developer's Windows box — so **7 of 7 classes in the matrix** took the "no record for this class
yet" branch, which was a counted `PASS`. A ratchet that has never compared anything, reporting
green on every platform. Nothing in the suite noticed except the in-container mutation gate,
counting one more check that survives a do-nothing binary (93 against a budget of 92).

Three defects under one finding, each fixed:

- **The key could not tell two images apart.** It read `/etc/os-release`, which answers `ubuntu`
  for both the ubuntu image and `tools` — because `tools` is built on Ubuntu. Measured on that
  run: 25 real lifecycles and 7, filed under one record. Each Dockerfile now declares its own
  `LINIX_IT_IMAGE`, and a predicate fails if one does not.
- **No container class had a record.** Every number in `lifecycle-floor.txt` below the first was
  read off that run rather than chosen.
- **A first run is now counted as neither pass nor failure**, and a `SMOKE_ONLY` run is not judged
  at all — its honest count is 0, which no floor can sit under.

**The CRLF gate was blind on the only platform where the bug it guards happens.** `head -c … |
grep -q $CR` does not fire on a CRLF file under Git Bash: MSYS grep opens files in text mode and
normalises CRLF before matching. It had caught a *lone* CR — one not followed by LF, which no
translation touches — which is exactly how it passed its own review last session. It now goes
through `od -c` and **tests its own detector against a file that must trip it** before concluding
anything from a negative. Same fix in `run.sh`'s pre-build check, its twin. And
`scripts/lifecycle-floor.txt` — mounted into the container and parsed there — was the one mounted
file `.gitattributes` did not pin to LF and no glob covered; a trailing CR makes the ratchet
compare against something that is not a number, and `[ -lt ]` failing takes the branch that
reports it satisfied.

**Two fixtures named a backend the host may not have**: the grader's `check` control (`scoop:` —
`not a backend` on macOS, `not in your priority list` on a Windows runner) and, in this session's
own new file, `repo:scoop:`. Bare is not the fix: an unqualified name is searched for across the
managers and a deliberately absent one is refused outright. A qualified cross-platform backend is,
and `repo:` is read from the `priority` the fixture just wrote.

**`cargo test` is fail-fast across targets, so each host reported one failing target and skipped
the rest** — three findings visible and the remainder of the suite unmeasured on every platform,
at 45 minutes a dispatch to learn the next one. `--no-fail-fast` now, and the whole suite was run
locally on Linux in a container (with helm 3 installed on purpose) rather than one failure per
dispatch.

**Verified**: Windows 43 targets / 0 failures, clippy `-D warnings` clean, `fmt` clean; Linux in
a container **37 of 38 targets**; harness predicates **57/57**, each new one watched failing first;
`decision-count.sh --check` ok.

**Confirmed on CI** (run **30508477764**, the same dispatch shape): **15 of 16 jobs green**, the
sixteenth being the tag-gated release job. `Integration (ubuntu)` now reports `PASS real-lifecycle
ratchet: 7 >= 7 recorded for container-linux-ubuntu-local` — the first time in this repo's history
that the ratchet has compared a container run against a record — and its mutation gate reads **45
caught, 92 survivors within the budget of 92**, where it was 44 and 93 before. The four distro
legs and `tools` all cleared the floors read off the previous run, so those numbers reproduce. On
`Build on ubuntu-latest`, `--no-fail-fast` ran the whole suite: **42 targets green and exactly one
failure**, which is the one below.

**Open, and deliberately not answered in code: Q14.** The one remaining red target is
`grade2_flag_drift_blindspot_tests`, and it is red for a reason that is not drift. The capability
table names helm **4**'s `--verify=false`; on helm 3 the flag does not exist, LiNix withholds it,
and `@unverified` on a `helm:` line is accepted and does nothing. On such a host withholding a
correct flag and withholding a drifted one are the same action, so **no assertion can tell them
apart** — the grader's own gate, verified green on their helm 4.2.3 box, cannot pass on helm 3.
Left red on purpose: it is the owner's ruling, and a red test is where an unanswered question
belongs.

## Session 2026-07-29 (later) — round 3's six findings, and two rulings that had been half-applied

`docs/GRADE-2026-07-29.md` graded the tree **B** and left nine red tests for six findings. All six
are closed and every red test is green, at the mechanism rather than the instance. The two the
grade said to do first (N-1, N-2) were both the same shape: **a fix had gone to the instance that
had a reproduction attached, and the family was never enumerated from the code.**

**N-1 — E1 came back because withdrawal read the wrong property.** `install` withdrew a wedged
line when the failure was `CommandFailed { retry: Permanent }`, and that reading is wrong in both
directions. Only **12 of 48** backends had an `ExitPolicy` at all, so the other 36 could never
produce that verdict — a mistyped `npm:` package wedged the config while the identical typo behind
`scoop:` did not, and nothing about npm was special. And permanence is not existence: helm's
`plugin already exists` is permanent about a name that is plainly there. `ExitPolicy` answers the
two questions separately now (`absent_markers` vs `permanent_markers`), `Error::NoSuchPackage`
carries the name for backends that resolve names themselves, and `says_a_name_is_absent()` is the
one predicate every caller reads. Prose is demoted to picking *which* line in a batch — pixi wraps
its output through the middle of a package name, which is what a prose-parsing reader looks like
when it finally meets a manager that formats. Five managers gained a policy (npm, gem, pipx, go,
pixi), every phrasing captured from the tool on this host and dated in the source; three had
theirs split (cargo, nimble, scoop) so a real crate with no binary, a wrong version pin and a
failed *uninstall* stop withdrawing declarations. **36 backends still cannot report a missing
name, and that is now a derived, ratcheted set** rather than something discovered one backend at a
time (`tests/absent_marker_coverage_tests.rs`, a set and not a count because 48 backends register
on Windows and 56 on Ubuntu).

**N-2 — the model was missing half of itself.** `link:`, `service:`, `setting:`, `shim:`,
`schedule:` and `repo:` were outside the model everywhere except the apply loop. One computation
(`Extras::changes`) now feeds `check`, `check drift`, `plan`, `apply` and `sync`'s summary; `plan`
freezes resources in the file and in the hash and `apply` executes them through the one phase
list. The third value matters as much as the other two: a resource LiNix cannot read back is
**named**, not assumed converged. Full reasoning in **V.90b**; the rule is in **II**.

**N-3 — `info` answered about the resolver.** `get_info` opened with
`if let Ok(specs) = resolve_spec(…)` and the error it dropped was the answer. Fixing that closed
the 83-second wrong answer, and the bare-name half now asks the machine instead of trusting
`priority` order. **The sibling sweep is the interesting part: Q9 had been half-applied.** It binds
"every verb taking a backend name" and enumerated the four that take it as a `--backend` flag —
"checked from the code rather than from the one that was reported" — and the `backend:name` spec
form was never in that list. Nine verbs took it unchecked, and `hold nosuchbackend:foo`
*recorded* a hold against a manager that does not exist and answered `Held 1 package(s).` at
exit 0. The check is derived from `--help` now, with validated exemptions.

**N-4 — the flag-drift gate could not fail on the flag it was built for.** `tool_help::accepts_flag`
asks the installed tool and withholds a flag its help does not document, so the argv the gate
inspects has no flag in it *exactly when the table is wrong*. Measured here in both directions: a
bogus flag planted in `VERIFIES_ITSELF` left the argv-reading gate green. The fix asks the question
an argv cannot carry — does the installed tool document the flag this table names? — and that
assertion goes red on the same mutation the old one slept through. The user-visible half went with
it: the advice was `Add @unverified to the line` regardless, so a helm with no way to skip
verification answered a refusal by naming a flag it would drop.

**N-5 — a gate reporting its own absence in a voice nothing tallied.** `.dockerignore` excludes
`scripts/`, so `lifecycle-floor.txt` was in no image and the real-lifecycle ratchet was in force on
**one host class of five** — the Windows sweep, which has the least coverage, and absent from the
four distro legs and the `tools` image, which have the most. Both harnesses' `else` branch printed
a note and incremented neither PASS nor FAILC, so a run with the gate missing looked exactly like a
run that passed it. Mounted on every leg; the absent case is a counted failure; a new predicate
asserts every CI leg that mounts the harness mounts the floor.

**N-6 — the local container gate could not run on the development platform.** Four `.sh` files had
been rewritten to CRLF and dash aborts on `set -u<CR>` after an 18-minute build. `.gitattributes`
was correct and irrelevant: `eol=lf` governs what checkout writes, not what an editor writes
afterwards, and nothing checked the working tree.

**What every one of these has in common, and it is worth writing down again.** Five of the six are
a sentence that quantifies over paths and was verified against the paths the sentence names. N-1
against the two backends with a reproduction; Q9 against one of its argument's two spellings; the
drift gate against the argvs it could see; the ratchet against the one host class where its file
happened to be. `READINESS` §5.3 named this class, round 2's report named it again, and it produced
six more findings one layer in. **The habit that catches it is not "check the list" — it is asking
which gates would go red if the thing they name were happening right now**, which is why every fix
in this session ships with a mutation that proves its check can fail.

**Verified:** `cargo build --all-targets`, `cargo clippy --all-targets --all-features -D warnings`,
`cargo fmt --check`, the full suite, and `harness-logic-test.sh` at 56/56. The container matrix and
the `tools` image were **not** re-run this session — the changes to `run-in-container.sh` and
`ci.yml` are unproven against a real container run, and that is the first thing the next session
should do.

---

## Session 2026-07-29 — what the nightly jobs were saying, and the ruling that closed the register

**Two nightly jobs were red and the seven failures in them were four separate faults.** The
per-push matrix was green throughout, which is why this took a nightly to surface: `tools` and
the two native sweeps only run on a schedule or a dispatch.

**1. Four "X is on PATH" failures, and neither half was where it looked.** `github: fd`,
`go: hello`, `yarn: cowsay` (Windows runner) and `nimble: nimjson` (`tools`) all installed
correctly and all showed up in `list`.

- The **product** had no answer for two of the directories. `reachable.rs` — the E6c/W4 warning
  that names a bin directory the machine's PATH does not — had no entry for `github`/`web`/
  `appimage`, which is **LiNix's own deploy directory**, and it cleared `npm`/`yarn`/`pnpm`/`bun`
  as "their installer wires that up". CI falsified the second claim for yarn. A constant would
  have been wrong too: `yarn global bin` reads `…\scoop\apps\yarn\current\global\bin` on the
  developer's box and `/c/npm/prefix/bin` on the runner, so those four are asked now, and a tool
  that cannot answer produces no claim rather than a guess.
- **Three copies of `~/.local/bin` went with it.** `github.rs`, `web.rs` and `appimage.rs` each
  built that path from `dirs::home_dir()` and ignored `Config::bin_dir` — the field whose own
  comment says it exists so a sandbox can move it "which is what stops a test writing an
  executable into the developer's real `~/.local/bin`". A sandboxed run put its shims in the
  sandbox and its downloads in the developer's home.
- The **check** asked the host, not the product. W4's ruling is that an unreachable install
  *warns*; `on_path` fails runs where LiNix did everything it promised, and — the half that
  matters — passes runs where it said nothing at all, as long as the machine happens to be
  wired. That is why the developer's box never saw it: `~/go/bin` and `~/.local/bin` are both on
  PATH here. `assert_binary_reachable` asserts the promise; `binary_present` and a fourth
  argument to `assert_binary_gone` close the same hole in the survived-unmanage,
  gone-after-uninstall, dry-run-did-not-install and reinstalled-by-rebuild checks, which had all
  been reading PATH for "is it on the machine".

**Proven on the runner rather than argued:** all three Windows failures came back as
`PASS github: fd is not on PATH, and the install said so, naming /c/Users/runneradmin/.local/bin`
and the same for go and yarn.

**2. The three `tools` failures were one question, and the owner ruled it.** `mix` had no Hex,
`asdf` had no plugin for the tool it was handed, `opam` had no switch; each made *every* install
through that manager fail. Q10 and Q11 had been open since 2026-07-28 and asdf joined them as
Q13. **Ruled 2026-07-29: LiNix asks whether to do the setup for you, with a flag to force it.**
That is `[[prereq]]` rows — compiled in, extensible per repo through `adapters/prereq.toml` on
the II.12 ledger — offered in II.7 phase 0 right after the bootstrap offer, with `--yes` as the
flag. II.7a and V.102 carry it, and **the register is now at zero open for the first time**
(122 entries: 120 answered, 2 parked).

Every argv in those rows was measured in the `tools` image, and the measuring found two more
defects that the first one had been hiding:

- **The mix canary could never have passed.** `mix:hex` was it, and `mix archive.install hex hex`
  answers `No package with name hex (from: mix.exs) in registry` even with Hex present. An
  impossible canary and a real defect were reported as one failure. It is `phx_new@version=1.6.16`
  now — pinned, because this image's Elixir is 1.14 and the current release declares `~> 1.17`,
  which also meant adding the positional version pin `mix` never had.
- **`mix archive.uninstall` without `--force` prompts**, takes the empty answer from a closed
  stdin, exits 0 and leaves the archive installed. LiNix reported removals that never happened —
  E7's shape, one manager over.

**And the `tools` image itself was wrong.** Its Dockerfile ran `opam switch create default` with
no compiler and swallowed the failure with `|| echo "SKIP opam init"`, so the image has never
had a switch, every opam install in it failed, and the nightly read that as an opam defect. It
installs `ocaml-nox`, creates the switch with `ocaml-system`, and asserts `opam switch show`
succeeds.

**3. A bug I introduced, found the same way — by pushing and reading.** The four new PATH
helpers were defined beside `assert_binary_gone` at line 617 and called from section 5 at line
329. Shell reads top to bottom, so those calls were `command not found`: one check reported
`rc=127` and **one vanished entirely** — no PASS, no FAIL, nothing in the count. The container
harness had three of them.

That is a class, not an incident, so it has a gate: `harness-logic-test.sh` now checks that every
harness function is defined above the first place the script *calls* it. Only top-level calls
count — a name used inside another function body is resolved when that body runs — and quoted
text is not a call, which took two attempts: the first version reported `echo "[5] Real
lifecycle"` and `ok "sync commits" …` as call sites. Verified by putting `binary_present` back
where it was and watching the gate name it.

**4. The drift gate does not get the setup commands, and that is measured rather than assumed.**
Feeding them in was the obvious move — they are argv LiNix runs on a manager. But the gate
confirms a finding by running `<subcommand> --help`, and in the `tools` image `mix local.hex
--help` offers to install Hex while `asdf plugin add --help` clones asdf's entire plugin
repository. A gate that changes the machine it is auditing is worse than one that skips it, so
`mix` and `asdf` are exempt *with that measurement written down*, and the rows' argv is verified
by the nightly lifecycle instead.

**Also true and worth stating: the mutation experiment did not catch the ordering bug**, because
a `command not found` on stderr does not fail a run and the check it silenced was one the stub
run never reached. The lexical gate exists for exactly that reason.

### What the nightly said next, which is the half worth reading

**The Windows sweep went green** — 274 pass, 0 fail — and the `tools` image went from six
failures to two. mix, asdf and opam each installed for real for the first time: the offers
fired under `-y`, the switch fix landed, and `opam installed ocamlfind for real` is a line that
image had never printed. The two that remain are both **new findings from the checks added this
session**, which is what those checks are for:

- **`opam: ocamlfind is not on PATH and nothing said where it went.`** True, and LiNix's fault:
  opam installs into the current switch's `bin`, and it expects `eval $(opam env)`, which a
  declaration cannot do for you. There is no constant to write — `/root/.opam/default/bin` here,
  something else under another switch — so opam joins the ask-the-tool family with `opam var bin`.
- **`asdf: jq is gone from list (expected non-zero, got 0)`.** The removal worked; the *parser*
  was wrong. `asdf list` prints an added plugin whether or not any version of it is installed
  (`jq` followed by `  No versions installed`), and every unindented line was being read as an
  installed package. That is permanent phantom drift — `sync` removes the version and finds it
  again on the next run — and it is the E6 family in a fourth backend. Fixed with fixtures
  captured from the tool on both sides of an uninstall.

**And one regression I caused and the sweep caught.** Q11's health half demotes a manager whose
setup is missing, so `mix` stopped being `[READY]` in an image with no Hex — and the sweep builds
its lifecycle list from `[READY]`, so mix silently dropped to a plan-smoke and the run still said
PASS. The health verdict is right; the sweep's rule was too narrow. Both harnesses now also
lifecycle a backend that is degraded *only* because of a setup step LiNix offers to run, since
`lx -y install` performs that setup — leaving them out tests the offer nowhere.

**A ✅ I wrote that was not earned.** I reported this session's `cargo fmt -- --check` as clean.
It was not: the diff was in my own verification log, twenty lines below where I stopped reading,
because I had filtered that log for test results and error lines — neither of which rustfmt
prints. CI's ubuntu job read the same command's output correctly and failed. Rule 9 says a ✅ is
earned by a command; this is the narrower version of it — **a filter chosen for one tool's output
silently answers for every tool behind it.**

### The run that closed it — every job green, and mix's first real lifecycle

Run `30465017776`, dispatched against the fixes above. **Sixteen jobs, all green**, which this
branch had not managed once this round:

| sweep | result |
|---|---|
| `tools` (container) | `pass=352 fail=0 soft=9` — **25 real lifecycles**, up from 4 failures a run ago |
| Windows native | `pass=275 fail=0 soft=13`, 10 real lifecycles |
| macOS native | `pass=265 fail=0 soft=6` |
| ubuntu / fedora / alpine / arch / gentoo | green |
| argv drift ×3, mutation, build/test/clippy/fmt ×3 | green |

**`mix` ran a real install → list → remove for the first time in this project's history**, and
it did it as `mix:phx_new@version=1.6.16` on an image whose Hex the sweep itself installed
through the offer. The line above it in the log is the one worth keeping:

    needs setup, and the sweep exercises it anyway: mix

— the coverage rule catching precisely the case that health had, correctly, taken away.

`opam: ocamlfind is not on PATH, and the install said so, naming /root/.opam/default/bin` is the
other one: a check that used to fail because the product was silent now passes because it is not.

Verified here before that run: `cargo build --all-targets`, `cargo test` (35 binaries),
`cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt -- --check` (read in full
this time), `harness-logic-test.sh` 54/54, `decision-count.sh --check`.

### Two latency claims, measured against this tree rather than quoted

`READINESS` E14 and E15 are cited in the round-2 brief as the items that would move the grade
most. One of them is already gone and the brief did not know:

- **`linix info cargo:ripgrep` — 0.131s**, and correct (`Backend: cargo`, `Version: 15.2.0`).
  E14's 98-second wrong answer is fixed; round 1's work did it.
- **`linix search ripgrep` — 1m12.7s**, 103 rows. **E15 is live.** Debug build, but the time is
  network-bound across ~20 registries rather than CPU, so a release build does not explain it.
  Nothing measures it: `search` still has no latency budget, which is how a 145-second read-only
  command shipped in the first place.

## Session 2026-07-28 (later) — the push, what CI said, and Q6

**The 25 unpushed commits went to CI and CI was right to fail.** Four things had been green
locally for a fortnight and were broken everywhere else:

| what | why it was invisible |
|---|---|
| `a_windows_gopath_keeps_its_drive_letter` | asserts a Windows path rule; on Unix a colon *is* the separator, so the assertion is false there and the test had only ever run on Windows |
| `choco_msi_reboot_codes_are_benign_and_do_not_leak` | a Unix `ExitStatus` cannot hold 1605: `from_raw(code << 8)` reads back as `code & 0xff`, so the fabricated status was 69 |
| `winget_noteworthy_codes_are_benign` | same, for negative HRESULT-shaped codes |
| `find_placeholder` | clippy 1.97 on the runner flags a `question_mark` the 1.96 toolchain here does not |

Each is fixed at the cause rather than gated away: the exit-code tests now assert the **policy**,
which is plain data and true on every platform, and prove the `ensure_status` wiring with a code
the OS can carry. A third test states the constraint outright — `a_fabricated_status_round_trips_only_what_this_os_can_hold` — so the trap cannot be re-entered
silently. The GOPATH test is `#[cfg(windows)]` with its Unix half already covered by
`the_first_entry_of_a_gopath_list_owns_bin`, which runs on both.

Verified on Linux this time, not assumed: `cargo test --lib` under WSL, **1263 passed, 0
failed**. The clippy lint is the one thing still taken on faith — the runner is a newer toolchain
than anything available here.

**Q6 — a config file may take a built-in's name, and only by saying so.** The onboarder skipped
any name already in use, so a definition called `apt` was silently ignored. That kept a pulled
config from hijacking `apt` by guessing a popular name, and that half stays. What it also did was
leave the person watching a manager change its CLI — helm v4's signatures, pixi's renamed
`global upgrade-all`, nimble's `--` — able to see the fix and unable to apply it before a
release. `overrides = true` is the way, and the key is the design: without it the two behaviours
are indistinguishable from outside, and a name alone would be a supply-chain attack with no
attack in it.

Two deliberate acts, never one — the sentence in the definition, and II.12's approval of the
file. Proved end to end: unapproved, the file registers nothing and says so; approved, `scoop`
runs `my-scoop`, and `check health` moves that backend to **critical** because `my-scoop` is not
installed. Health needed no special case — it probes the definition that won, which is the true
answer about the machine.

Scoped to backends on purpose. Snapshot providers, init systems and secret stores still never
shadow a built-in: same argument, different blast radius, and that is a ruling nobody has made.

## Session 2026-07-28 — Q5: `@unverified` reaches past the backends that download

**E11 was not an argv defect, and the review's proposed fix would have been a security
regression.** helm v4 verifies a plugin's signature before installing it, and a git URL — which
has no `.prov` beside it — is refused outright, so no declaration could install a helm plugin at
all. Adding `--verify=false` to helm's install command would have turned verification off for
every plugin every user installs, which is the global switch `@unverified` exists as a per-line
flag to prevent.

**Ruled by the owner (Q5): widen the flag.** `@unverified` now means *nothing vouched for these
bytes* rather than *LiNix skipped its checksum*, and it is legal wherever something otherwise
would have checked. On `helm:` it becomes `--verify=false`. Recorded in `decisions.md` (Q5),
`target-state.md` II.2 and `why.md` V.94, in the same commit as the code.

Four properties, each a test written and watched failing first:

- **`@allow_http` did not follow.** The two flags were checked in one loop, which is what made
  them look like a pair; they are separate branches now. helm's plain-HTTP switch addresses OCI
  registries LiNix does not reach, so `@allow_http` on `helm:` is still refused, and its hint
  must not name helm.
- **Per line, still.** An install batch whose specs disagree becomes two commands.
- **The advice is actionable.** helm names `--verify=false`, an argv no declaration can write;
  LiNix appends the flag that can go on the line.
- **`does not support verification` is Permanent**, not transient — a source that carries no
  signature never grows one. helm had no `ExitPolicy` at all before this.

**Measured, not assumed** (helm v4.2.3, 2026-07-28): the refusal is in
`tests/fixtures/helm/plugin-install-unverifiable-source.txt`. Both harnesses' helm canary moved
from `databus23/helm-diff` to `jkroepke/helm-secrets`, because helm-diff needs `pwsh` in a
post-install hook and this host has none — it installs the plugin and *then* exits 1, so the
check would have reported someone else's weather. helm-secrets fails without the flag and
succeeds with it, which is the property the canary is for.

One thing the flag does not get: the argv-drift gate scores subcommands, not options, so
`--verify=false` is verified by the measurement above and by nothing automated.

## Session 2026-07-27 (last) — working `docs/BUILDER.md`, and four owner rulings

`docs/READINESS-2026-07-27.md` indexed 36 defects (`E1`-`E34`) and `docs/BUILDER.md` turned them
into work orders. This session worked them in tier order. **23 commits, unpushed.** Verified on
Windows only: `cargo build --all-targets`, `cargo test` (23 binaries, 0 failures), `cargo clippy
--all-targets --all-features -- -D warnings`, `cargo fmt -- --check`, `harness-logic-test.sh`
35/35. Linux and macOS have compiled none of it.

**Four owner rulings, taken and recorded as `Q1`-`Q4`** (a new register namespace, chosen after
checking every other prefix for collisions — `PRODUCTION-READINESS-REVIEW.md` had reused `U1`-`U3`,
which are real register IDs, and that document's labels are now `PR1`-`PR3`):

- **Q1 — a failed install withdraws its line** when the failure can never succeed
  (`Unresolvable`, or a `CommandFailed` the backend's policy calls `Permanent`). Read off
  `CommandFailed` and never off `retryability()`, which also calls a refusal and a cancelled
  prompt permanent. A line kept on purpose now names its file and `linix unmanage`. II.8, V.90.
- **Q2 — `Absent` is its own health state.** A manager nobody installed is not critical; one
  named in `priority` still is. The rollup and the detail view read one tally. II.8, V.91.
- **Q3 — a usage error exits 1, every refusal exits 3.** The published four-code table stays
  four codes. II.8, V.92.
- **Q4 — nothing is labelled "experimental".** The owner rejected the review's central
  recommendation, and the reason is the rule: *this codebase does things; it does not cover for
  not doing them.* A label turns an unfinished job into a permanent disclaimer. So the coverage
  is the work, and **missing coverage is a release blocker**. `init` keeps scaffolding every
  manager it finds. V.93.

**What the measurements corrected in the review**, each verified rather than accepted:

- **E6 was not a parser bug.** `bin_dir` split `go env GOPATH` on `:` as well as `;`, so a
  Windows drive letter was decapitated and `list` was silently empty. A second, real bug sat
  behind it: identity came from `mod` rather than `path`, so a program in a module subdirectory
  could never equal its own declaration.
- **E6b was LiNix, twice.** `nimble` was listed as honouring `--`; its own help says `--` passes
  args to the Nim compiler, so **every nimble install of a binary package failed**. And nimble
  exits 0 on failure with no `ExitPolicy`, so LiNix called a build that produced nothing a
  success. The harness's two FAILs were right; its PASS was the lie.
- **E11 and E12 are not LiNix defects, and the review's suggested fixes would have made things
  worse.** helm v4 verifies plugin signatures by default — adding `--verify=false` would have
  disabled signature verification for every helm plugin install, against SEC2. luarocks fails
  because this host has Lua 5.5 and luarocks.org publishes no 5.5 manifest; pinning a version
  would install rocks for an interpreter the user does not have.
- **E4 was Windows-only.** `release-check.sh` already rated `fmt` hard.

**Three checks were built, watched passing, and found to be testing nothing.** Each is recorded
because the pattern is the whole subject: the argv-drift gate passed with `pixi global
upgrade-all` reintroduced (a bad `perl` edit had landed on `composer`, so the defect was never
back); the `check health` list-probe reported scoop and winget as unable to answer `list` when
the 20s cap and eight-way concurrency were mine and they take 1.2s alone; and the latency budget
claimed a 5s ceiling would catch E14, then passed at 4.34s with the defect restored. The last
one is now honest about being only a ceiling, and the property is pinned by a test that counts
commands instead of timing them.

**The mutation experiment, run for real.** A `linix` stubbed to exit 0, against the full Windows
harness: **88 checks still passed, 30 caught it.** Most survivors are exit-code checks followed
by an assertion that does look at the effect — that pair is correct. The ones that mattered were
checks whose name claimed an effect with nothing after them: `profile create scaffolds one` and
`module create scaffolds one` both passed against a binary that created no file.
`scripts/harness-mutation-test.sh` makes it repeatable with the survivor count as a ratchet.

**Still open, and named rather than implied:** the container matrix has not run against these
changes (Docker is in WSL; `run-in-container.sh` was edited for E2 and E5 and those edits are
unexercised); the 23 commits are unpushed so CI has seen none of it; `search` (2m41s) and the
`check` rollup (10.4s) have no budget; `opam`, `spack` and `emerge` still have no captured
parser fixture; and whether `@unverified` should extend to helm is a Part II question that needs
a ruling. *(That last one was ruled the next day — Q5, at the top of this file.)*

## Session 2026-07-27 (later) — a production-readiness review, and the blind spot every gate shared

A full read of the tree (202 files, 72,824 lines, 1,324 tests) against the running binary on
Linux produced `PRODUCTION-READINESS-REVIEW.md`. Its verdict was **not production ready**, on one
finding that mattered more than the rest: **nothing in CI, in the container harnesses, or in any
test had ever run LiNix attached to a terminal**, and every blocker lived in that gap. A green
suite was not evidence against the findings; it was the reason they survived. All of them are
now fixed, and each was watched failing first.

- **S42/S43 — the terminal.** `RawExecutor::execute` decided all three of the child's handles
  from whether *LiNix's own* stdin was a tty. Attached, the child inherited stdout, so
  `output.stdout` came back empty and all 79 `run_output` call sites parsed an empty string:
  `linix list -b apt` reported **609 packages piped and 1 under a terminal**. And with stdout
  inherited, `systemctl` started a pager, so read-only `linix status` hung on a keypress and
  printed 80/640/83 lines across three identical runs. Capture is now a property of the call
  (`ChildStdin::Closed` for reads, `Interactive` for mutations); pagers are suppressed on the one
  env map every spawn inherits. Ruled **U40**, rule II.12c, reason V.84.
- **S45 — the rollback, and the mass-uninstall chain.** A `@version=` change schedules an
  `Install` node for a package that is already there, and rollback compensated every `Install`
  with `remove()` — so a failed upgrade *uninstalled* the package. `needs_change` also read "I
  could not ask" as "not installed", which under S42 was **every package**, so a single failure
  rolled back across the whole managed set. And `transaction.rs` contained **zero** references to
  the guard: rollback's removals never passed the plan-time gate, while II.10 said every removal
  path calls it. Ruled **U41**, rule in II.10, reason V.85.
- **S44 — repositories.** `{url}` was never substituted on the removal path, so apk's `sed`
  searched for the literal text, matched nothing and **exited 0** — a repository reported removed
  that was still there. An unsubstituted placeholder is now a hard error. The fix uncovered a
  second fault: apt's and apk's repo rows named a *program* in the argument position, so the real
  commands were `apt add-apt-repository …` and `apk sh -c …`. **`repo add`/`remove`/`list` had
  never worked on either manager.**
- **S46/S47, and the ones that were already right.** Two `.expect()`ed spawns on the dry-run path
  (`/bin/false`, `cmd /C exit 1`) replaced with `ExitStatusExt::from_raw`; `TaskResult.attempt`
  renamed to `retries`, which is what it held. **The review's H4 is half wrong and the entry says
  so**: there is no hardcoded `v6.0.0` in `src/` — the banner it saw came from the stale July
  binary on disk. What is real is `Cargo.toml`'s `version = "0.1.0"`, and that number is the
  owner's.

**The gate that closes the class is `tests/pty_tests.rs`**, which runs the built binary under
`script -qec` against a stub manager on `PATH` and asserts that what LiNix printed is what LiNix
parsed. It is a named step in CI's fast half. Verified failing against the previous behaviour in
WSL before it was made to pass.

**Verified, not asserted:** `cargo build --all-targets` → `cargo test` → `cargo clippy
--all-targets --all-features -- -D warnings` clean on **both Windows (1,340 tests) and Linux
(1,339 tests, run in WSL)**. Two Linux-only faults were caught by that second run and would
otherwise have gone to CI red: an import that is Windows-only once the fabricated `ExitStatus`
stopped spawning, and a scheduler test pinning the systemctl argv that `--no-pager` changed.

**Left open on purpose.** The review's U1 (45 top-level commands, overlapping clusters) and U2
(INFO logging on by default) remove or change behaviour a user would notice, so they are the
owner's to rule; the SOLID items in §6 are real and are explicitly not shipping blockers.
Uniform retry semantics (`transaction.rs` retries a nonexistent package as patiently as a held
dpkg lock) needs structured errors to fix properly and is not attempted here.

## Session 2026-07-27 — the push went green, and the nightly half said why that was not enough

> **Closed green.** Run `30237464029` (`945fba0`): **every job passed, including
> `macos-native`, which had never once completed.** Its final sweep is `pass=263 fail=0
> soft=6` — 46 registered backends all covered (8 real lifecycles, 38 plan-smokes), 56 of 61
> subcommands executed for real and the other 5 exempt by name. The `tools` image and Gentoo
> are green in the same run. **This is the first time every gate this repo has, on every
> platform it claims, has been green at once.**

**CI run `30228257690` is the first green `main` this repo has had**: all three build/test/clippy
columns, plus the four fast container integrations. S33 and S34 are confirmed fixed on the
platforms that reported them, which is the negative S33's own entry said only CI could prove.

**Then the broad half ran, and found two more.** The jobs that carry the real coverage — the
`tools` image's 18 managers, Portage, and the macOS native sweep — are `schedule`/`workflow_dispatch`
only, so the green push had not touched them. Dispatched against the same commit (`30229794698`):
gentoo passed, `tools` failed on one check out of 333, and `macos-native` failed on all of them.

- **S36 — `tools`.** `go: hello is still on PATH after removal`, with `go: hello is gone from
  list` passing on the line above it. Not a go bug: cabal's canary is also `hello`, cabal has no
  uninstall verb, and its copy stays for the whole run on purpose. **This is S32 verbatim** — the
  same vacuous PATH check, with different backends in it — and it came back because S32 was fixed
  by renaming one canary instead of by fixing the predicate.
- **S37 — `macos-native`, on its first ever execution.** `TO="timeout 900"`, and macOS has no
  `timeout`; every command exited 127. **The run still printed passes**, because `nok` read 127 as
  the refusal it wanted. A whole platform's sweep, reporting a result for checks that never ran.

**The lesson both share is one the repo already wrote down and did not apply to itself: an
assertion that cannot fail is worse than a missing one**, because it is counted. Both are now
fixed at the mechanism — the removal check compares against the pre-install resolution rather
than asking PATH, and 127/126/124 are hard failures in both harnesses rather than refusals.

**The guard is the part that matters.** Both faults lived in two-line shell predicates, in files
that only run nightly, so both were found a day late by whoever read the log.
`scripts/harness-logic-test.sh` lifts those predicates out of the harnesses — lifted, not copied,
so it cannot drift — and tests them in a second on **every push**. Against the previous revision
it fails exactly four of its 13 cases, which are exactly the two findings.

**Fixing S37 made the macOS column legible, and it had more to say. Two further findings, one
of them still open.**

- **S38.** The host harness was driving a command surface that no longer exists: `doctor`,
  `status`, `absent`, `unmanaged`, `conflicts` and `audit` were folded into `check <section>`
  and only the container harness was updated. Six checks failed — but the real damage is that
  `READY_LIST` and `ALL_BACKENDS` are both built from `lx doctor`, so **two whole sections
  iterated over an empty list and reported nothing wrong.** `READY backends:` printed blank and
  the run carried on. A stale verb reads as no coverage, not as reduced coverage. The same file
  had never had `answers()`, so the aggregate `check` — which exits 2 for *findings* under
  U21's table — was asserted with a hard `ok`.
- **S40.** Section 11b of the *Windows/macOS* harness stages a manager that cannot answer by
  shadowing cargo with a `cargo.bat` — which Windows resolves through PATHEXT and macOS does
  not resolve at all. On macOS it shadowed nothing, so the two assertions that need a silent
  manager failed for the one reason their names cannot say: there wasn't one. Both branches
  are written now. Three faults this week were the same thing — a file whose header promises
  it mirrors the container harness section for section, and had stopped.
- **S41 — the one real product bug the whole sweep found.** With the harness finally honest,
  macOS came down to a single failure, and it was ours: **`restore --force` cannot overwrite a
  read-only file.** `bundle` copies the whole config root, that root is a git repo, git writes
  its objects at 0444, and `fs::copy` carries the bits across — so the first restore lays down
  a read-only tree and the second, which is the only thing `--force` exists for, gets EACCES.
  It named none of the several hundred paths it had copied. **Nothing caught it because the
  container harness runs as root**, and root bypasses the permission check; it reproduces on
  Windows too, so it was never a macOS bug, only a non-root one. Fixed at one function —
  `copy_over` in `utils/file.rs`, whose neighbour already carried the lesson in a comment — and
  applied to its family: four copies in `bundle.rs`, plus `github.rs` and `web.rs`, which write
  over a previous copy on reinstall.
- **S39, closed — and it was never the product.** `rollback HEAD`, `activate` and
  `restore --force` each exited 1 on macOS. The cause is one line of the harness: section 6
  installs an unresolvable name on purpose and then deletes the manifest line with
  `sed -i '/…/d' file 2>/dev/null || true`. **BSD sed reads the argument after `-i` as a backup
  suffix**, so on macOS the deletion never happened and was silenced twice — by the redirect
  and by the `|| true`. Every later converge then tried to install a package brew cannot
  resolve, and LiNix **correctly** refused and said so. The comment directly above that line
  predicted it in those words. Fixed in both harnesses with the portable read-filter-move the
  file already used elsewhere, and the removal is now asserted rather than assumed.

  **Worth recording because I got it wrong twice, and both times because the harness would not
  show its output.** First reading: three panics. There is no `panicked at` anywhere in the
  run. Second reading: three product bugs in a converge path. The product was right every
  time. A harness that prints six stack frames instead of one error line does not merely fail
  to help — it produces confident wrong diagnoses, twice, before anyone reaches the real one.

**One regression, mine, worth writing down because it is the same shape as the bug it came
from.** The `never_ran` guard made 127 a hard failure in `nok`; `on_path` answers "not found"
through `command -v`, which returns 1 under bash and **127 under dash and busybox ash** — so
ubuntu, alpine and tools each failed two checks on names that were correctly absent. A predicate
that says "no" with the code that means "I could not run" is the same ambiguity `never_ran`
exists to remove, so `on_path` is normalised to 0/1 at the source rather than special-cased at
each caller. The guard covers it, and only bites where `/bin/sh` is dash — which is the runner
it runs on.

## Session 2026-07-26 (later) — the assessment's list, worked: CI green on three platforms for the first time

**The previous entry measured the gap. This one closes the top of it.** Seven commits, in the
order the owner asked for: S33, helm, rustfmt, `main.rs`, the comment sweep, then the coverage
that findings 2 and 3 named.

**The headline is not any one fix. It is that the suite had never been green anywhere but this
Windows box, and nobody could see it because the first red job hid the second.** CI run
`30219497156` — the first run to ever see the 112 pushed commits — failed on all three
platforms, and on *two different bugs*:

- **Windows: S33**, as predicted. `tests/feature_logic_tests.rs:90` commits to a temp repo and
  unwraps, so it passes only where a global git identity exists.
- **Linux and macOS: S34**, which nobody had ever seen, because S33 was the one being watched.
  `app::eval::tests::sources_are_repo_relative_and_forward_slashed` asserted Windows path
  semantics (`Path::strip_prefix` on `C:\repo\…`) on every platform. A backslash is a separator
  on Windows and a filename character everywhere else.

Both are the same *class* — **a test whose truth is a property of the machine that wrote it** —
and both are now fixed at the mechanism rather than at the instance. S33's fix is a `Once` in
`TestKernel` and in `git.rs`'s test gate that sets `GIT_AUTHOR_*`/`GIT_COMMITTER_*` and points
`GIT_CONFIG_GLOBAL`/`GIT_CONFIG_SYSTEM` at absent paths: the tests now read **no** host git
config at all, so neither a missing identity nor a host that signs every commit can decide the
result. `identify()` and its three call sites are deleted. Set once per binary, not once per
test, because a step each test has to remember is the step the next test forgets — which is
literally how S33 was written.

**Proven, not argued.** The whole suite was run in the `linix-it-ubuntu` container, which has no
global git identity (`git config --global user.email` is empty there): **1,307 tests, 0 failures
on Linux**, including the `tests/` binaries that `cargo test --lib` never reaches. That is the
first green Linux run this repo has.

**What else landed:**

- **U39 — helm.** Finding 6, ruled, built, and **proven against real helm v3.21.2 in the
  `tools` container**: `helm:diff@url=https://github.com/databus23/helm-diff` installs,
  `helm plugin list` reports `diff 3.15.10`, `linix list --backend helm` agrees, and
  `uninstall helm:diff` removes it. That is the same lifecycle that failed when the bug was
  found. `capability::INSTALLS_FROM_SOURCE` is one table read by both the grammar and the
  backend row, and the whole ruling — `decisions.md` U39, `target-state.md` II.2, `why.md`
  V.83 — shipped with it.
  **The first version of the fix was broken and only the live run said so.** The unit tests
  built a `PackageSpec` in code, so nothing asked the grammar whether `@url` was a legal
  option key — II.2's table is closed and it was not, so every real declaration came back as
  a misspelling while 1,310 tests passed. This is finding 2's argument in miniature, made by
  finding 6: a mock cannot tell you the grammar refuses your line.
- **rustfmt.** 130 files, and the gate is now hard in both places (`continue-on-error` off in
  CI, INFO → HARD in `release-check.sh`). A fmt check nothing fails is a check that measures
  nothing, which is how the tree drifted 609 hunks while both gates reported on it every run.
- **`main.rs`: 6,774 → 902 lines.** Nine modules under `src/verbs/`, one per verb family, each
  test module moved with the code it tests. Pure motion — 1,310 tests before and after,
  `--help` still lists 62 commands.
- **F3's comment sweep, by defined class rather than by claim.** Marketing was already clear.
  Citation-as-explanation went 13 → 4 (the 4 that remain close a sentence that states the
  constraint). Restatement: 64 flagged by identifier-overlap, 10 real and deleted. **What is
  not claimed is written down as not claimed:** a comment that narrates its block without
  echoing an identifier is a judgement over 9,593 lines and one pass cannot finish it.
- **Finding 3, answered where it can be without a Mac: compile every platform's backend rows
  on every platform.** The OS-native registrars were `#[cfg(target_os = …)]`, so `mas`'s and
  `macports`' argv only ever *existed* on a Mac and apt's only on Linux. They are compiled
  everywhere now and still registered only on their own OS (`create_default_registry` keeps
  the gate, at runtime), and a new test asserts the **program name and the verbs** for 26
  generic backends on whatever host runs the suite. **It found a live bug on its first run —
  S35:** `macports` had `binary: None`, so LiNix probed for a `macports` executable, which no
  Mac has. The program is `port`. The backend could never have come up READY on a real Mac,
  `doctor` would have called it absent, and a `macports:` line would have been refused — and
  Linux and Windows CI could not have seen any of it. Found on Windows.
- **Findings 2 and 3, in the only place they can be closed — the gates.** `fedora`/`dnf` joins
  the per-push container matrix, which had been running only ubuntu/alpine/arch while the local
  runner swept five. `tools` and `gentoo` move from dispatch-only to nightly, turning the widest
  live coverage this repo has (18 real lifecycles) from *existing* into *consulted*. And a
  `macos-native` job runs the same brew sweep `release-check.sh`'s Darwin branch does — nightly,
  not per-push, because a job whose first execution gates other people's commits is a job that
  gets disabled rather than fixed.

**What this does not do.** It does not close findings 4 (the hardware-deferred destructive
effectors) or 5 (never released), and it does not make macOS *proven* — it makes macOS
*scheduled*, which is the most that can be done without hardware. The numbered list below is
updated in place to say which items are now closed.

## Assessment 2026-07-26 — production readiness, measured; and the tree pushed after 112 commits

**No code changed this session. It read the tree, pushed it, and ran the one gate that had never
seen it.** The question asked was how close this is to production, and the honest answer has two
halves that point in opposite directions: **the safety architecture and the test apparatus are
past most shipping tools; the validation surface and the release apparatus are pre-release.**

**The finding that reframes every green in this file: `origin/main` was 112 commits behind
`HEAD`, last pushed 2026-07-23.** Everything in Tiers 1–4 — D5, the whole U-series, the provider
mechanism, `repl`, user verbs, module parameters — had been built, unit-tested and committed on
one Windows box and **never once compiled on Linux or macOS, and never run through the container
matrix.** CI is not a formality here: it is the only thing in this project that builds the tree
on a platform other than the developer's. It was pushed this session (`d901a77..2db5aca`).

**And the two CI runs before it had both failed** — 2026-07-23 and 2026-07-23 — while this
document recorded green for the same tree. Both halves of that are true at once, and the
explanation is **S33**: `tests/feature_logic_tests.rs:90` commits to a temp git repo and unwraps,
so it passes on any host with a global git identity and fails on any host without one. The green
was a property of the developer's `git config`. `cargo test --lib`, which `SPEC.md` cited, does
not run that binary at all; `--all-targets`, which this file cited, does, and passes here.
**Neither report was dishonest and neither was a measurement of the code.**

**What is genuinely strong, measured not asserted:**

- **Zero `todo!()` / `unimplemented!()` in 74,462 lines.** One `TODO`-shaped grep hit and it is a
  `/gnu/store/xxx` path in a parser fixture. No `#[allow(dead_code)]` anywhere.
- **39 `.unwrap()`s outside test blocks**, 24 of them in two files that are tests by content
  (VI.3 re-measured). Production maximum 5 in one file.
- **1,231 unit tests in `src/`, 82 integration tests, and a container harness whose section 17
  is a coverage audit that hard-fails on any registered backend or any `--help` subcommand that
  no real lifecycle and no plan-smoke touched.** A gate that can notice what is *missing* is rare;
  this one exists and is green.
- **63 subcommands in `--help`, 58 actually executed in-container, 5 exempt with a named reason
  each.** Nothing is shipped as `--help` text over a stub.

**What is not ready, in the order it should be fixed:**

1. ~~**S33 — CI red.** One test, one fixture, and the fix is the `identify()` helper `git.rs`
   already has.~~ **CLOSED 2026-07-26 — and it was two bugs, not one.** S33 was red on Windows;
   Linux and macOS were red on **S34**, a second host-dependent fixture that nobody had seen
   because the first red job hid it. Both are fixed at the mechanism, and the full suite now
   runs green in a Linux container that has no global git identity at all (1,307 tests).
2. **Breadth of live validation: 52 registered backends, and exactly 22 have ever been run for
   real.** Counted, not estimated: the `tools` image's 18 (`apt bun cargo composer conda dotnet
   emacs gem github go krew luarocks mise npm pipx pixi pub uv`), plus `dnf`, `pacman` and `apk`
   from the distro images, plus `scoop` from the native Windows sweep — `winget` and `choco` are
   deliberately plan-smoked there because they install machine-wide on a real machine. Per image
   it is 7 real lifecycles (ubuntu / fedora / arch / alpine) and **45 plan-smoked**; only the
   `tools` image reaches 18. **Partly closed 2026-07-26:** `tools` and `gentoo` moved from
   manual dispatch to nightly, so the 18 are now *consulted* rather than merely *available*, and
   `fedora`/`dnf` joined the per-push matrix, which had been running three images while the local
   runner swept five. The count of backends that have ever run for real is unchanged; what
   changed is that the widest run of them happens without anyone remembering to press a button.
   **A second, weaker but real widening:** every OS-native backend's argv is now compiled and
   asserted on every host, not only on the platform that can run it — 26 backends, program name
   and verbs. That is not a live run and does not move the count of 22; it is the difference
   between a typo being caught by whoever has the hardware and being caught by anyone. It found
   S35 the first time it ran.
   The remainder are proven against mocks, **"which is where format drift is invisible by
   construction"** — this file's own words, written before S22, S23 and S31 each proved it by
   being found only when a real manager was run. **This is the single largest gap between "the
   suite is green" and "a user's machine is safe."**
3. **macOS builds and unit-tests in CI and has never been *run*. Scheduled 2026-07-26, not yet
   proven.** A `macos-native` CI job now runs the same brew sweep `release-check.sh`'s Darwin
   branch does, nightly — the first time that branch will ever have executed. It is deliberately
   not on the per-push gate: it has never run, and a job whose first execution gates other
   people's commits is a job that gets disabled rather than fixed. **Still unproven until it goes
   green:** mas, macports, the create-only APFS provider, and launchd scheduling — though
   macports is now at least *possible*, which it was not: S35 had it invoking a program no Mac
   ships. There is no macOS hardware in this project, so a runner is the whole of what is
   available. The macOS column has already earned its keep twice — S34 was caught by its unit
   tests, and S35 by the test written because it had never run anything else.
4. **The hardware-deferred list is exactly the high-blast-radius set.** btrfs / zfs / lvm
   *restore*, D5's live `dpkg -i` / `rpm -U` handoff, `U30` storage-object removal (which
   destroys filesystems), and the `U38` secret providers. Each is argv-tested and pure-logic
   tested, and none has ever run. It is a coherent risk profile — the mechanism is proven, the
   effector is not — but it means **snapshots cannot yet be described to a user as the safety net
   under a bad sync**, because no one has watched one come back.
5. **Never released.** `version = "0.1.0"`, no tags, `CHANGELOG` still `[Unreleased]`, and
   `scripts/install.sh` builds from git `HEAD` with `cargo install --git`, so it requires a Rust
   toolchain and pins nothing. The tag-triggered release job in `ci.yml` has never fired. There
   is no artifact a user could install, and nothing to roll back *to*.
6. ~~**One open product bug, recorded in the harness and nowhere else:** `helm plugin install`
   takes a URL and `helm plugin uninstall` takes a name, so **a helm plugin LiNix installed,
   LiNix cannot remove.**~~ **CLOSED 2026-07-26 as U39** — the name is the identity, the URL is
   an option, and both harnesses now run helm's real lifecycle instead of naming the bug and
   skipping it.
7. **Hygiene, none of it blocking. Three of four closed 2026-07-26:** the tree ~~is not
   `rustfmt`-clean~~ **is fmt-clean and the gate is hard in both places**; ~~`main.rs` is 6,676
   lines~~ **`main.rs` is 902 lines and the verbs live in `src/verbs/`**; ~~F3's comment sweep is
   still openly not done~~ **F3's greppable classes are swept and its residual is stated rather
   than claimed**. **Still open:** seven scratch artefacts tracked in git at the repo root
   (`prompt.txt`, `blocks.json`, `docker_log_7_10.txt`, `docker_log_7_23.txt`, `scratch_u9.sh`,
   `Cargo.txt.txt`, `todo.md`).

**The verdict, in the terms the question was asked.** Judged as *architecture*, this is closer to
production than most 1.0s: the removal guard is on every path, there is one parser, one
transaction engine, a WAL that heals, a dry-run that cannot mutate, and an exit-code table.
Judged as a *product*, it is a **beta on Linux for apt / dnf / pacman / apk plus the dozen
language managers the `tools` image really exercises, and an alpha everywhere else** — because
the other thirty backends, the macOS platform, and every destructive filesystem operation have
been proven by construction and not by use. **The gap is not build quality. It is that this
codebase's last four real bugs were each found by running a real manager, and thirty backends
have never been run.**

## Build session 2026-07-26 — the last register items built: D5, U27 built-ins-as-rows, P0 re-measure

**Everything the owner cleared the day before is now built, unit-tested, and committed. The
register is at zero unbuilt items.** `cargo build --all-targets` → `cargo test --all-targets`
(1205 lib + every integration binary, 0 failed) → `cargo clippy --all-targets` all green on the
Windows host.

1. **D5 — `github:`/`web:` install a `.deb`/`.rpm` via the system manager.** `97d2163`. One shared
   `backends/artifact/system_pkg.rs` builds the handoff argv (`dpkg -i` / `rpm -U --replacepkgs`,
   `dpkg -r` / `rpm -e`, and the `dpkg-deb`/`rpm -qp` name read); github and web both branch a
   handoff format to it and record `installed_by` + `system_package` (github in the lock, web in its
   state). `installable_here` gates the handoff on the manager existing, so a machine without it
   falls through to download-only. Dedup/deference ride the existing per-backend ownership:
   `Queryable::owned_system_packages` (default none, overridden by github/web) feeds
   `installed_but_unmanaged` and `adopt`'s discovery, which subtract those names — `check` does not
   double-count and `purge-unmanaged` defers. `rpm -U` not the ruling's literal `rpm -i`, because
   `-i` refuses an already-installed package and the installer must own the upgrade; the builder
   records why. Only the live `dpkg`/`rpm` round-trip is deferred to a real apt/rpm box.
2. **U27 built-ins-as-rows — the hardcoded `Vec` is gone.** `7f1061d`. btrfs, timeshift, apfs,
   windows-restore and zfs are rows in `src/core/snapshot_builtins.toml`, compiled in via
   `include_str!` and read through the same `ConfigSnapshotProvider` loader a user row goes through.
   The built-in file is **not** hook-ledger-gated (a first-party compiled-in asset; gating it would
   leave a fresh machine with no safety net until `linix lock` ran); the user file still is and
   registers last; zfs ships last among built-ins so btrfs wins on a machine with both.
   `SnapshotProviderDef` gained `id_template` (LiNix-named: btrfs/zfs), `create_id_pattern`
   (tool-named, read from create output: timeshift/apfs), `detect_path` (btrfs subvolume mounted),
   `list_needs_root` (timeshift), and `powershell`. **Windows is the one row beyond plain argv:**
   `powershell = true`, `{id}` substituted only after `windows_sequence_number` parses it as a `u32`
   (`fill_ps`), so a crafted list id never reaches the elevated shell — SEC5 holds by construction
   (V.82). lvm is the exemplar *user* row, not shipped built-in. Live restore stays hardware-deferred.
3. **Phase 0 comment re-measure.** `src/` carries **9,572 comment-block lines** (was 8,896 est.,
   884 measured 2026-07-16). The marketing/self-praise subset is confirmed swept (the sales
   vocabulary greps empty; the two `magic` hits are pejorative). The narration subset stays a
   per-comment judgement call for the comment-audit passes.
4. **Harness drift fixed (K2).** Both integration harnesses (`run-in-container.sh` and its Windows
   twin `integration-windows.sh`) still asserted `nok "bare rebuild is refused"`, which K2 overturned
   — a bare rebuild now warns and rebuilds `--all`. Replaced with a `--dry-run` assertion that it is
   accepted and warns "EVERY declared package". `run.sh` now auto-smokes gentoo per-distro so one
   `DISTROS="ubuntu fedora arch alpine tools gentoo"` run is correct.

**A rotted "green" caught by running it.** Running the container matrix this session surfaced a
build that had been broken on Linux **and** macOS for a while: the U26 BSD work added
`remove_binary` to `ManagerConfig` and a copy-paste put `remove_binary: None` into the
`ManualListing::Command` / `OrphanDryRun` literals too — inside the `#[cfg(target_os = "linux")]`
and `macos` register functions, which the Windows host never compiles. So `cargo build --release`
failed in every Linux container with E0559/E0560 while the host stayed green, and the "matrix
green, run for real" claim was true only of a build that no longer compiled. Fixed (five stray
fields removed, `fix(registry)`), which is exactly the "green is a belief, not a command" lesson
this document keeps relearning: the matrix was never re-run after U26 landed.

**What is still owed is validation, not construction:** D5's live `dpkg`/`rpm` install and the
Linux snapshot providers' live restore, both on real filesystems. The container matrix
(ubuntu/fedora/arch/alpine/tools + gentoo smoke) was re-run this session via `wsl -- docker`.

## Owner decision session 2026-07-26 — clearing the last two items to build before real-machine testing

**No code this session — six rulings, each written into the spec so the next build session works
from the docs, not a chat log.** The owner's goal: build everything now and validate on the real
machine afterwards, so nothing is left "parked on hardware" that could instead be built blind.

1. **D5 — build now, test later.** The `github:`/`web:`-installs-a-file capability (a `.deb` to
   `dpkg -i`, an `.rpm` to `rpm -i`), the lock-records-installer wiring, `check`'s dedup and
   `purge-unmanaged`'s deference are built and unit-tested now; only the live `dpkg`/`rpm` round-trip
   waits for a real apt/rpm box. No longer held back whole. (`decisions.md` D5, `plan.md` Tier 1.)
2. **U27 built-ins-become-rows — Option A, build now.** btrfs/zfs/timeshift/lvm become argv rows in
   `adapters/snapshot.toml`, replacing the hardcoded `Vec` at `core/snapshot.rs:528`; live restore is
   the Linux-box job. **Windows System Restore becomes a row too, via typed-placeholder substitution**
   — id substituted only as a validated `u32`, label only as the `SnapshotLabel` enum, never raw
   interpolation — so SEC5's property holds by construction and the row is testable on this Windows
   host. This was the session's one real decision; the alternative was a permanent hand-written
   exemption, which the owner declined so the K17/U1 "every built-in goes through the tested door"
   invariant stays whole. Reasoned in **V.82**; rules in `target-state.md`; `decisions.md` U27;
   `plan.md` Tier 3 item 6.
3. **F4 — closed, leave as-is.** No wiring of `--help` to the registry; `doctor` already carries the
   live count. (`bugs.md` F4.)
4. **K18 — stays parked.** No dead atomic-swap key; it lands with the first backend that exposes
   atomic swap. (`decisions.md` K18.)
5. **Phase 0 comment sweep — part of this build.** Re-measure the comment lines against today's tree
   and sweep the P6-violating (narrating / self-congratulating / spec-citing) ones; report the new
   figure. No longer a standing "unmeasured" caveat. (`plan.md` Phase 0.)
6. **Doc drift fixed / scheduled.** `target-state.md`'s "NOT YET BUILT (checked 2026-07-26)" note on
   U27/U28/U29 was stale (they built 2026-07-27) and is corrected now; the build that removes the
   hardcoded vec rewrites what remains of it. (`target-state.md`; `plan.md` Tier 3 item 6.)

**After this session the register is zero-open and everything is cleared to build before the real
machine.** What the real machine still owes is validation only, not construction: D5's live install,
and the Linux snapshot providers' live restore.

## Session 2026-07-27 — the whole ruled backlog, built and green

**Every ruled U-item that was outstanding is now built — U27, U28, U29, U30, U32, U33, U34, U35,
U36, U38 — plus one latent-bug fix, each committed on its own; the full verify chain green at the
end: `cargo build --all-targets` clean, `cargo test --lib` 1195 passed / 0 failed, `cargo clippy
--all-targets` clean.** The order was safest-first: the provider mechanism and the additive
command-surface features, then the deep grammar change (U32), then the riskiest capability (U33),
then the two data-destroying/secret-handling surfaces (U30, U38) last. The session paused before
U32/U33/U30/U38 to put the risk to the owner — U32 is deep and interwoven, U33 is the spec's own
"maybe never", and U30/U38 cannot be *validated* on this Windows host — and the owner reaffirmed
all four ("finish it off"), so they were built with the safety discipline the register demands and
their pure logic and safety rules unit-tested, with the live OS operations (zfs/lvm destroy, a
real secret decrypt) noted as unexercisable here rather than claimed as verified.

**The adapter-approval family fix (prerequisite).** `linix lock`'s `approve_adapters` approved a
*hardcoded three* — backends/settings/bootstrap — and silently omitted `firewall.toml`. A repo
carrying a firewall adapter could therefore never approve it, so its rows were refused on every
sync: a live sibling of "a guard on one command is a guard on nothing", in the very folder the
approval mechanism exists to protect. It now scans every `*.toml` in `adapters/`, so firewall is
fixed and each new kind (`init.toml`, `snapshot.toml`) is approvable the day it lands. The one
approval check was hoisted to `core::hook_lock::adapter_refusal` so the onboarder and the new
snapshot loader share it rather than each re-deriving the hash/verdict dance.

**U36 — init systems are declared provider rows.** `backends/service.rs` was a closed
`enum InitSystem` behind a hardcoded command match; it is now a table. The shipped five live in
`init_providers.toml` and a user's `adapters/init.toml` adds more through the same approved
loader, so s6/dinit/runit/Shepherd are six lines of TOML instead of a release. A row register
last and never shadows a built-in; a row that cannot both start and stop is refused, not
half-loaded; restart with no native verb derives stop-then-start. systemd keeps its `--`
terminator; the tested behaviour is now data.

**U27/U28/U29 — the snapshot layer becomes data.** A `ConfigSnapshotProvider` reads
`adapters/snapshot.toml` through the one ledger and registers LAST, so a row never shadows a
built-in. The safety rule that keeps it from being the V.60 footgun: a capability the row does
not declare, it does not have — `restores_running_system` defaults false, so a provider is
create-only unless the file names live restore *and* gives a restore command; a "restore" that
could roll nothing back never runs, it refuses and names the snapshot. **U28**: the active
provider is chosen by a declared `snapshot_priority` list (the `priority` shape) — first
available in the list wins, not registration order, not capability-guessing; empty keeps the old
behaviour. **U29**: APFS on macOS, declared create-only (a restore needs a recovery reboot, so
claiming Live would be exactly V.60); the second platform gains a safety net, and the ownership
rules mean LiNix never reaps an APFS snapshot it cannot prove it made.

**U35 — user-defined verbs.** A `[verbs]` table names a sequence of built-in verbs
(`refresh = ["sync", "upgrade --all"]`), riding the existing pre-clap alias substrate. A verb
never shadows a built-in; each step inherits the leading global flags; **composition only** is
enforced — a step that names anything but a built-in is refused with a pointer at `exec:`/U33.
One data lock covers the whole verb, taken as a writer (the safe default). The arbitrary-argv
half of U35's ruling waits on U33's key.

**U34 — `linix repl`.** A prompt over the *one* resolver: a name delegates to
`resolve_spec`, `when <expr>` to `config::parser::eval_when`, `:vars`/`:eval` to the resolver's
own methods (the U20 rule — a thin front end, never a second implementation). Read-only, no
locks.

**U32 — module parameters.** `param user` / `param gpu = none`, bound by `use workstation(
user=shaul, gpu=nvidia)`. The substitution reuses the `$name`/`vars` machinery one scope wider —
params bind *before* `when` is evaluated (so a `when $gpu` inside the module sees the parameter),
an unknown `$ref` is left for the global pass so the scopes compose, and a missing required param
is a loud error, never a silent empty string (V.78). Args thread the four data-flow points —
profile `use m(args)` → `reach` → set-math → `expand`, and module→module — each covered by tests.

**U33 — generated declarations, off by default.** `generate:PATH` runs a command and treats its
stdout as declarations, spliced into the stream before probing and collection so it gets the same
guard, conflict check and removal preview as a typed line. Four rules keep it on the safe side of
XIII.32 (V.79): off by default (`allow_generators`), ledger-approved (scanned first in `lock`,
because resolving now runs generators), a failure is a failed resolution (never an empty set,
VI.0), and the output is shown not trusted. The `exec:`-may-do-anything half is the U4
documentation amendment: exec already runs arbitrary code gated per-script by the ledger, so no
new blanket gate was added (it would break every existing `exec:` line).

**U30 — storage objects.** ZFS datasets and LVM volumes join btrfs as one family; being ordinary
backends is what routes their filesystem-destroying `remove` through the normal guard, protectable
and counted like a package (V.80). Pure argv builders unit-tested; the live zfs/lvm operations
cannot run on this Windows host and are marked so, not claimed.

**U38 — declared secret providers.** Vault/1Password/KMS/GPG as `[[secret]]` rows plugging into
the existing decrypt path, inheriting the T-series plaintext rules unchanged. A provider that does
not promise `stdout_only = true` is refused, not trusted (V.81). The pure gate/command logic is
tested; a real decrypt is not exercisable here.

**What could not be exercised on this host, and is recorded as such rather than claimed:** the
live zfs/lvm create/destroy, a real secret-provider decrypt, and U27's "built-ins become rows"
half (btrfs/zfs restore, Windows System Restore) still need a Linux box with those filesystems /
a real secret manager. The argv construction, the config gates, the ledger approval, the
fail-loud paths and the safe-default refusals are all unit-tested; the OS-level effects are the
container/hardware job, the same boundary btrfs/zfs restore has always sat behind.

## Session 2026-07-26 (later the same day) — building the ruled backlog, safest-first

**Eight ruled items built, each committed on its own, `cargo test` 0 failed and `cargo clippy`
clean at the end.** The order was the plan's — additive and testable first, the safety-critical
and hardware-blocked last — and the two of those that could not be validated on this host were
deliberately left rather than built blind.

**Tier 1 (four of five):**
- **D3b — download-only artifacts.** `ArtifactOptions.download_only`; `@download_only` fetches
  without unpacking/shimming/PATH and is refused off a non-download backend; `github:` does the
  same by *default* when nothing it was asked for is installable (only a `.deb` on offer) rather
  than failing. web/appimage skip their deploy step. The dead `@type=program` branch in `web.rs`
  (unreachable — `type` was never a grammar key) was replaced by the real flag.
- **K4 — `clean_cache_on_remove`.** New `config::{clean_cache_on_remove, cache_dirs}` +
  `model/cache.rs`: on removing a download-backend package, delete cached copies by **exact
  filename, regular files only, bounded depth** across the user's dirs plus `$XDG_CACHE_HOME`/
  `~/.cache`/`/var/cache`. Conservative by construction — a cache clean is a removal.
- **D13 — a channel change refreshes in place; a downgrade is named.** snap `info` reads
  `tracking:`, the planner detects a *readable* channel change (readable-only, so no refresh loop),
  `snap install` switches in place via `snap refresh --channel=`, and a downgrade (edge→stable, an
  older build) is named loudly in the plan. snap wired; flatpak stays inert until its channel is
  parsed.
- **T6 — `@backup=no`.** The per-line opt-out `link.rs::wants_backup`; restore-on-removal was
  already built.
- **D5 left unbuilt on purpose** — it runs `dpkg`/`rpm` and needs a real apt box to verify
  cross-backend dedup, so it is for a session that can test it.

**Tier 2 — 7e finished.** Windows registry is a full `setting:` adapter row (HKCU/HKLM folded into
the argv so `@scope` picks the hive; `already_set` now reads `reg query`'s verbose output for
idempotency — verified against this Win11 host). KDE (three-part addressing), COSMIC (a file tree,
not a command store) and Hyprland (**not** a `setting:` — `link:`'s job) are ruled in the table's
own comments; the mechanism is complete and open.

**Tier 3 — the three items independent of the snapshot mechanism:**
- **U26 — BSD backends.** FreeBSD `pkg` (single binary) and OpenBSD `pkg_add`/`pkg_delete`/
  `pkg_info`. OpenBSD's separate remover needed a model extension — `ManagerConfig.remove_binary`,
  wired through `run_removal`, with an empty `remove_args` no longer read as "no uninstall" when a
  separate remove binary is set — which also lets a *custom* backend declare a separate remover.
  `parsers/bsd.rs` shares the pkgsrc name-version heuristic under the right label.
- **U31 — health commands ride the II.12 ledger.** `linix lock` approves every declared health
  *command*, and `sync` refuses before the change if any is unapproved (a check that cannot run is
  a failed check). Port probes run no code and are never gated. The open vocabulary was already
  there; this is the trust half.
- **U37 — notifications = docs.** A copyable Slack/webhook `hooks/after_sync` in `readme.md`, and
  the ruling recorded that there is no `[[channel]]` block — the hook already sends anything.

**Not built, and why — the snapshot-provider cluster (U27 and everything on it), U36, D5.** U27's
*additive* half (a config-driven `ConfigSnapshotProvider`, restore-capability declared, ledger-
approved, registered last so it never shadows a built-in) is the safe design and is what delivers
the user value; the ruling's "built-ins become rows too" half was **not done blind** — btrfs/zfs
restore and Windows System Restore cannot be validated here, and Windows System Restore is
typed-PowerShell that cannot become safe argv-data without reintroducing SEC5, which is an owner
question. U36 (init `[[init]]` rows) is safe to do here — its `plan_service` argv tests guard the
enum→table conversion — and was deferred for time, not risk. Rewriting a tested safety net blind
is the one thing "we cannot afford bugs here" forbids, so it waited.

**Fixtures the changes shifted, fixed in the same pass:** three `setting.rs` shipped-store counts
(two rows now), the `download_only` grammar test (its harness knows `appimage`, not `web`), and one
clippy `unwrap`-after-`is_some`.

## Audit 2026-07-26 — the specification checked against the tree, claim by claim

**Nothing was built this session. The spec was made to match what was.** Every checkable claim in
`docs/spec/` was re-derived from `src/` by grep, and the stale ones rewritten. What was wrong, in
descending order of how much it would have cost the next person:

- **Part VII (this file) was 52 commits behind** — see the note above.
- **Part III's ordered build list was wrong about six of its eleven near-term items**, all in the
  same direction: it listed as unbuilt work that had already shipped. `K2` (rebuild), `7c`
  (backend bootstrap), `7d` (`sync --locked`), `7n` (dotfiles) and `7o` (`firewall:`) were
  committed on 2026-07-24 — before the list naming them as remaining was written on 2026-07-26 —
  and `T6` was sitting finished in the working tree. Two more (`7e`, `U31`) were listed as wholly
  unbuilt when they are half built. **The list was assembled from this file, and this file did
  not know.**
- **The decision register's index claimed 59 open questions over 59 entries that each said
  ANSWERED.** The three index tables — *Open and blocking — 13*, *Open, not blocking — 46*,
  *Answered — 43* — were never touched as the rulings landed between 2026-07-23 and 2026-07-26.
  The register is the one file whose entire purpose is to stop a settled question being re-argued,
  and its front page was inviting exactly that. Rebuilt from the entries' own `Status:` lines:
  **104 decisions, 102 ANSWERED, 2 PARKED, 0 OPEN.**
- **Part II was missing three shipped statements.** `exec:`, `dotfiles:` and `firewall:` are in
  `Statement`, are parsed, are tested and are documented in the readme — and II.2's statement
  table did not list them. A grammar table that omits a shipped statement reads as the closed set
  it is not. The option-key table was missing `scope`, `backup`, `runs`, `undo`, `value` and
  `notify` for the same reason.
- **Part II carried a rule the owner had reversed.** II.11b still said *"Scope is required. A bare
  `linix rebuild` errors and names the three forms"* two days after K2 ruled warn-and-proceed and
  `app/rebuild.rs` stopped bailing. This is the failure mode Part II is most dangerous in: it is
  canonical, so a reader who found the code disagreeing with it would have "fixed" the code.
- **Part II's II.10 enumerated seven removal paths; `GuardScope` has twelve.** The paragraph
  directly above that list says *"a list is an assertion about what is absent"* and *"count the
  paths from the code"* — and the list under it had not been. It is generated from the enum now.
- **II.8's command table still listed `unmanaged`, `absent`, `status` and `doctor`** as commands,
  eight days after U9 folded them into `check` and deleted the names.
- **Part VI's G3 said `teleport` was deleted and the grep was silent.** It is not: a later session
  re-added it as a declarative command, correctly, and II.8 lists it. A record of a deletion
  outlived the deletion.
- **Part V gained six entries** (V.66–V.71) and an addendum to V.49, because rules had been added
  to Part II across four sessions without the matching *why* — which is the rule `CLAUDE.md` puts
  above almost everything else.

**What was checked and found true**, so it is not re-verified next time: Phase 0's deletion list
(every II.17 entry is quiet in live code; the remaining hits are comments recording the bug each
one caused); Phase 1's unification (every surviving `backend:name` splitter either is the grammar
or consults the registry first, with the one in `context.rs:1336` validating on the next line);
the guard's ten refusals against `Objection` + `Protection`; and the `[guard]` template test that
holds four disarming keys out of the shipped config by assertion.

**Not verified, and now marked as such:** Phase 0's *"delete the ~884 marketing comments"*. That
figure was measured on 2026-07-16 against a smaller tree and has never been re-measured; `src/`
carries 8,896 comment lines today and the marketing subset is a judgement no grep makes. It is
recorded as unmeasured rather than as done.

**The suite at the time of the audit:** `cargo test --lib` — **1139 passed, 0 failed**, which
includes the uncommitted `@backup=no` work in the tree.

## Session 2026-07-26 — the extension-surface round, ruled: U27–U38 and T6

**Fifteen commits, no features. Every remaining open decision was put to the owner and ruled**,
which is what took the register to zero-open. The through-line is XIII.33: a closed provider-list
is a place where LiNix decided in Rust something the user could have declared in data.

- **U27 — snapshot providers are a full plugin.** A registry beside `BackendRegistry`, providers
  read from a file through 7a's one approval ledger, built-ins as rows. **Restore capability is a
  required declared field, never inferred** — a provider that does not prove live restore is
  create-only and refuses the rollback (V.60).
- **U28 — the active provider is chosen by a declared priority list**, the `priority` shape:
  first available in the list wins. Not by capability-guessing, not by registration order.
- **U29 — macOS gets APFS**, declared **create-only**, because an APFS restore needs a reboot into
  recovery and is therefore not a live undo.
- **U30 — storage objects (zfs/lvm/btrfs) are one family**, and a `remove` that runs `zfs destroy`
  goes through the **ordinary** guard gate — no special escalation — with a volume protectable
  exactly like a package. zfs/lvm have no implementation; this is real new work.
- **U31 — health checks are an open vocabulary.** A user-declared check command, argv, exit 0 =
  healthy; a check that cannot run is a **failed** check; and it rides II.12's ledger like every
  other runnable thing. *(The vocabulary half turned out to be already built — `Probe::Command`
  takes any command. The ledger half is not.)*
- **U36 — init systems become `[[init]]` rows**; the built-in enum becomes rows the same loader
  reads. A row missing `start`/`stop` is refused, not half-used.
- **U37 — notifications route through the existing event hook.** No new channel kind: a channel
  beyond desktop/email is a 7j hook that shells out. Documentation, not mechanism.
- **U38 — secret decryption opens to declared providers**, the T-series gate now being clear
  (T1/T2/T5 fixed, T7 settled) — bound by the plaintext rules, and a provider that cannot promise
  them is refused rather than trusted.
- **U32 — modules take parameters**, and the user decides **per parameter** whether it is typed.
- **U33 — generated declarations are built, AND `exec:` may do anything** — each behind its own
  config key, off by default. **This overrode the recommendation and XIII.32's refusal**, and it
  amends U3 and U4. The guard, the removal preview, the ledger and fail-loud are never waived; a
  failed generator is a failed sync.
- **U34 — `linix repl`, if it is easy.** The non-negotiable is a single engine: it shares the one
  parser and resolver or it is not built.
- **U35 — user-defined verbs are built**, and the owner widened it past the recommendation's
  composition-only scope: a verb may run arbitrary commands, through U33's trust model.
- **T6 — both sub-questions closed.** Per-line `@backup=no`, no machine-wide key; and no
  orphan-cleanup command, because restore-on-teardown means backups cannot pile up.

## Session 2026-07-24 (second half) — Phase 7 built, almost all of it

**Thirty-seven commits (`c590036..HEAD` up to 2026-07-24), plus four immediately before them that
this file never recorded at all** — `b772b71` (7b `exec:`), `2060d4b` (7d), `82d659d` (the
`adapters/` folder, U10) and `8c2ec0b` (U19 `@scope`). **The phase that was going to take weeks
landed in a day**, because 7a's loader and K17's adapter-table ruling meant most of these were
rows and wiring rather than new mechanisms. Everything below is in the tree; each names its
commit. Written retrospectively on 2026-07-26 from the log and the source, not from a session
note — which is why it is shorter on reasoning than the entries above it, and why the rule at the
top of this file exists.

**The features:**

- **7b `exec:`** — `b772b71`: run a script the config carries, once per content, keyed by content
  hash in `locks/exec.toml` so a script that succeeds and makes its own condition false does not
  flap. **`@undo=` (U3)** followed in `413c3a0`: what removing an `exec:` line means is `@undo=`
  if the line carries one, and dropping the record and saying so if it does not — LiNix does not
  invent an inverse for a script whose author did not write one.
- **The `adapters/` folder (U10)** — `82d659d`. Three files, one folder, in the shareable config:
  custom backends, setting stores and bootstrap rows all read by the one approved loader.
- **S31/S32, found by the `tools` container image on its first run** — `9f5406e`. `mise`'s `info`
  asked the plugin *catalogue* rather than the installed set, so the planner thought every mise
  tool was already installed and `linix install mise:X` printed *already up to date* while
  installing nothing — **every `mise:` line in every config was inert.** The sibling in the same
  file: `list_installed` ignored mise's own `"installed": false`, so a tool stayed listed after
  `mise uninstall`. No unit test could have reached either; none of them runs a real mise.
- **7c backend bootstrap (U10)** — `ed0d996`. `adapters/bootstrap.toml`, through the ledger; the
  command is printed in full and confirmed before it runs (P8: ask, then do).
- **7d `sync --locked` (U11)** — `2060d4b`, and **generalised past the question asked**. `sync`
  itself now defaults to the recorded version; `watch` inherits it; moving forward is
  `--upgrade`. It was a live defect: `locks/versions.json` was read only under `--locked`, so a
  machine rebuilt from a config got whatever upstream published that morning.
- **7f health-checked upgrades (U7)** — `323215f`. Two scopes, one revert path; a declared check
  with no snapshot provider is refused **before** the change (V.65).
- **7g kernel/DKMS rebuild** — `fce6ae7`, verified against real DKMS in a container (`cfff4dc`):
  a forced `linux-headers-generic` install drove `dkms autoinstall`, and an unbuildable module
  failed the sync loudly at exit 1, before any reboot. Matcher narrowed in `3299b6b` so
  `kernel-tools` and friends no longer trip it.
- **7h `linix try` (U12)** — `a014ed0`. Config mounted read-only, container `--rm`; no runtime is
  a refusal naming what is missing (`0bf1971` stopped it asserting what it could not know).
- **7i the ten looking commands become one (U9)** — `5418def`. `check` with sections;
  `doctor --fix`'s repairs moved to `heal`. **`check` looks, `heal` acts.** An `approvals`
  section followed in `d8bf5d9` for event hooks that would otherwise silently not run.
- **7j event hooks (U15)** — `85632eb`, with two real fixes on top: they read `[events]`, not
  `[hooks]`, so they no longer double-fire (`d984791`), and `on_guard_refusal` fires where a
  refusal becomes an exit code rather than inside the guard (`9660bbf`) — a side effect inside a
  decision function runs wherever the decision is evaluated, including in tests, which would have
  had `cargo test` executing the developer's own hooks.
- **7k `linix eval` (U17)** — `afbae78`. Versioned JSON, no locks, repo-relative sources.
- **7l `git blame` for a declaration** — `6d93235`. `why` names the commit from git's pickaxe;
  nothing new is written at sync time to support it.
- **7m the exit-code table (U21)** — `ed0d996` + `8837042`. Every guard refusal is
  `Error::Refused` → exit 3, from one place; the install ceiling used to exit 1 while its
  siblings exited 3.
- **7n the dotfiles directory (U22–U25)** — `c7dea64`. Files, never directories; each file keyed
  individually in the extras ledger so the teardown is the shared one; never decrypts.
- **7o `firewall:` (N4–N7)** — `ca9466b`. Three adapter rows (`ufw`, `firewalld`,
  `windows-defender`) in `firewall_adapters.toml`, and the session-port lockout check that is the
  feature's precondition rather than one of its features.
- **U2/U6/U26** — `d66730e`. First-class custom backends, Linux-only markings, and `when family`
  answering correctly on a host with no `/etc/os-release` (never empty — an empty family is what
  makes every `family ==` silently take the else branch).
- **U14 `linix add`** — `daafadf`, covered in the container matrix by `c85b939`.
- **U16/U13** — `780ae78`. Absolute-path binaries for a custom backend; `@runs=always` visible.
- **U18 backend groups** — `8caddfe`. Named backend chains, nestable.
- **U19 `@scope=user|system`** — `8c2ec0b`, on the three statements where it can vary.
- **K2 rebuild warns-but-rebuilds** — `51bd3b1`.
- **D11/D12/D14** — `5c50a5e`, `b66d662`. The default format order is versioned so a LiNix upgrade
  cannot silently change it; `why` explains which rule chose the artifact.
- **T3/T4 hardware-token decrypt** — `d0d159a`. Timeout, and an unattended tick skips rather than
  hangs on a touch-required key.

**Also:** `a135591` moved `after_sync` to fire **last**, after the check that can revert
everything — a post-sync hook that ran before the health check could act on a machine that was
about to be rolled back. `1a5131d` deleted the `BEHAVIOR.org` sections describing a program that
no longer exists. `b4542e5` recorded U20 (LSP) as deferred: not "very easy", and `check` is the
thin hook if it is ever wanted.

## Session 2026-07-24 (first half) — real-world validation, and making the read paths fast

**The Ubuntu container integration harness passed, run for real** (`DISTROS=ubuntu
./docker/integration/run.sh`, via Docker in WSL — recorded in memory). 271 hard checks, 0 fail, 5
soft; the coverage audit is green (7 backends got a real install→list→remove, 45 plan-smoked,
every registered backend and every non-exempt subcommand exercised). This is the first time
Part VII can cite a container run rather than a WSL-by-hand one. The remaining images
(fedora/arch/alpine/tools) are the same harness on other bases and are the next runs.

**One thing the harness surfaced: `teleport` is alive, and correctly so.** R2 deleted the
imperative `Teleporter` engine that built its own transaction and bypassed the guard, and the
history recorded "grep is silent". It is not silent: a later session re-added `teleport` as a
**declarative** command — `handle_teleport` calls `App::retarget` to rewrite the declaration's
line, then `handle_sync`, which runs the guard. That is exactly the shape R2's own ruling
permitted ("a convenience verb must route through `handle_sync`, never its own transaction"), so
the code is right and only the R2 entry's "deleted / grep silent" line is stale. No second
transaction engine exists; `core/transaction.rs` is the only one.

**Performance: the hot paths fan out now, and they were validated on real package managers.**

- **Read fan-out.** `App::list`, `get_info` and `installed_but_unmanaged` queried every backend
  one after another — each a separate process (`apt list`, `cargo install --list`, …). They now
  go through one `query_backends_concurrently` helper, bounded by `max_parallel`, results in
  registry order. `get_info` no longer waits on every backend that lacks the package.
- **The planner.** `identify_needed_actions` checked "is this installed?" one declared package
  at a time — the dominant cost of `sync`/`status`/`plan` on a large config. It overlaps those
  waits with `buffer_unordered` now; the per-spec decision is extracted unchanged into
  `spec_is_missing`. Plan OUTPUT is sorted now, so the same change set prints identically each
  run (it was HashMap-ordered, i.e. non-deterministic, before).
- **`update`.** Metadata refresh fans out too. `upgrade` is deliberately left serial: it changes
  packages, so concurrent sudo operations would interleave, and it is dominated by real work.

`buffer_unordered` borrows `&self`, so the fan-out stays on one task — no spawn, no `'static` —
which is all that is needed because the time is spent waiting on child processes. Covered by
`list_aggregates_every_backend_that_answers` and `the_report_is_sorted_for_a_stable_plan`.

**The `tools` image is green too, and it earned its keep on the first run.** 316 hard checks, 0
fail, **18 real backend lifecycles** (against 7–9 on the fast images) — the ecosystem managers
getting the real install → list → remove that IV.1 said the image existed for and had never
delivered. That first run found **S31**, a fail-silent bug no unit test could reach: `mise`'s
`info` asked the plugin *catalogue* rather than the installed set, so the planner thought every
mise tool was already installed and `linix install mise:X` printed *already up to date* while
installing nothing. Every `mise:` line in every config was inert. Fixed, re-run, 0 fail, twice.

**Real-world, run for real via Docker in WSL (memory: [[docker-in-wsl]]) — the whole fast matrix
is green:** ubuntu (apt) 271/0, fedora (dnf) 279/0, arch (pacman) 271/0, alpine (apk) 266/0. Zero
hard failures on any image; each built `linix` from the session's final source, so the
concurrency changes above are exercised against real `apt`/`dnf`/`pacman`/`apk`, not only mocks.
This is the fast half of IV.2's matrix, the set CI runs on every push. **This is the first time
Part VII can cite the container matrix rather than a WSL-by-hand run** — the gap IV.2 named
("verification that only ever ran by hand on one machine is a claim, not a gate") is closed for
these four. The slow `tools` image (the ecosystem managers' real lifecycle) is the next run.

**Reliability audit.** The resolver, config parser, edit path and the untrusted-URL download
backends (`web`/`appimage`/`github`) carry **zero `.unwrap()`/`panic!`/`unreachable!` in
non-test code** — a malformed config or a hostile release is a clean error, never a crash. Three
user-facing messages that printed a run of spaces mid-sentence (collapsed line continuations, one
from this session's own T5 work) were fixed.

## Session 2026-07-23 (sixteenth session) — building, starting with VI.0

**S24 and S25 are fixed.** They are one code path seen from the execute side and the preview
side, so they landed in one commit with four tests. `cargo test` is green and `cargo clippy
--all-targets` is silent; run them rather than reading this.

**The fix is a deletion**, per V.64: `heal()`'s pre-reinstall `let _ = handler.remove(…)` is
gone, along with the comment above it that argued for it in this document's voice. Recovery of
an interrupted install re-runs the install. Nothing was added to the guard, because the guard
was never the thing that failed — the path was.

**Two decisions I made while building, both recorded here rather than asked, because neither is
visible from outside the program.**

1. **S25's dry-run check went inside `heal()`, not at the call site the ruling named.** The
   ruling said to move `heal()` below `reconcile`'s dry-run branch. That fixes `sync --dry-run`
   and leaves `linix heal --dry-run` — a second caller, four thousand lines away, reaching the
   same mutation. Fixing the branch and not its sibling is the failure `CLAUDE.md` opens with,
   so the refusal lives in the function every caller must pass through. The ruling's effect is
   met and its mechanism is stronger.
2. **`TestKernel` no longer sets `config.dry_run = true`.** The fixture claimed preview-only
   while every test using it asserted that commands ran; the executor is a mock, so nothing
   reached the machine either way. It meant **no test built on that fixture could have caught a
   preview-only defect**, which is exactly the defect S25 turned out to be. It is now an
   ordinary run against a mock, and the one test that wants the preview path clones the config
   and sets the flag itself. All 972 tests pass under the change.

**The test that vouched for the bug is deleted, and so is its duplicate.**
`test_journal_self_healing_logic` was documented as verifying that healing *"correctly
uninstalls and re-attempts"* and primed the mock with `brew uninstall stale-pkg`. There was a
**second copy of it** in `tests/security_and_resiliency_tests.rs` — same fixture, same primed
uninstall, weaker assertions — and nothing in fifteen sessions had noticed the pair. Neither
was ported. The replacements assert the shape rather than the symptom: no uninstall **and** the
install re-run; the removal branch still removes; a protected removal is still refused and its
entry still resolves; a dry run does none of it and leaves the journal wanting recovery.

**Not built, deliberately:** II.10's per-backend remove-before-install capability. No backend
declares it or is known to need it, and a capability with no caller is a second removal path
with no test — which is the thing being removed here.

### The ruled work, in the order the plan listed it

**K13 — the unattended-refusal set is a `[guard]` list.** `NEVER_UNATTENDED` is deleted; the
same two names ship as `never_unattended`'s default and a machine permits one by taking it out.
The list is threaded into `schedule_config` rather than read inside the model, so it has one
home and the check needs no config on disk to test.

**D1 and K14 were already done, three days before the register said they were owed.** D1's
"errors if both tag spellings exist" landed in `8a63c80` on 2026-07-20 with tests for both
halves; K14's test is section 12 of the container harness, in the honest shape the entry asked
for — a real package really removed and reinstalled, git asked directly for its commit count.
**What K14 owes is a run, not a test**, and there is no container runtime here.

**7a — a backend definition travels with the repo.** `custom_backends.toml` moved into the
config repo and the machine-local path is deleted, not kept as a fallback. It inherits II.12
rather than getting a trust model of its own: the file's hash is in `locks/hooks.toml`, `linix
lock` approves it, and an unapproved or edited file registers nothing. Two choices worth naming:
the check is at **load**, not at the sync gate, because a registered backend is reachable from
`search` and `list` which no sync guards; and **one identity for the whole file**, because a
per-definition identity would let an edit that *adds* a `[[backend]]` pass unnoticed, and adding
one is the whole attack. XIII.12's `name`/`binary` split went through `ManagerConfig`, so every
built-in is the `binary: None` case of one rule rather than a second rule.

**K17 — a `setting:` adapter is a row, and `gsettings` is a row in it.** The enum is gone.
User rows live in the same `custom_backends.toml`, so the adapter inherits 7a's approval instead
of needing a second ledger entry for an identical question — one reader, one refusal message.

**D2 — the classifier was checked against real releases, and the check found two live defects.**
Both were the quiet kind the entry predicted. The code asked *does not contradict this machine*
where the rule says *matched this machine*, so `MD5SUMS` — an actual asset of every rclone
release — was an executable candidate on every platform. And the token matcher would not end a
word on digits, so `jq-linux64` — an actual asset of jq's release — named no OS and was an
executable candidate on Windows. The fixture is real filenames with hand-verified answers, so it
is a check that can fail rather than an inspection that happened once.

**T1, T2, T5 and T6's ruled half — and the defect none of them named.** The extras ledger keyed
a `link:` by its **source**, so undoing a declaration deleted the file in the user's own dotfiles
repo and left the deployed copy in place. **Found by reading the removal path, not by a test: no
test called the undo at all.** Links key on their destination now. On top of that: removal
restores the backup and deletes it, decrypt mode cannot reach a backup, a decrypted target inside
the config repo is refused before the tool is launched, and the plaintext is restricted before it
lands — on Windows by stripping inherited ACEs with `icacls`, which is the platform the ruling
was written about and the one the test ran on.

### S26 and S27 — the two that stood between anyone and finding S24

**The rate limit is capped, retried once, and settable** (`rate_limit_max_wait_secs`, default
30). The part the ruling did not name is the part that made the old behaviour pure loss: it
slept up to an hour and then **returned the same 403 it had slept on**, so the wait bought
nothing at any length. The decision is a pure function of `(status, header, now, cap)` with five
tests, because no integration test can make api.github.com rate-limit on demand — and writing it
that way surfaced a case the entry never mentioned: **a 403 that is not a rate limit** (a bad
token, a private repo) used to be slept on too, if it happened to carry a future timestamp.

**S27's message was wrong in a way no timing change could fix.** It ended "remove `linix.lock`
if nothing is running." The lock is an OS lock on an open handle, released when the holder
process exits — so a lock still contended after the wait *proves* a live holder, and the advice
was to take the lock away from a running writer. It now says exactly that.

**S30, found by carrying one key across every layer that holds it.** Adding
`rate_limit_max_wait_secs` to the config template and to `examples/preferences.toml` meant
reading the example, which still documented `[retention.generations]` and
`[retention.manifests]` — deleted with the generation format a phase earlier and silently
ignored ever since, because `RetentionConfig` did not deny unknown fields. **The template has a
parse test; its long-form twin, the file a reader is likelier to paste from, had none.** It has
one now, and the struct denies unknown fields for the reason already written on `Config`.

### The fixture was load-bearing, and it was hiding two more bugs (S28, S29)

Turning off `TestKernel`'s `config.dry_run` did not just enable a test. **It broke one, and the
break was real** — `dependents_only_config_resolves_and_applies_the_dependent_phase` started
failing because it was now writing a shim into the developer's actual `~/.local/bin`. A 35 MB
copy of the test binary was sitting there named `rg.exe`, ahead of ripgrep on PATH. It was
deleted, and so was the mechanism: **`ShimManager::new()` resolved `~/.local/bin` itself at
three call sites**, so `Config::sandboxed` — the function written for exactly this after S11 —
could not reach it. `bin_dir` is a `Config` field now (`serde(skip)`: a repo must not be able to
move LiNix's shims onto a machine's PATH by declaration), the self-resolving constructor is
deleted rather than left beside the injectable one, and the S11 test asserts seven paths.

**Two sessions of tests could not have caught either S25 or S29, for the same reason.** A
fixture that says "preview only" while every test on it asserts that commands ran is not a
convenience — it is a whole class of defect that the suite is structurally blind to. This is the
third thing in one session found by *running* rather than reading.

**S28, the entry S24's reading asked for.** Three `remove_task` implementations swallowed every
failure, so a schedule that could not be torn down was reported as removed and kept firing. Each
now deletes through one `remove_generated` and then asserts the end state with the
`is_task_active` probe the trait already had — necessary because on all three platforms the
removal command exits non-zero for "already gone" as readily as for "refused", so **the exit
code cannot answer the question and the probe can.** Fixing it surfaced the sibling one layer
up: `reconcile_extras` dropped a failed teardown from `locks/extras.toml`, which turned one
warning into a service LiNix permanently forgot it owned. A failed undo now stays recorded and
the next sync retries it.

## Session 2026-07-23 (fifteenth session) — the refactor, and fifteen decisions that were answered in code

**No behaviour changed. One file of 9,308 lines became a map plus thirteen parts, and the six
decision registers became one.** This is the work the fourteenth session recorded as owed and
did not do.

**The split is mechanical and lossless.** Every line of the original is accounted for: 8,261
lines of parts, 861 of register entries, 186 of preamble. A line-by-line comparison against
`HEAD` leaves 64 unmatched, and all 64 are two deliberate deletions — the frozen status block at
the top (which said *735 tests passing* against a tree that now passes 970, and which the
document itself instructed readers not to trust) and the six per-register section headers, whose
content is preserved in the new register's own grouping.

**What the reconciliation found, and it is the reason the register was worth building.** The old
registers recorded a question and sometimes a recommendation, and **never whether anyone had
answered.** Checked against the tree rather than against the sentence:

- **Fifteen entries were already answered by shipped code and nobody had ruled on any of them.**
  `linix path` and `linix edit` both exist (K10). The settings file enforces its one key in the
  parser (K11). The closed format vocabulary is one table (D10). `vars.d/` is ignored by name
  (W6). Each was built to the register's own recommendation, which is the quiet failure mode:
  **a recommendation nobody rules on becomes the rule by being implemented**, and the register
  goes on describing it as an open question. They are now filed as **BUILT, NEVER RULED** — the
  owner's to confirm or reverse, and reversing costs more every week.
- **Two entries the registers filed as questions are live defects**, verified present: `T1`
  (`link.rs:319` still backs a previous secret up to a world-readable file) and `T2` (nothing
  compares `@target=` against the config root).
- **Nineteen are open and blocking, thirty-three open and not.** The largest single block is
  `U1–U26`, which is Phase 7 — and **U19** (is LiNix acting for a user or for the machine) gates
  `7e` on its first line, as the thirteenth session already warned.

**Kept deliberately:** the PROMPT and its rules of engagement, verbatim, in `SPEC.md`. It is
instruction, not description, and the one part of the document that every session reads first.

### Eighteen rulings the same day, and the two that changed a design

All fifteen *built, never ruled* entries were put to the owner and answered, so that category is
now empty. Twelve were confirmed as built. Three were not:

- **K7 — `setting:` must work everywhere, and `gsettings` is a stage.** The owner runs KDE,
  Hyprland, COSMIC and Windows; **the only adapter that exists is the one store they do not
  use.** "Everywhere" was then explicitly widened from the four they named to the general rule:
  a blessed list of five is a list always missing the sixth, and the machine holding the sixth
  gets a refusal for a key LiNix could have written.
- **K17, which that ruling created and which is filed as blocking.** A closed
  `enum SettingStore` cannot mean everywhere — every new desktop would be a LiNix release.
  **Ruled: adapters are a table, the built-ins are rows in it, and adding a store is a plugin.**
  `gsettings` stops being special, because *an adapter mechanism the built-ins bypass is one
  nobody has tested.* This precedes 7e: the second adapter is where the shape sets.
- **K13 — reversed, then generalised past the question that was asked.** The proposal was a key
  that opts `rebuild` into `schedules`. The owner's answer was better: **the forbidden set is a
  list in `[guard]`, shipped with `rebuild` and `purge-unmanaged` in it, and you take a name
  out.** That answers the sibling in the same change rather than leaving `purge-unmanaged`
  refused by a constant — the exact patch-one-line-leave-the-sibling failure `CLAUDE.md` names,
  which the first draft of this ruling had walked straight back into.

### The five that blocked building, and a green test that asserted the worst bug

The owner asked to stop deciding and start building, with the builder asking as it reaches
questions rather than in advance. **Five decisions genuinely blocked the first commit** and were
ruled; the rest can wait to be reached. The asking rule is now in `CLAUDE.md`: stop for a
register ID, for user-visible behaviour, for removing a feature, and for Part II looking wrong —
nothing else, and a ruling ships in the commit that answers it.

**S24's ruling took the stronger of the two options offered.** The recommendation was to route
`heal`'s pre-reinstall removal through the guard; the owner ruled that **it does not remove at
all.** Recovery re-runs the install. Remove-before-install becomes a per-backend capability, off
by default, and where a backend declares it the removal is ordinary — guard, count, plan,
`--dry-run`, real error. The reasoning is in **V.64** and it is about paths rather than checks: a
guard is a good defence against a removal you know about and no defence against one nobody
remembers is there, which is what this was for thirteen sessions.

**Then the code was read, and it produced the finding of the session.**
`tests/critical_paths_tests.rs:186` is documented as verifying that healing *"correctly
uninstalls and re-attempts"*, and primes the mock executor with `brew uninstall stale-pkg`.
**The suite was not blind to this path. It asserted it.** VI.0 says the 970-test green run was
aimed at paths nobody thought of; that is too kind. Somebody thought of this one, wrote a test
for it, and wrote the wrong expectation — so every run since has been confirming the bug. **A
test can do worse than miss a defect: it can vouch for it**, and a vouched-for defect is
invisible in exactly the way a missing test is not, because the grep for coverage succeeds.

Also established, so it is not rediscovered: the comment at `sync/mod.rs:404-408` argues for the
bug in this document's voice and is deleted with the line it defends; `watch` needs no separate
fix because `reconcile()` already unified it; and of the seven `let _ =`-around-a-removal sites,
six are LiNix's own files rather than user software — though `scheduler/mod.rs` swallows a failed
removal silently, which is a different rule and gets its own entry rather than a ride on this one.

**No code was changed this session, on the owner's instruction.**

### The firewall, approved — and N1 turned out not to be a firewall question

**N1, N2, N3 ruled; N4–N7 still open.** The useful part is N1. It was written as *"is a declared
perimeter exclusive or additive"* and offered as the feature's central choice. The owner's answer
was that **it is not a firewall question at all: `sync` is additive and `purge-unmanaged` is
exclusive, as always** — the model's existing split, applied to a new backend, and the same
answer for every backend that will ever ask.

**The rule was true the whole time and had never been written down anywhere.** That is why the
question could be asked, and why answering it produced two edits rather than one: **II.11** now
states it, and **V.63** records the bug it prevents — a backend shipping its own exclusive mode
has re-implemented `purge-unmanaged` without its ratio guard, its listing, or its snapshot.

**N3's answer changed because of a ruling made an hour earlier.** The entry argued that below two
adapters the honest recommendation was to build nothing, and that argument was entirely about
cost per adapter. **K17 changed the cost** — adapters are a declarable table, so five firewalls
are five rows, not five Rust backends. The revised recommendation was put to the owner *as* a
change of position rather than presented as if it had always been the answer.

**N1 also narrowed N7 without anyone deciding N7.** "Does `watch` revert firewall drift" can no
longer mean "does it purge rules nobody declared" — that is `purge-unmanaged`'s job. It now means
only: when a rule LiNix owns is changed out of band, does an unattended tick put it back? Which
can close a port somebody opened at 2am, with nobody there to read the refusal.

### The secrets batch, and a recorded defect whose stated mechanism was false

**T1, T2 and T5 ruled; T6 and T7 opened.** T1 is the one worth recording, because checking it
against the code before writing the ruling found that **two of the three facts the register gave
for it were wrong**, and the real defect was worse than the recorded one:

- The backup is *not* written under the default umask — `tokio::fs::copy` (`link.rs:203`) copies
  the source's permission bits, so a `0600` original yields a `0600` backup.
- `.linix-backup` is *not* absent from every ignore file — `core/git.rs:169` writes
  `*.linix-backup` into the config repo's `.gitignore` at `linix git init`.
- **What nobody had written down: nothing ever deletes the backup.** `remove` (`link.rs:369`)
  drops the target and leaves `<target>.linix-backup`, and `backup_once` refuses to clobber an
  existing one. A decrypted credential's predecessor therefore outlives the declaration
  permanently, invisible to `check` because it is not managed.

The entry had been reviewed as a security finding twice and the mechanism was never checked
against the file it names. **This is the fourteenth session's lesson arriving again in the same
week:** an audit reads what is written; only running it reads what is there.

**T6 came out of the ruling rather than the register** — the owner asked for a way to opt out of
`backup_once` or bound how many pile up, which is not a secrets question at all: every `link:`
managed-content write calls it. Its sharpest sub-question is whether removing a declaration
should *restore* the backup, which is the shape `extras_lock` already has for every other extra,
and which would make the backup a rollback rather than a leak.

**T7 reopens runtime injection into process memory.** XII.2 ruled it out and said not to re-open
it; the owner has since said the conversation stays open. The refusal is downgraded to a
question rather than withdrawn, and XII.2 now says so — a document that still reads *"do not
re-open"* about a live discussion is the drift this whole refactor exists to end.

**`linix reset` appeared in K16's explanation as a contrast and read as part of the decision.**
Recorded in the entry, because a comparison offered to clarify a decision became a second thing
the reader had to rule on.

## Session 2026-07-23 (fourteenth session) — a live uninstall, found while reading the document

**No code changed. Four bugs recorded (S24–S27), one of them caught mid-execution.** The
session was meant to be an accuracy pass over this file; it started by checking whether the
tree still builds, and `cargo test` could not write `linix.exe` because a LiNix process was
holding it.

**What that process was doing:** `linix -y install nimble:nimjson`, started seven minutes
earlier by `scripts/integration-windows.sh`, with one child — `winget uninstall --silent
Google.Chrome.EXE`. It was killed on the owner's instruction. Chrome survived; the uninstall
had not completed.

**The chain, established from the journal rather than guessed:**

1. The harness ran `adopt` against the real host, so `modules/adopted.txt` declares 98 real
   packages — `winget:Google.Chrome.EXE` at line 82 among them.
2. A sync opened an `Install` transaction for that line and was interrupted.
3. The next run called `reconcile` → `heal()`, and `sync/mod.rs:432` recovers an interrupted
   install by **uninstalling first** — `let _ = handler.remove(&package)` — which is the
   `winget uninstall` that was observed.

So the removal was not drift, not `absent:`, not `purge-unmanaged`, and not a mis-scoped
manifest. **It was the recovery path, removing a package that every file in the config
declares.** It reached no guard, its result was discarded, and it appeared in no plan.

**What this says about the document.** S6 already asked whether a heal removal should pass the
guard, decided the question was real, and deferred it — *for the branch that completes an
interrupted removal*. The branch that completes an interrupted **install** also removes, and
nothing here had noticed it. One paragraph covered one of two paths and read as if it covered
the mechanism. That is the same shape as the `command -v` case in `CLAUDE.md`: a fix, and its
untouched sibling.

**Three more, found in the same twenty minutes and all in the way of anyone testing on a real
machine:** `--dry-run` runs `heal()` and takes no data lock (**S25**), so a preview mutates;
the GitHub 403 handler sleeps up to an hour and then returns the 403 anyway (**S26**), holding
the lock while it does; and the lock's own wait is 120s against a sleep of up to 3600s
(**S27**), so every concurrent command fails with a message inviting the user to delete a live
lock file.

**Unverified and owed:** whether the harness's two long-running processes overlapped, and
whether anything besides `heal` can reach a backend's `remove` without the guard. The second
is the family question S24's fix has to answer, and it was not answered here.

**One feature request, recorded the same day: a folder of dotfiles that LiNix links into
place (XIII.21).** `link:` already places a file; what it cannot do is place thirty without
thirty lines that each name the same path twice. The proposal is a tree whose layout *is* the
declaration, and the write-up spends most of its length on what it must borrow rather than what
it adds — `resolve_target`, the ownership rule, the `extras_lock` teardown — because a folder
walker is exactly the kind of feature that arrives with its own second copy of all three. Four
decisions (**U22–U25**), three blocking, and **U23** is the one that decides whether it is
usable: a first sync on a fresh machine meets a home directory already full of files, and a
tree that half-links is worse than one that does not run. Phased as **7n**.

**Two more requests recorded the same day.** **XIII.22 — BSD**, which P7 implies and nothing
here had costed: `pkgin` is already registered, `pkg`/`pkg_add` are not, and the blocking half is
neither of those — **`when family` has no answer on a BSD**, so every `family ==` block silently
takes the else branch on a platform whose package manager LiNix already drives (**U26**). And the
dotfiles tree above (**XIII.21**, U22–U25).

**State of the tree at the end of this session:** `cargo build --all-targets` clean, `cargo test`
**970 passed / 0 failed**. Both journals were cleared afterwards (harness and real), because
`github:sharkdp/fd` sat `InProgress` in each and S24 arms on exactly that.

**The refactor this session was called for did not happen, on the owner's instruction** — the
decision reconciliation (84 entries across six registers, none of which record whether they have
been answered) is still owed, and is now larger by five.

## Session 2026-07-23 (thirteenth session) — three feature requests, answered in three ways

**No code changed. This session was scope.** Three features were proposed; they got three
different answers, and the difference is the useful part of the record.

- **A kernel-building engine: refused, and not filed as a K-item.** LiNix builds nothing — it
  drives managers that are already on the host. Reasoning in **XI.7**. Two smaller things
  were kept out of it and are in **XIII.1** (a DKMS-after-kernel-upgrade check, and a
  `hardware` command that suggests declarations and writes none).
- **A firewall backend: accepted as a proposal, written up as Part XI.** It fits because it is
  a new *backend*, not a new mechanism: `setting:`'s statement shape, `extras_lock`'s drift
  path and the guard all already exist. **XI.2 records the case against building it** — an
  nftables user can declare their perimeter today with `link:` plus `service:`, and the
  backend only earns its cost across several firewalls (N3). **Seven open decisions, three
  blocking**; N1 (is the perimeter exclusive) and N2 (refusing to close the port carrying the
  session) are the two that decide whether this is a feature or a lockout.
- **Hardware-backed secrets: half of it was already built and undocumented.** `link:` mode D
  (`link.rs:271-295`) has shipped age/sops decryption since Phase 2p and this document
  mentioned it only in a table of option keys. Now **Part XII**. Runtime injection into
  process memory is **ruled out** (owner, 2026-07-23) — XII.2 says why and says not to
  re-open it. Hardware identities (TPM, YubiKey) are in scope and are probably a change to
  what `@identity=` accepts rather than new crypto.

**Two live defects were found while documenting mode D, and are recorded, not fixed (rule 4):**
`backup_once` copies a previous secret to a world-readable `<target>.linix-backup` (**T1**), and
nothing stops `@target=` from writing the plaintext back inside the git-tracked config root
(**T2**). Both are in XII.5 **and are filed as open rows in VI.2**, because a decision that lives
only in a proposal part is a decision the bug ledger cannot see. **The secrets half is
documented for users in `readme.md`**; the firewall half is not, because it does not exist.

### Two principles, ruled the same day

**P7 — LiNix is not Linux-first, whatever the name says.** Windows and macOS are not ports and
not a later phase; a feature is unfinished until they have an equivalent or a written reason
there can be none. XIII.4 turns the rule into the actual gap list, which is shorter than it
sounds: `service:` already covers all five init systems, packages are covered everywhere, and
the two real holes are **`setting:` (GNOME only)** and **the snapshot safety net (Linux
filesystems only)** — the second being the quiet one, because `rebuild`'s revert and the guard's
pre-sync snapshot are written here as unqualified promises that silently do not hold on two of
three platforms (**U6**).

**P8 — LiNix does the thing; it does not hand you the thing to do.** Ruled while rejecting a
`hardware` command that would have printed declarations for the user to paste into a module.
Output whose next step is retyping has done the easy half. The correct shape is *ask, then do*
(`install`, `adopt`), never *inform, then leave*.

### Part XIII — seven proposals, five of them compositions of built things

Written from one conversation. Nine decisions (**U1–U9**), four blocking. The two findings that
matter most are not proposals at all but things already in the tree that nobody had written
down:

- **The onboarder is the plugin system, and it has been shipped all along.**
  `src/backends/onboarder.rs` (593 lines) teaches LiNix a whole package manager from
  `custom_backends.toml` — argv from TOML, output parser as a declarative `ParserSpec`. This
  document had mentioned it twice, in passing, and never described it. **Its real defect is
  location:** the file is read from the machine-local config dir, never from the config repo,
  so a repo that says `paru:yay-bin` works on the one machine where somebody hand-wrote a TOML
  file and fails on the fresh machine the repo exists to set up (**U1**).
- **LiNix already upgrades the kernel** — as a package, with no kernel awareness beyond the
  guard's protected prefixes. XIII.1 answers why out-of-tree modules still break: the
  distribution's DKMS hook fires for its own manager only, and LiNix's whole premise is
  several managers at once, so the cross-manager case is the one nothing covers.

The rest: `exec:`, the escape hatch (XIII.3); health-checked upgrades that revert on failure
(XIII.5, reusing K3's rulings wholesale); a preview for *what leaves if I deactivate this*
(XIII.6); cross-machine diff on `fleet`'s existing parse (XIII.7); and **collapsing the ten "is
my machine all right?" commands into one `linix check`** (XIII.8, **U9**) — the only item here
that breaks existing invocations, and under P2 that means the old names go in the same change
rather than being aliased.

### `exec:` — approved, and the first draft of it was ruled against

The draft proposed two new option keys, `@unless=<command>` and `@creates=<path>`. **The owner
ruled that the condition is `when` and there is no second condition system** — Part IX already
made `when`'s variables user-programmable (a `vars.sh` / `vars.py` provider is run by LiNix, is
handed the machine's facts, and returns name/value pairs), so *"unless this command succeeds"*
is `when $tpm_enrolled == no` with nothing added to the grammar. The condition also gains a
name, and a named condition can gate a package and a `setting:` and an `exec:` at once.

**And the state that `when` cannot supply is a lock keyed by the script's hash, recording how
many times that hash has run** (owner ruling). Content-addressed, so editing the script makes it
a different script that runs again, and renaming it does not. The default is once per distinct
content; `@runs=always` is the explicit, loud opt-out (**U13**). This is what lets `plan` print
the true sentence — *hash `a1b2…`, run 0 times, condition true → this will run* — which is the
test every statement in this model has to pass.

### Three more approved the same day, and a Phase to build them in

**XIII.9 — backends are software too.** LiNix cannot install the one class of software it
installs *with*: declare `brew:` on a fresh Mac and the line fails because Homebrew is not
there. That is the first ten minutes of every new machine, which is the ten minutes the tool
exists to delete. Refusal-first, never a silent fetch — installing a package manager is running
someone's script as root.

**XIII.10 — `sync --locked`.** The line between *describing* a machine and *reproducing* one.
Fails on a resolution that differs from the lock; changes nothing. `watch` should probably imply
it (**U11**).

**XIII.11 — `linix try`.** Phase 6's containers, pointed at the user's config instead of at
LiNix: rehearse the sync on a clean machine before it touches the real one. A dry run predicts
from LiNix's model; `try` finds out what the package manager actually does — which is the exact
gap the twelfth session's container run proved was invisible to `cargo test`.

### `exec:` has three states, and that is the one place this model bends

**Owner note, recorded because it is an exception someone will later "correct".** For every
other statement a false `when` and a deleted line are the same fact — an undeclared package is
drift and gets removed. **For a verb they are different**, and the example that forces it is the
one the feature was designed around: a script that enrols a TPM makes `$tpm_enrolled` true,
which turns its own `when` false. If false meant removed and removed meant undo, the sync that
enrolled would un-enrol on the way out. So: `when` true → run if the hash's count allows; `when`
false → **nothing runs and nothing is undone, and the lock row is kept** (dropping it would
re-run the script every time a flapping condition swung back); line deleted → `@undo=` if given,
and the row goes. **`exec:` must therefore stay out of `reconcile_extras`** — that ledger undoes
nouns, and wiring a verb into it reintroduces the un-enrol bug through the back door.

### Four more, approved the same day

- **XIII.12 — the onboarder is one field from user-defined nouns.** `name` is currently both the
  prefix and the executable. Split them and `firewall:22/tcp` works from six lines of TOML with
  no Rust. **Part XI stays** — a TOML definition names `ufw` and therefore means nothing on
  Windows, and *one spelling across five firewalls* is the half only a built-in backend can
  supply (P7). What the split changes is that the built-in becomes optional rather than urgent.
- **XIII.13 — hooks on LiNix's own events.** `after_sync`, `on_drift`, `on_guard_refusal`, in
  `preferences.toml`, context on stdin as JSON. Today every integration request — notify me,
  push the repo, open a ticket — has to become a LiNix feature. This is why.
- **XIII.14 — sharing, and it is BLOCKED on U14.** `use` takes a name, never a URL, so there is
  no way to consume anyone else's module. The rule-compatible answer is vendoring (`linix add
  <git-url>` copies files into your repo, once, reviewable in a diff) rather than importing.
  **Not scheduled**: once `exec:` exists, a vendored module can contain a verb, and the safety
  story has to be decided before the convenient half is built.
- **XIII.15 — `linix eval`.** Print the resolved desired state as versioned JSON. Not a feature
  for LiNix; the feature that stops LiNix needing a new one every time somebody wants to know
  something the resolver already computed.

### And five more, in three categories

**Approved and phased:** **XIII.19**, `git blame` for a declaration — *when did `openssl` enter
my config, in which commit* — which reads git and keeps no store of its own (7l); and
**XIII.20**, the exit-code table, settled in one place before the commands that need it exist:
0 converged, 1 LiNix failed, 2 differences found, **3 refused by the guard** (7m). Separating 3
is the point — a guard refusal is neither a crash nor a divergence, and a CI job that cannot
tell those apart will either retry a legitimate refusal or report a crash as healthy drift.

**Recorded as maybes, deliberately not scheduled:** **XIII.16**, grouped backends with
per-group priority (**U18**) — the right resolution order genuinely differs by *kind* of thing
(CLI tools want `cargo` first, system libraries want the distro and nothing else), one list
cannot say that, and the workaround of writing the prefix costs the portability a bare name
exists for. It is a maybe because it can break II.7 rule 5 from a new direction: two modules
resolving the same bare name through different groups puts two `ripgrep` binaries on one
`$PATH`, the failure `app/conflicts.rs` already exists to catch. And **XIII.18**, a language
server (**U20**) — the grammar's closed vocabularies are exactly what completion and hover want,
but it is only worth building as a thin front end over the same parser; a reimplementation is
the second implementation this rewrite exists to end.

**Recorded as a decision owed, gating work already scheduled:** **XIII.17** (**U19**) — *is
LiNix acting for a user or for the machine?* Today the answer is implicit (whoever ran the
command) and the Linux backends agree with it by accident. **7e ends that on its first line**,
because the registry's opening question is `HKCU` or `HKLM` and this document has nothing to
answer it with. Decide before writing it, or the adapter's guess becomes the convention and then
spreads to macOS `defaults`.

**The owner's instruction that day was that a specification which only *says* is worth
nothing.** So the approved items are also **Phase 7 in Part III**, in dependency order, each with
the one command that shows it is done: custom backends into the repo (7a), `exec:` (7b), backend
bootstrap (7c), `--locked` (7d), `setting:` on Windows (7e), health checks (7f), the DKMS rebuild
(7g), `try` (7h), and the command collapse last (7i) because everything above adds to what it
must report.

## Session 2026-07-23 (twelfth session) — V.57's last two pieces, and what running them found

The eleventh session left two things owed because they "need a Docker/WSL run this machine
could not do". They are built, and the run happened: **the coverage audit** and **the `tools`
image's real ecosystem lifecycle**. Everything below the first heading was found *by running
it*, and none of it was visible from reading the tree or from `cargo test`.

**The harness — `docker/integration/run-in-container.sh`, mirrored section for section in
`scripts/integration-windows.sh`.**

- **§14, the real multi-backend lifecycle.** A real install → `list` → PATH → remove → gone
  cycle for every manager the image ships, from a canary table — not just the distro's
  native one. Install failure is soft (a registry outage is not a LiNix bug); **everything
  after a successful install is hard.** A READY backend that cannot run a lifecycle here is
  named with its reason, and one with neither a canary nor a reason says so out loud.
- **§15, the plan-smoke.** Every registered backend the image cannot run gets its
  argv/planner wiring proven by a dry-run install, enumerated from `doctor --json` against a
  config whose `priority` lists all of them — V.15 refuses an unlisted backend, so the smoke
  config has to list every one. `service:`/`link:`/`setting:` are statements rather than
  package names, so they are smoked through `check` and a dry-run sync instead, and `btrfs`
  through `snapshot`.
- **§16, the command surface executed.** Every subcommand is *run*, not `--help`'d;
  `<cmd> --help` is ledgered separately and does not satisfy the audit. `bundle` → `restore`
  is round-tripped in both directions.
- **§17, the coverage audit.** Hard-fails on any backend or subcommand that no lifecycle and
  no plan-smoke touched, outside an exempt set that is printed **with a reason for each**
  (`shell`, `undo`, `history`, `bisect`, `fleet`; a SMOKE run names `rollback`, `diff` and
  `run` too). It caught `nix` on its first run, which is the point: a fixed list of checks
  cannot notice what is missing from it.
- **Part IV's named proofs are numbers now.** The `purge-unmanaged` ratio check runs
  **before** `adopt` — the only state in which it tests anything — and each refusal asserts
  *which* rule refused, so the ratio and the protected set cannot pass on each other's
  behalf. `adopt` is counted from `modules/adopted.txt` against the manager's own
  user-chosen list and against every installed package: the old check read `linix list`,
  which answers "what is installed" and which `adopt` does not change, so its before/after
  numbers were identical by construction.

**Measured, 2026-07-23, WSL2 + Docker 29.6.1, full transcript in `docker_log_7_23.txt`:**

| image | result | real lifecycles |
|---|---|---|
| ubuntu (apt) | **271 / 0 / 5 soft** | apt, cargo, gem, github, npm, pipx, uv |
| fedora (dnf) | **279 / 0 / 5** | dnf + the same language managers |
| arch (pacman) | **271 / 0 / 5** | pacman + the same |
| alpine (apk) | **266 / 0 / 3** | apk + the same |
| **tools (apt)** | **316 / 2 / 14** | **18** — apt, bun, cargo, composer, conda, dotnet, emacs, gem, github, go, krew, luarocks, mise, npm, pipx, pixi, pub, uv |
| gentoo (emerge) | **214 / 0 / 9** | none — SMOKE_ONLY, and it says so |

The comparison that matters is not the totals but what they are made of: the previous
record for ubuntu was **82**, and **24 of those were `<cmd> --help`**. Hermetic gates ran
alongside: `cargo test` **967 on Linux / 970 on Windows, 0 failures**, `cargo clippy
--all-targets` silent, on both platforms.

`tools`' two failures are one finding and it is left red on purpose: **`mise` installs and
its binary lands on PATH, but `linix list --backend mise` does not report it and the removal
therefore leaves it behind.** `mise list --json` in the same image reports the tool
correctly and LiNix's parser reads that shape, with or without the option terminator — so
the fault is somewhere between them and is not yet explained. It is a real defect found by
a real run, and softening it would be the vacuous check IV.1 bans.

**What the run found.**

- **The Linux build did not compile at all.** `registry.rs` used `OrphanDryRun` without
  importing it, inside the `#[cfg(target_os = "linux")]` apt block — so a Windows-only
  session could not see it, and every container image failed at `cargo build`.
- **A refused `install` wedged the config.** `install` writes the line and syncs after it
  (S15), and nothing checked the backend before the write: `linix install dnf:jq` on a host
  without dnf left `dnf:jq` in `modules/imperative.txt`, and from that moment `status`,
  `plan`, `check`, `why`, `upgrade`, `conflicts`, `activate` and **every later install** were
  a hard parse error until a human edited the file. `App::declare` refuses such a line before
  writing, which covers every landing and `absent:`/`repo:` with it; `retarget` (`teleport`)
  had the identical fault one file over and is refused the same way. A name nothing can
  resolve is still written and then withdrawn — that is a failed install, not an unusable line.
- **`gem` has been unable to install anything since V.62.** `core/argv.rs` listed `gem` as a
  manager that ends its options at `--`. RubyGems' `--` is not an option terminator: it is the
  separator before the **build arguments** for a C extension, so `gem install -- colorize`
  names no gem and dies with *"Please specify at least one gem name"*. The module's own
  header says a manager joins that table only "when someone has checked its argument parser";
  this one had not been. Moved, with the reason recorded and a test on both verbs.
- **A failed install by one manager failed every manager after it.** Every later install
  syncs the whole model, so the line `gem` could not install was retried by each subsequent
  backend and nine lifecycles reported *gem's* stack trace under their own names. That is the
  designed behaviour for a pinned name (V.7c), so the harness clears the line, as its own
  negative-path section already did.
- **`krew` was READY on a host without krew.** Its probe asked for `kubectl`, and krew is a
  *plugin* — `kubectl krew …` works only because krew installs `kubectl-krew`. So every krew
  command failed with `unknown command "krew"`, and it took `linix update` down with it.
- **One backend could cancel every backend after it.** `App::update` and `App::upgrade` swept
  the registry with `?`, so the first manager that could not refresh or upgrade silently
  skipped the rest and the ones that had succeeded went unmentioned. Each failure is named
  now and the sweep finishes.
- **scoop's `list` counted a failed install as an installed package.** scoop keeps the row
  forever with an empty Version and Source and `Install failed` in Info; read by splitting on
  whitespace, that is a package named `jq` at version `2026-07-21` — so `sync` believed there
  was nothing to do, `adopt` would write it into a manifest, and no `jq` was ever on PATH.
  It is sliced by header offsets now, sharing one `slice_fixed_table` with the winget parser
  that had already learned this exact lesson — and `scoop search` was moved onto it too.
- **`cargo test` wrote into the repository.** Three test helpers fell back to `"."` when
  neither `TMP` nor `TMPDIR` was set, which is every plain Linux shell — so a Linux
  `cargo test` left `linix-embedded-*.linix`, `linix-marker-*` and `linix-vars-test-*/` in
  the working tree. All three use `std::env::temp_dir()` now.

**Reported, not changed — they are the owner's call.**

- **A helm plugin LiNix installs, LiNix cannot remove.** `helm plugin install` takes a URL
  and `helm plugin uninstall` takes the plugin **name**; a declaration carries one name, so
  the removal goes out as `helm plugin uninstall https://github.com/databus23/helm-diff` and
  helm answers `Plugin: <url> not found`. Proven by a real run on the `tools` image — the
  install passed and the removal failed — and it is worse than a failed command: the
  registry then reports the plugin as drift on every later sync, and each one fails
  identically, so one helm plugin wedges every subsequent operation. It needs a decision
  about how a helm plugin is identified, not a guessed URL→name mapping, so `helm` is
  plan-smoked with that sentence as its reason rather than left as a permanently red row.
- **`bun`'s own `remove -g` keeps the launcher.** Reproduced against bun directly with no
  LiNix involved: the package leaves the lockfile and `cowsay.exe`/`cowsay.bunx` stay on
  PATH. The harness reports it every run rather than tolerating it, and only when it
  actually happens, so a bun that starts cleaning up still has to pass the strict check.
  Whether LiNix should delete another manager's leftovers is a decision, not a bug fix.
- **A flake reference cannot be written in a manifest.** `nix.rs` supports `nixpkgs#hello`,
  and `#` opens a comment in the one grammar — so the branch is unreachable from a file and
  the validator rejects the name besides. The harness uses `nix:hello`, which the backend
  turns into `nixpkgs#hello` itself.
- **Two commands hung on Windows, in different backends, and I could not say why.**
  `linix -y uninstall gem:colorize` ran eight minutes with no child process and no output,
  on a host where `gem uninstall colorize` typed directly finishes instantly; then
  `linix -y install github:sharkdp/fd` ran fifteen minutes on the same host, while the same
  spec had completed normally in the ubuntu container minutes earlier. `scoop`, `bun` and
  `cargo` lifecycles in the same run were fine. Both were killed rather than diagnosed, and
  neither reproduced under a single command by hand — so what is recorded is the shape:
  **on Windows a sync-path command can stop returning**, and `network_timeout_secs` did not
  bound the second one. The *harness* fault it exposed is fixed either way: the Windows
  sweep drove the binary with no timeout at all, so a wedged command stopped the whole run
  indefinitely and reported nothing. Every call is wrapped now, the way the container one
  already was — which is what turns this from a hang into a named failure next time.
- **`pip`'s real lifecycle cannot pass on a PEP 668 distro.** The harness detects the
  `EXTERNALLY-MANAGED` marker and names that as the reason rather than letting a permanent,
  expected refusal read as ecosystem flakiness run after run.

**Still owed.** The Windows sweep is written and its audit passes, but no clean end-to-end
run of it exists: the two hangs above ate it twice, and both interventions contaminated the
rows after them. It needs one uninterrupted run on a Windows host now that every call is
wrapped in a timeout. The `mise` failure above needs an explanation, and `helm` needs a
ruling before it can leave the plan-smoke list.

## Session 2026-07-22 (eleventh session) — the tenth session's rulings, built

Everything the tenth session **RULED and OWED** is now built, verified against the tree at the
commit that last touched this section: `cargo build --all-targets` clean, `cargo clippy
--all-targets` silent, **960 tests, 0 failures**.

- **V.62 — a name is data.** The grammar refuses a leading `-` wherever a name appears (package,
  subtraction target, shim/service/link/setting/schedule name, repo spec). Every manager
  invocation that honours `--` ends its options before the names, from one table
  (`core/argv.rs`); flags that must trail a name (cargo `--version`, gem-style pins) move ahead
  of the terminator. `validate_command`'s uncalled denylist is **deleted**; `validate_path` is
  wired on the `vars` `read_file`, and `validate_package_name_for` now runs on every removal
  target — including one out of `registry.json` that never saw the grammar.
- **V.61 — one writer.** A cross-process lock on the data directory (`core/datalock.rs`), held
  for the whole run, naming its holder when contended. A `hook-reconcile` spawned by a manager
  LiNix is driving stands down rather than deadlock on its parent's lock.
- **V.60 — a restore that cannot restore says so.** `RestoreCapability`; btrfs refuses a live
  root rollback and explains why. `undo`'s second restore implementation is gone — one lives in
  the provider, and `undo` calls it.
- **V.56 — a removal is a list of names; remove ≠ purge.** `remove-orphans` enumerates via a
  dry run or loses the capability by name; the native-verb path and `clean_orphans` are deleted.
  apt's `remove_args` is no longer `purge`; purge is a separate capability (`[remove] purge`,
  `uninstall --purge`), off by default and machine-wide.
- **V.55 — a `vars` provider goes through the ledger.** `vars.linix` and external `vars.<ext>`
  are hashed under a `vars:<file>` id and refused if unapproved or changed; `linix lock`
  approves. The line file executes nothing and is not hashed.
- **V.59 — `restore DIR`.** The other half of `bundle`, a command not a README; refuses a
  non-empty config unless `--force`; end-to-end proof runs without git.
- **teleport PKG BACKEND** — built (owner-ruled); rewrites the line in place, syncs, no second
  copy left behind.
- **V.58 — `0.1.0`, `SYKhayyat/LiNix` everywhere, `migrate`→`adopt` in the install scripts,
  `full-test.ps1`/`verify.ps1` deleted.** **Still owed:** the branch is not pushed — the first
  push is what makes IV.2's CI real and fires the release job, and it is the owner's to make.
- **V.57 — partial.** `FAST` (declared, never read) is deleted everywhere; CI now runs the fast
  container matrix (ubuntu/alpine/arch) on every push and PR, tools/gentoo on dispatch. **Still
  owed:** the coverage audit that hard-fails on an untested `[READY]` backend, and the `tools`
  image's real ecosystem lifecycle assertions — both need a Docker/WSL run this machine could
  not do.

## Session 2026-07-22 (tenth session) — a readiness audit, and what it found

A full-tree audit against the question *"can someone else run this on a machine they cannot
rebuild?"* The hermetic gates were green when it started and still are — `cargo build
--all-targets` clean, `cargo clippy --all-targets` silent, **915 tests, 0 failures** (847 lib
+ 17 `main` + 51 across eleven integration binaries), and **16 `.unwrap()` in non-test code**,
each checked and structurally infallible.

### RULED and OWED (owner, 2026-07-22): a `vars` provider goes through the hook ledger

**Ruled, not yet built.** Reasoned in **V.55**; the rule is in **II.6b** and **II.12**.
`vars.linix` was given `sh`/`read_file`/`env`/`http_get` on the stated grounds that it is
"trusted the same as a hook", and no hash was ever recorded — so the trust boundary was the
sentence and not the mechanism, on a file that runs at step 0 of II.7 (before the plan, and so
on `status`, `plan` and `plan --dry-run`) and that `watch --pull` will execute unattended from a
pulled repo before `verify_all_approved` can fire. The external `vars.py`/`vars.js` path has it
identically and is fixed in the same change (*options offered: the ledger, or stripping the
standard library; the owner ruled the ledger, because stripping `sh` moves people to the
external provider rather than closing anything*).

### RULED and OWED (owner, 2026-07-22): a removal is a list of names, and `remove` is not `purge`

**Ruled, not yet built.** Reasoned in **V.56**; the rules are in **II.10** and **II.11c**, and
the Phase 5 entry that offered the choice now records that it was taken. `remove-orphans` ran
`apt autoremove -y` for backends that cannot enumerate, and the one thing standing there — a
printed warning that those removals could not be previewed or checked against the protected
list — was printed *by the confirmation*, which `--yes` skips. The native-verb branch is deleted
rather than gated: where a dry run can produce the list it becomes an ordinary enumerated
removal, and where it cannot the backend loses the capability by name. In the same change, apt's
`remove_args` stops being `["purge", "-y"]` — drift removal was destroying `/etc` configuration
for every package whose line someone deleted — with purge available as `--purge` and
`[remove] purge`.

### RULED and OWED (owner, 2026-07-22): the harness tells the truth, and CI runs it

**Ruled, not yet built.** Reasoned in **V.57**; the rules are in **IV.1** and **IV.2**. Three
things, all yes:

- **The `tools` image gets the ecosystem lifecycle it already advertises** — a real
  install → list → remove against composer, opam, luarocks, nimble, cabal, stack, mix, helm,
  krew, pixi, spack, go, dotnet and pub — plus the **coverage audit** the README describes and
  the tree does not contain, which hard-fails on any `[READY]` backend or subcommand that no
  real lifecycle and no plan-smoke touched. Until then those backends are proven only against
  mocks, which is where format drift is invisible by construction.
- **CI runs the hermetic gates and ubuntu + alpine + arch on every change**; `tools` and
  `gentoo` on manual dispatch.
- **The rest is bug-fixing, not choice:** `FAST` read or deleted (`SMOKE_ONLY`'s sibling, left
  live in the same file); the three checks that cannot fail; `full-test.ps1` and `verify.ps1`
  **deleted** — pre-v7, they call the nonexistent `linix backends` and `full-test.ps1` has one
  `exit 1` in the whole file (NO LEGACY); and Part IV's three unproven named proofs turned into
  real assertions, with the `purge-unmanaged` ratio check moved **before** `adopt`, which is the
  only state in which it tests anything.

**Measured this session, and the record is honest:** ubuntu **82/0/0** and the live Windows scoop
sweep **61/0/0**, re-run against this tree and matching session 9 exactly. What that number does
*not* say is how much of it is `--help`: **24 of ubuntu's 82 and 23 of Windows' 61** are
`<cmd> --help` exit checks, which prove clap is wired and nothing else.

### RULED and OWED (owner, 2026-07-22): `0.1.0`, and an install path that works

**Ruled, not yet built.** Reasoned in **V.58**; the rule is in **II.18**.

- **`0.1.0`.** Nothing has been released, so `6.0.0` was a count of internal rewrites answering a
  question about what a user has. The rewrite keeps the name **v7** in Part VII and the
  CHANGELOG; that is a codename, not a version.
- **The repo is `github.com/SYKhayyat/LiNix`** — renamed, and confirmed resolving. The local
  `origin` still says `Nexus` (GitHub redirects, so nothing broke loudly) and both install
  scripts still say `OWNER/linix`, which never resolved at all.
- **`migrate` → `adopt` in both install scripts.** II.17 has listed `migrate` as deleted since
  the rename; the scripts kept calling it, so the documented first run failed at the adopt step.
- **The branch is 219 commits ahead of the remote.** Pushing is what makes IV.2's CI real, and
  what lets the tag-triggered release job fire for the first time.

### RULED (owner, 2026-07-22): K9 is answered — `bundle`, finished

**Ruled, not yet built.** Reasoned in **V.59**; the rule is in **II.8**, and X.5 and the K9
register entry are updated. `bundle` packs everything a backup needs and stops at a `RESTORE.md`,
so the restore path was prose that nothing had ever executed — while X.5 makes a git-less machine
a supported machine, and on such a machine `bundle` is the *only* way a config leaves at all.
K9's own constraint (**not a second archive writer**) decided the shape: finish the command that
exists. `restore DIR`, refusing a non-empty config directory unless told otherwise, with an
end-to-end test that runs **without git** — bundle, restore into a clean directory, assert the
model parses and resolves to the same package set.

### DIRECTED (owner, 2026-07-22): what reaches a command line, and `teleport`

**Not yet built.** Reasoned in **V.62**; the rule is in **II.12b**.

- **No backend terminates its arguments.** A package name is constrained to "one word" and a
  leading `-` is refused only in the `Subtract` position at line start, so `apt:--allow-downgrades`
  parses as an ordinary package and reaches the manager as an option. ~30 call sites
  (`generic.rs` install/remove, brew, snap, flatpak, nix, conda, krew, mise, setting, service,
  vscode), about half under sudo; `conda` reaches a `preferences.toml` value. **`fleet.rs`
  already does it correctly** — rejects the dash, emits `--` — and is the only one that does.
- **`Validator::validate_command`/`validate_path` have zero callers.** The `rm -rf /` / `mkfs`
  denylist, `TRUSTED_BIN_PATHS` and `FORBIDDEN_PATHS` (`/etc/shadow`, SAM) enforce nothing;
  their tests pass. Wire each on the path it names or delete it — deliberately, per check.
  `FORBIDDEN_PATHS` is also duplicated in `undo.rs`.
- **`teleport PKG BACKEND` is in II.8's command table and is not a subcommand.** Confirmed
  against the binary: `error: unrecognized subcommand 'teleport'`. **Owner ruled 2026-07-22 that
  it is to be built**, not struck from the table — moving a package between managers is a real
  want, and it is an edit-the-line-then-sync command like every other, so it introduces no new
  mechanism. Until it exists the table describes a command that is not there, which is the
  Part VIII/X header fault at the level of a single row.

### DIRECTED (owner, 2026-07-22): the two that needed no ruling, only fixing

**Not yet built.** Reasoned in **V.60** and **V.61**; the rules are in **II.13** and **II.8**.

- **btrfs restore never restored anything.** `btrfs subvolume snapshot <snap> /` creates a
  nested subvolume and exits 0; a live root rollback needs a subvolume swap and cannot be done
  over a mounted `/`. Every recovery path believed it — including `purge-unmanaged`'s *"Snapshot
  taken. That is your undo."* The duplicate in `undo.rs` has the same bug, prints *"SUCCESS:
  System root has been restored."*, and handles only btrfs and Timeshift, so ZFS and Windows
  restored nothing at all. **One implementation survives, and a provider that cannot restore
  refuses and says so in `doctor` and before the change.**
- **No cross-process lock on the data directory.** `fs2` was used at one site, around a
  subprocess, never around state. The second writer is `apt`, via the `DPkg::Post-Invoke` hook
  LiNix installs — so the race needs no second LiNix user, and the entry that loses a
  last-one-wins whole-file write becomes drift, which is a removal.

### Corrected in this audit, without a ruling needed

The document described itself wrongly in four places, each in the direction that makes it harder
to trust:

- **Part VIII said "Not built. Not in Part II."** It is essentially fully built and it *is* in
  Part II (V.48). **Part X said the same**; four of its six sections are built. A banner like
  that is read as capability that is absent, or as absence by someone about to build it twice.
- **The 2026-07-17 security block said "nothing in the deferred set was touched"** while the head
  of the document said SEC1–SEC6 are built. Both SEC1/SEC2 (2026-07-19) and SEC3's confirmation
  (eighth session) had landed.
- **II.10 named `clean`** — deleted and split into `remove-orphans` and `clean-cache` — as a
  removal path that calls the guard, and II.8's command table still listed it.
- **Two Part VII entries describe live bugs in code that no longer exists** (`linix shim
  --source`, `confirm_destructive`), and session 9 cites the deletion of `confirm_destructive`
  as the *cause* of a real bug two thousand lines from an entry calling it live.

### The runs, re-executed against this tree

Not recalled — re-run during the audit, full matrix and no `FAST`:

| image | hard pass | fail | soft |
|---|---|---|---|
| ubuntu (apt) | 82 | 0 | 0 |
| fedora (dnf) | 82 | 0 | 0 |
| arch (pacman) | 82 | 0 | 0 |
| alpine (apk) | 80 | 0 | 0 |
| tools (apt) | 82 | 0 | 0 |
| gentoo (emerge) | 59 | 0 | 6 named |
| Windows scoop (native) | 61 | 0 | 0 |

Session 9's record is accurate. **`tools` scoring identically to `ubuntu` is the evidence for
IV.1's last rule**, not a coincidence: it runs the same apt lifecycle and none of the ecosystem
managers its Dockerfile advertises.

## Session 2026-07-22 (ninth session) — Phase 6 actually ran, and the lock got a shape

**The containers had never been run. They have now**, on real Docker through WSL, doing real
installs and removals. That is where everything below came from: none of it was visible from a
green `cargo test` on Windows, which is rule 11 in one line.

**Every image green: Ubuntu, Fedora, Arch and `tools` at 79 hard checks each, Alpine at 77,
0 failures anywhere. The live Windows scoop sweep: 58, 0 failures.** First time green on any
of them, and three only went green because of bugs found *by* the run — recorded below.
(`gentoo` is opt-in; it ran later the same day — see §8. Two image builds failed transiently
on the registry and passed on retry, which is worth knowing before reading a red result as a
defect.)

### 1. Backend chains (II.7b) — owner ruling, 2026-07-22

The prefix on a package line is now a list in preference order: `apt:rg` pins, `apt,dnf:rg`
tries two, `apt,list:rg` tries apt then the rest of `priority`, and `list:rg` is what a bare
name has always meant, spelled out. Comma rather than hyphen because `nix-env` and `apt-get`
are real manager names and a separator a name can contain stops working the day one is added.
`list` is reserved and must come last; a repeated name, an empty slot, an unknown link, and a
pattern spread over a chain are each errors.

**The problem it solves.** A line had two settings — one manager forever, or a bare name whose
answer got frozen to whichever machine synced first — and neither is what someone with two
machines means. Wanting apt's ripgrep here does not mean wanting nothing on the Fedora box.

### 2. The bare-name lock is per host (II.6, V.16) — owner ruling, 2026-07-22

`locks/bare.toml` → `locks/bare.HOST.toml`. `locks/` travels with the config but *which manager
has ripgrep* is a fact about a machine, so one shared file had the two boxes overwriting each
other's answer on every sync — churn in a tracked file, a conflict every time. Each machine now
writes only its own file; all of them commit cleanly; each still reproduces exactly.

A lock is honoured only when the line still accepts the manager **and this machine has it**;
otherwise it warns and asks again. The lock exists to stop an unedited line changing meaning,
which is not the same as insisting on a manager that is not here — insisting is what a pin is
for. Proven in the container: a `bare.some-other-box.toml` dropped into `locks/` is ignored and
left untouched.

### 3. `linix unlock [NAME…] [--list]` — owner ruling, 2026-07-22

Forgets what an unpinned name resolved to, so the next sync asks again. For when a better
source appears: `ripgrep` on cargo because apt did not carry it yet moves to apt once it does,
**and that sync uninstalls the cargo copy** — the old one is a managed package nothing declares
any more, which is what drift removal already does. No new mechanism, and no second copy left
behind.

### 4. Bugs the containers found

- **dnf was invisible on Fedora 41+, and every unpinned name skipped it.** `parse_dnf_search`
  read dnf4's `name.arch : summary`; dnf5 prints `name.arch<TAB>summary` with `Matched fields:`
  headers between the rows. The parser matched nothing, so `linix search jq` returned **zero**
  dnf rows on a box where `dnf list --available jq` lists it — and bare `jq` fell through the
  whole priority list to `cargo`, whose `jq` is a **library** crate with no binaries, so the
  install failed. One pass now reads either separator. Also fixed alongside: the name was cut
  at the first `.`, which renamed `python3.12.x86_64` to `python3` and made it unmatchable.
- **`cargo` claims any crate with a matching name, including libraries it cannot install.**
  Not fixed — `cargo search` cannot say whether a crate ships a binary without the registry
  API. With dnf visible again the system manager wins on Fedora and this stops being reachable
  there, but a bare name whose only holder is a cargo library still fails at install time
  rather than at resolution. **Owed.**
- **The harness lied about removals for nine sections.** `command -v` answers from the shell's
  hash table, so a package looked up in section 4 still "existed" in section 9 after apt had
  deleted the file. Every PATH assertion goes through a fresh `sh` now. This was the last
  Ubuntu failure and it was never LiNix's bug — worth recording because a check that cannot
  fail is worse than no check. The same check, plus its twin (`sh -c "lx …"`, which cannot see
  a shell function and so ran nothing at all), was live in `scripts/integration-windows.sh`
  and is fixed there too.
- **`adopt` wrote a file LiNix could not parse, and it wedged the config.** Found by the live
  Windows sweep: `winget list` reports Add/Remove-Programs entries as `ARP\Machine\X64\Android
  Studio`, and a package name is one word, so `modules/adopted.txt:69` was a parse error in a
  file LiNix had just generated — `rollback` and every other command that reads the model died
  on it. `adopt` now holds such names back and reports them in the commented section instead
  of writing them. That left a second hole, so the guard closed it: a name no line can hold is
  **protected** (V.7b), because it can never be adopted and would otherwise be a permanent
  `purge-unmanaged` candidate that adopt could not clear.

### 5. The families behind those bugs

Owner added *"fix the whole family, not one instance"* to `CLAUDE.md` mid-session. Applied
backwards to the four fixes above, the siblings were live:

**A manager that cannot answer is read as a manager that said no.** That is what made the dnf
bug invisible — and it is the shape of the whole class, because a parser that returns zero
looks exactly like a repository that has nothing. Three more did it:

- **zypper** — `skip_while(|l| !l.contains("---"))` consumed the *entire* output when no header
  rule was printed. This parser is zypper's **installed lister** as well as its search, so
  zero is not a bad search result: it is *"nothing is installed"*, which is the input to a mass
  removal.
- **apk search** used the whitespace splitter, keeping `jq-1.7.1-r0` as the name, so a result
  could never equal the name asked for. apk was invisible to every unpinned line, exactly as
  dnf was.
- **choco** — `list -lo` is an error on Chocolatey 2.x, so the command failed and its empty
  output read as an empty installed set.

**A name truncated at a delimiter that occurs inside real names.** `parse_dash_version_list`
turned `xz-libs-dev` into `xz`, and it is apk's installed lister — a package recorded under
the wrong name can never be matched by `info()`, so `remove` silently did nothing. `nix-env`
parsing had the same missing guard; both use the digit check `xbps` and `pkgsrc` already had.
`web.rs` cut a filename at the first `.` and installed `ripgrep-14.1.0-x86_64.tar.gz` as a
binary named `ripgrep-14`.

**`adopt` was one caller of a general hole.** `Editor::add` never validated the line at all:
`key_of` parsed, returned `None` on an error, and the line was appended anyway. The second
caller is the pm-hook, which takes its target off a real `choco install "Google Chrome"`
command line and writes it **behind your back** — so the file would break with nobody having
typed anything. `add` refuses first now, and the module-file rule is *shared* with the reader
(`set_math_in_a_module`) rather than copied, because a writer and a reader with two copies of
one rule is how a generated file comes to be unreadable.

**Checked and not affected**, so the sweep is honest about its edges: pacman, xbps, pkgsrc,
brew, winget's column slicing, dotnet, conda and every JSON path (no literal separator, or
already digit-guarded); `export`/`bundle`/`sbom`/`unmanage`/`profile save` (not LiNix grammar,
or built from lines that already parsed). `fix.ps1` was **deleted** — it overwrote
`src/backends/apk.rs` with a pre-v7 `PackageManager` implementation carrying the un-fixed dash
bug, so running it reintroduced a fixed bug (NO LEGACY).

### 6. Still owed from this session

- The opt-in `gentoo` image. **Ran** — see §8 below.
- **A config written by an older build stops every command, and the error does not say which
  file.** `confirm_destructive` was deleted in the rewrite (V.23), so a `preferences.toml`
  still carrying it is refused — correctly, since silently ignoring a setting someone wrote is
  worse. But the bare TOML error says `line 17` of nothing in particular. It names the path
  now. Found on the owner's own machine, where the current binary would not run at all.
- **A manager that cannot answer is read as a manager that said no.** **Ruled and built** —
  see §7 below.

### 7. RULED and BUILT (owner, 2026-07-22): silence falls through, but is never recorded

*(Options offered: hard-stop on "could not tell", or fall through. The owner ruled the third
thing neither option said — fall through, **and do not write the lock** — plus "leave the
cargo gap, but make the failure and cause clear", and asked the question that decides whether
the ruling is worth anything: **"then, on the next sync, it will be changed — will it even?"*
The answer is yes, and it is the `unlock` migration doing it: the guessed copy is a managed
package that nothing declares once the real manager is back, and "a managed package nothing
declares is drift, and removing it is what sync is".)*

Reasoned in **V.7c**; the rule is in **II.7b**. What changed:

- **`CommandExecutor::search_output`** — a read whose emptiness is an *answer*, so a command
  that could not produce one errors instead of returning nothing. Non-zero exit alone is not
  the signal (`pacman -Ss` and `dnf search` exit non-zero on an ordinary miss); a non-zero
  exit **with a complaint on stderr** is. Every backend's `search` uses it — the generic one
  (apt, apk, zypper, choco, scoop, winget) plus brew, cargo, conda, dnf, flatpak, krew, mise,
  nix, pacman, snap, xbps, emacs and psresource. The registry-backed ones (npm, pnpm, yarn,
  pip, vscode) already failed loudly on an HTTP error.
- **`Verdict::{Has, Lacks, CouldNotTell}`** in the resolver, replacing a `bool`. A manager
  this machine does not have, and one with no search facility, are still a plain `Lacks`:
  settled facts, and re-asking gets the same answer forever.
- **The lock records only an unanimous no.** A pick made past silence resolves, warns naming
  the silent manager and what it said, and writes nothing.
- **"No package manager has it" now says which ones could not answer**, because a stale index
  and a misspelling look identical from outside and only one is fixed by editing the line.
- **The cargo gap is explained rather than closed**, as ruled: `cargo install` failing on a
  library crate now says crates.io has the name but ships no program, **and that
  `cargo search` cannot tell the two apart** — so a name can reach cargo and install
  nothing. Worded to be true of a pinned `cargo:jq` as well as a resolved one.

**Two siblings found by the same sweep, in the same family** (a command that could not do its
job read as one that did): `psresource` ran **install, uninstall and upgrade** through
`run_output`, which discards the exit status — a failed `Install-PSResource` reported success.
`emacs` did the same for its archive refresh. Both now use the checked path. **Checked and not
affected:** every other backend's install/remove already goes through `run`/`run_exclusive`,
which enforce status; `list_installed` still tolerates a failed read, because an empty
installed list can only cause a reinstall attempt, never a removal (removals come from managed
state, not from listing).

*Verified against the binary and the harnesses:* the live Windows scoop sweep at **61/0** (58
before these three checks), and **every container image green with the new check in it** —
ubuntu, arch and tools at 82, fedora and alpine likewise passing, no failures anywhere. The
check stages the fault rather than waiting for one: it shadows **cargo's `search` only**, so
exactly one candidate in the chain goes silent while the manager under test is untouched
(breaking the network would break both), then asserts the package still resolves, that the
lock stays empty, and that the plan names which manager could not answer. Both the
fall-through warning and the nothing-found error were also read back off a real run before
being worded — the first draft of each said the same phrase twice.

**One gap left knowingly, and not hidden:** `apt-cache search` with an empty index exits zero
and prints nothing, indistinguishable from a real miss. No generic signal remains to read;
closing it needs a per-manager index-health check. The `tools` image works around it with a
final `apt-get update`.

### 8. The gentoo image ran, and it had never been able to

The last item on the owed list. It had **three** reasons it could not have passed, none of
which any other image could reach:

- **`SMOKE_ONLY` was declared in three places and read in none.** The Dockerfile bakes
  `ENV SMOKE_ONLY=1`, `run.sh` forwards it, and the harness never looked at it — so the run
  would have driven a full install→remove lifecycle through Portage and built jq from source,
  which is exactly what the image's own header says it must not do. The harness honours it
  now: everything that does not mutate the machine still runs (the grammar, the planner, the
  guard's refusals, every read verb), each skipped check is **named**, and the closing banner
  says which run it was — *"OK" over a third of the checks, printed the same way, is how a
  narrower sweep gets mistaken for a full one.* Proved both directions on ubuntu before
  gentoo was touched: **60 hard checks with 6 named skips under `SMOKE_ONLY`, 82 and no skips
  without it.**
- **Two names for the binary.** The image set `ENV LINIX=…`; the harness read `LINIX_BIN`,
  which nothing sets. gentoo was also the only image not to put the binary on PATH. Both
  halves deleted rather than reconciled: the harness now reads `LINIX` (what the Windows
  script and `release-check.sh` already use) and the image installs to `/usr/local/bin/linix`
  like the other five. The `FATAL: not runnable in this image` guard caught it on the first
  run, which is what it was added for.
- **A live bug, and the reason this image was worth running: X.5's audit was wrong.** See
  X.5 — `is_repo()` guards the *directory* question, not the *git* question, so on gentoo's
  git-less stage3 base `linix git log` printed an empty history and `git status` advised an
  `init` that could only refuse. `GitManager::require()` is now asked by every history verb.
  The harness asserts the refusal rather than skipping the section, so the one image that can
  reach this path also tests it.

*Ruled while reading the failure:* the fix is **not** to install git into the image. A
git-less machine is a supported machine (X.5), so the honest thing is for the sweep to have
one — and for the harness to assert the refusal there rather than skip past it.

**Result: gentoo (emerge) PASS — 59 hard checks, 0 failures, 6 named soft skips**, and
ubuntu re-run alongside it at 82/0/0 to show the gating changed nothing where nothing was
gated. The soft six are the whole of what Portage's source builds cost: install, remove,
rebuild+K14, the per-host lock, the sync past a silent manager, and history. Everything
declarative — the grammar, the chain rules, the guard's refusals, adopt, the planner, the
resolver's silence rule, the full command surface — ran for real against emerge.

## Session 2026-07-21 (eighth session) — the owed list, continued

Green at each commit (`cargo build --all-targets` clean, `cargo clippy --all-targets` silent,
`cargo test` all suites — 809 lib tests plus the integration suites).

**1. SEC3's confirmation half is built.** The last undecided-looking item on the owed list was
not undecided at all: the owner ruled on 2026-07-19 that `@target` stays unconfined and that a
destination outside the home directory asks once. Only the asking was missing, so a `link:` line
naming `/etc/cron.d/x` was placed with no beat between the pasted line and the system path. The
question is asked in `reconcile`, **before the repo phase and before any package is touched** — a
confirmation offered after the file is placed is a notification. Details and the
`--dry-run`/`--yes`/non-interactive rules are recorded under SEC3 in Phase 5.

**2. II.13's signature check is built, and LiNix commits as you.** Two rulings were needed and
both were the recommendation *(options offered: your git identity, keep `linix@localhost`, or
you-as-author/LiNix-as-committer; and: show-and-refuse-if-asked, show-only, or always-refuse)*:

- **The identity override is gone.** `core/git.rs` set `user.name=linix` /
  `user.email=linix@localhost` on every call, so a signed commit would have carried your
  signature and a fake author. Nothing is injected now — not the identity, not the signing
  flags. A repo with no identity is git's error, and `commit_all` adds the one sentence git
  cannot: LiNix commits as you, so git needs to know who that is.
- **Git answers, LiNix repeats it.** `git log` and the `history` browser show each commit's
  signature and signer; `Signature::Good` is kept apart from a signature by an untrusted,
  expired or revoked key, because collapsing them is V.32's whole complaint. `rollback` refuses
  a commit git will not vouch for **only when `[guard] require_signed_history` is on** — off by
  default, since a fresh repo signs nothing. Reasoned in V.32b.

*Verified against the binary:* a fresh `linix git init` + commit is authored by this machine's
real git identity (it was `linix <linix@localhost>` before), `git log` prints no signature noise
for an unsigned repo, and with the rule on, `rollback` refuses the commit by hash and says
`git says it is unsigned`; with it off the same rollback proceeds. **A signed commit was not
exercised end to end** — no signing key on this box — so the `G`/`U`/`B` handling is covered by
unit tests over git's documented `%G?` codes and not by a real signature.

**The first two attempts at it were both wrong, and running the tool is what said so.** The check
first went beside the guard in `SyncEngine::sync`, which is where every *package* funnels through
— and a `link:` line is an extra, applied by `App::apply_dependents`, so it never passes that
point at all. The unit tests were green. The second version asked the applied-extras ledger
whether the line was new; the ledger keys a link by its source path, so re-pointing an existing
line at `C:\linixtest` was placed with no question asked. Both were found by running `linix sync`
against a scratch config with a real destination outside the home directory, and the four cases
(non-interactive refusal, `--dry-run`, `--yes`, second run) were each exercised against the
binary.

**3. S23 — `nimble`'s format legend was two phantom packages, and two commands printed
nothing.** Found by running the tool, not by reading it: `linix list` on this machine reported
`nimble:{PackageName}` and `nimble:└──`, because `nimble list --installed` prints the shape of
its own output when nothing is installed. The fix is in the shared `is_noise_line`, so it covers
every first-token parser (recorded as S23 under "Bugs found while implementing"). In the same
sweep, `schedule list`, `snapshot list` and `profile show` printed **nothing at all** when they
had nothing to show — indistinguishable from a command that failed. All three now say so;
`snapshot list` distinguishes *no snapshots yet* from *no snapshot provider on this machine*
(the whole answer when it is the second), and `profile show` names what a profile is made of,
because a profile reaching nothing is usually a `use` line pointing at an empty module.

**Still owed:** **K14** (nothing asserts that `rebuild` makes no git commit — unchanged: the
honest test still needs a backend that can really remove and reinstall, and this box has none
that is cheap and offline) and **Phase 6's containers**, which still have never been run (no
Docker here).

## Session 2026-07-21 (seventh session) — the owed list, and `@asset=all`

Started by checking the owed lists in this Part against the tree rather than reading them.
**Four of the items they carry were already built and the entries were stale** — see item 6.
Green at each commit (`cargo build --all-targets` clean, `cargo clippy --all-targets` silent,
`cargo test` all suites).

**1. K15 — a rebuild's removals are no longer called removals.** `rebuild` printed its own plan,
which never says "remove", and then ran two ordinary `sync` transactions whose summary reported
`Removals: 214` on a run where all 214 come straight back. `sync` now passes the run's shape to
the summary (`metrics::Narration`, derived from the guard scope), and under a rebuild the two
counters read `Reinstalled` and `Removed to reinstall`. **The backends' own progress logs were
deliberately left alone**: `apt` really is removing those packages at that moment, and a line
saying otherwise there would be false.

**2. W13's second half — `plan` names the variables that moved.** The note existed and only
`sync` printed it, so the command you read *before anything is touched* was the one that did not
explain its removals. `print_vars_changed` no longer takes the caller's resolver, so both paths
call it under one rule: removals present, variables named.

**3. `init` writes the `vars` file II.1 lists.** Comments only — a `role = desktop` LiNix
invented would be a condition nobody chose, and IX.3 makes every reference to an undefined name
an error, so an empty file is the honest starting state. *Verified against the binary:* a fresh
`linix init` writes it and `linix check` is clean.

**4. `status` shows extras that a sync would undo.** The applied-extras ledger (S20) has undone
removed `service:`/`link:`/`repo:`/`shim:`/`setting:`/`schedule:` lines since Phase 2, but
`status` reported packages only — so the run that disables a service was previewed as "nothing
to do". `App::extras_drift` is the one place the question is asked, and `reconcile_extras` asks
it there too: **a preview computed a second way is a preview free to disagree with the run.**

**5. RULED and BUILT (owner, 2026-07-21): `@asset=all` installs every match.** It parsed and
selected and then refused by name, because `GithubState` held one `bin_path` and the lock held
one entry per declaration. Both are now lists: `GithubState.artifacts` (asset, format, deployed
path) and `locks/github.toml` keyed by declaration with a list under it. Three rules were needed
and none of them was in the spec:

- **The deployed name.** *(Options offered: each file keeps the name of the program inside it;
  prefix every file with the repo's name; or leave the refusal standing.)* The owner chose the
  first. One artifact still deploys under the repo's name, as it always has; several each keep
  their own, and **two that would land on the same name is an error naming both files** rather
  than one silently overwriting the other. In Part II under artifact selection, reasoned in V.48.
- **Everything is downloaded and hashed before anything is unpacked or deployed.** With several
  files under one line, a supply-chain objection to the third must not arrive with the first two
  already on `PATH`.
- **One subdirectory per artifact.** Two archives under one declaration can both contain `bin/`,
  and unpacking them into one tree loses one of them.

The lock's comparison is now set-shaped (`verify_set`): a release that *reorders* its assets did
not change what is installed, but a name that was not locked, or one that is locked and no longer
resolved, is the same objection a changed asset always was. A declaration that stops deploying a
name it used to deploy has that file removed from `PATH`, or nothing declares it and no sync can
see it.

**6. Four owed entries were stale, and are corrected in place.** Each was checked against the
tree, not the document: **K3** (`rebuild` does take a `PreRebuild` snapshot and does restore it
on a failed reinstall), **VIII.2's lock half** (the resolved asset, url, format and hash have
been in `locks/github.toml` since the artifact work), **the `vars` script and executable
providers** (`vars.linix` and `vars.<ext>` both exist and are wired into resolution), and
**`expand_vars`'s early return** (an empty variable set no longer skips the walk, so `$role`
with no `vars` file is the same error as a misspelling). The `formats` block at `priority` level
(D7) is built as well.

**Still owed, and unchanged by this session:** **K14** (nothing asserts that `rebuild` makes no
git commit — it makes none, because `handle_rebuild` never calls `perform_maintenance`, but the
only honest test needs a real backend); **II.13's signature check**; **SEC3**; and Phase 6's
containers, which still have never been run. *(II.13 and SEC3 were closed by the eighth session,
above; K14 and the containers still stand.)*

**What was verified, and how far it got.** The github work was exercised against the real API and
a real release (`sharkdp/fd` v10.2.0), from a scratch repo with its own config *and data* root:

- `@asset=all` on that release selects both Windows archives, downloads and hashes both, opens
  both, and **refuses by name** — *"both `fd-…-gnu.zip` and `fd-…-msvc.zip` install a program
  called `fd.exe`"* — leaving `~/.local/bin` untouched. The first attempt at this deployed one
  file before refusing, which is what moved the collision check ahead of every deploy.
- The same line narrowed to one artifact installs, deploys `fd.exe` under the repo's name,
  writes `locks/github.toml` in the new list shape with the real sha256, is *"already up to
  date"* on the second run with no API call, and uninstalls the binary, the tree and the lock
  entry clean.
- `status` prints and JSON-reports the extras a sync would undo. The **diff and the output** were
  exercised against a hand-written `locks/extras.toml`; **no extra was actually provisioned and
  undone** — that path is S20's and is unchanged.

**No successful two-file install was performed:** no release to hand ships two differently-named
Windows programs, so the multi-deploy loop past the collision check has run only in unit tests.
**Also not verified this session:** anything needing Docker.

*One thing worth carrying: the first end-to-end run was made against the machine's real config
and registry, and `sync` scheduled removals of packages that registry claimed LiNix owned. It
did no damage — the two it reached were already absent — but a test of a download backend has no
business planning removals, and every run after it set `LINIX_DATA_DIR` as well as
`LINIX_CONFIG_DIR`.*

## Session 2026-07-21 (sixth session) — Part IX's tail: W11 and W8, and the bug under W8

Took the fifth session's owed list, which was the whole remaining Part IX register. **W1–W14 are
now built.** Green at each commit (`cargo build --all-targets` clean, `cargo clippy --all-targets`
silent, `cargo test` all suites — run the command, do not copy a number).

**1. W11's gating half — resolution carries the conditions that admitted each line.** The resolver
knew a statement was *conditional* and nothing else, so W12 could say what `$role` is and never
which package that value put on the machine. The flag is now a chain: `Gate` (a predicate and the
line it is written on) and `Gates` live beside `Origin` in the grammar, and the chain composes
across the three levels that can gate a package — the `active` block that turned the profile on,
the profile's block around its `use`, the module's own block. It lands on the spec as `__gated_by`,
**filtered to conditions that test a variable**: a `when host == laptop` has no second hop to
explain, and listing it would bury the ones that do. `Document::walk`, `modules::expand`,
`ProfileLoader::resolve` (whose `Resolved.modules` is now `UsedModule { name, gates }`),
`apply_set_math`/`eval_expression`/`atom`, `Reached` and `to_spec` all thread it.

*Two rules were needed and are written into the code:* a module or package **reached twice keeps
the shortest chain** — reached once inside a condition and once outside it, it is here
unconditionally, and naming the condition anyway is a wrong answer; and `to_spec`'s three
provenance arguments became one **`Provenance`**, because origin, scopes and gates answer three
different questions and had started to read as interchangeable.

**2. W11's output.** `linix why` prints, under `because:`, each variable condition and what its
variables are now: *"`when $role == travel` at active:2 — $role is travel, set at vars:1"*. The
`__gated_by` round trip is `Display`/`FromStr` on `Gate`, kept in one place because it crosses the
`PackageSpec` seam where everything is a string. A tag that does not parse is printed as written
rather than dropped. *Verified against the real binary on a scratch repo, both ways:* with
`role = desktop` the package is declared nowhere, with `role = travel` both conditions are named
with their values and origins.

**3. W8 — and the bug underneath it, which was the larger half.** The resolution path had been
taught variables in the fourth session. **Every path that edits your files had not.**
`activate -a`, `deactivate`, `uninstall` and `declares` read `active` through
`HostFacts::current()`, whose variable set is empty — and an empty set does not make
`when $role == travel` a block that fails to match, it makes `$role` an **unknown key**. All four
verbs refused a correct file outright. Closed by deleting the varless readers rather than
defaulting them (P5: a default nobody chose): `parse_active`/`read_active` take facts,
`Editor::new` takes facts, and **`StateResolver::facts_for_host` is the one place that produces
them** — `resolve_model` now calls it instead of resolving variables inline. The messaging half is
`model::profiles::describe_gate`: `activate` and `deactivate` name a block with its variables'
current values, *"`when $role == travel` ($role is desktop)"*, because `active` holds the condition
and `vars` holds the value. *Verified against the binary:* `deactivate Trip` on a variable-gated
block reports the removal, the emptied block and the value.

**4. RULED and BUILT (owner, 2026-07-21): `when $var` works in `priority`.** It could not,
because `priority` says which backends exist and resolving variables needs that vocabulary — so
one of the two has to go first without the other, and `when $role == travel { cargo }` reported
*"unknown `when` key `$role`"*, naming the wrong problem entirely. *(Options offered: refuse it by
name with a hint, resolve variables first in two passes, or leave the confusing error.)* The owner
chose to make it work.

`priority` is now read twice. The **bootstrap** pass (`Priority::every_backend`, on
`gated::read_every`) takes every backend the file names, `when` blocks included whether or not they
match, and **evaluates no predicate at all** — so there is no condition it can fail to answer. Its
result is a vocabulary and never an order: it exists only so the `vars` file has backend names to
parse against, it can only ever be a superset, and a `vars` file names no backend, so nothing it
over-includes can change what a variable resolves to. The **real** pass is `Priority::parse`
against the resolved facts, and that is what decides the order and what `allows` answers.
`StateResolver::priority_for_host` now resolves variables first like every other reader of a
`when`, and `resolve_vars_against` is the single variable resolution the three callers share.
*Verified against the binary:* with `role = travel` a `cargo:` line passes `check`; with
`role = desktop` the same line is refused with *"`cargo` isn't in your priority list"*.

**5. RULED and BUILT (owner, 2026-07-21): the state registry has no old-format reader.**
`core/state.rs` carried `#[serde(default)]` on `suspensions` and `held`, with comments saying they
existed so registries written by older versions still load — an old-format reader, which NO-LEGACY
deletes. *(Options offered: delete and fail loud, keep it and rewrite the comments as an honest
empty default, or leave both alone.)* The defaults are gone: a registry missing either field is
refused with an error naming the file and saying to move it aside and run `linix adopt`. Filling
it in instead would have been a claim about the machine — *"nothing is suspended"* — that nobody
checked. The test that asserted the old behaviour is deleted and replaced by one asserting the
refusal.

**6. `check` reaches what no active profile reaches (II.3).** Carried owed since the fifth
session. Resolution reads only what the active profiles reach — that is II.3's rule and stays —
and `check` promised to parse everything, while actually parsing exactly what resolution did. A
module with a broken line was clean until the day someone activated the profile that reached it.
`Resolver::parse_everything` now parses every file in `modules/` and `profiles/`, returns **every**
error rather than the first (they are independent files), and `check` prints them all and exits
non-zero: not active and still broken is still broken. Reached files are parsed twice rather than
tracked and skipped — the bookkeeping to tell them apart would be a second answer to "what did
resolution read". Parsing only: whether a `use` names a module that exists is resolution's
question, and asking it here would report every profile on a machine that activates none of them.
*Verified against the binary:* a broken module no profile reaches now fails `check` by name, and
`check` is clean again once it is removed.

**7. A loop names every file and line in it, in order — at all three layers (II.7).** The `use`
cycle errors said *"module `a` ends up using itself: a -> b -> a"*: the names, and not one file or
line, so the reader still had to find the loop. The walk was already tracking the path; it was
tracking names only. It now tracks the line that entered each name (`model::cycle::Visit`), and
**one renderer** (`model::cycle::describe`) writes II.7's shape for all three:

```
modules use each other in a loop

  modules/a.txt:1  use b
  modules/b.txt:2  use a
                   ^ back to a
```

`@requires` loops go through the same renderer — II.7 calls them one error, and two spellings of
it is how the second goes stale. Its *walk* still differs (Tarjan over the plan graph, not the
resolver's path) because the graph is packages rather than files and is built before anything
looks for a loop; `Origin` gained the `FromStr` that reads `__source` back for it, beside the
`Display` that wrote it.

**`check` catches loops no active profile reaches** (II.7's own sentence): `parse_everything`
follows `use` as far as resolution would, so item 6's parse pass is a resolve pass. Rooting at
every module finds one loop once per member, so the reports are deduped — two reports of one loop
are rotations of each other, same hops, different starting point. *Verified against the binary:* an
a↔b loop no profile reaches is reported once, with both lines.

**8. RULED and BUILT (owner, 2026-07-21): `re:` works, and is frozen on first sight.**
`re:` was in the grammar, in II.2 and II.15 with measured numbers beside it, and **expanded
nowhere**: an `apt:re:^fonts-` line reached the validator as a package literally named
`^fonts-` and died with *"Invalid characters in package name"* — blaming the user's regex
characters for a feature that was never built. *(Options offered for the "only if frozen"
qualifier, which named a freeze mechanism that did not exist: park it, always record without
freezing, or always freeze. The owner chose always freeze, with **deleting the entry** as the
way to re-find — which is II.15's own "the lock file IS the switch", now automatic.)*

Built: an `Enumerable` capability (a manager's whole catalogue, distinct from search, which
matches descriptions and ranks), implemented for **apt** (`apt-cache pkgnames`) and **pacman**
(`pacman -Ssq`) and absent everywhere else, since a language registry has no list endpoint; the
expansion step in the resolver beside the bare-name probe, because both turn one written line
into what it actually names and both must run before the merge; `locks/regex.toml` keyed by
`backend:pattern`, because one pattern against two managers is two questions. A pattern matching
zero packages is an error — it is a typo every time. `check` prints each pattern and its count,
`why` says *"matched by `re:^fonts-` at modules/dev.txt:3"*.

*Verified against the binary:* a `re:` on a manager with no catalogue is refused by name; a
frozen pattern expands with no manager asked at all (which is how it was testable on a Windows
box with neither apt nor pacman); `why` names the pattern, the file and the variable condition
together. **NOT verified:** `apt-cache pkgnames` and `pacman -Ssq` have never been run — there
is no apt or pacman here and no Docker. The two commands are the only unexercised code in this
item.

**9. RULED and BUILT (owner, 2026-07-21): the security pass. SEC1 and SEC2 are closed.**
Both had decided approaches and were parked for a dedicated pass; the owner opened it. Read each
entry under Phase 5 for the detail. Two things worth carrying here:

- **SEC1's headline exploit was already dead, and nobody knew.** `web:…@bin=../../.bashrc` has
  been a parse error since VIII.2's artifact-option validation landed on 2026-07-20 — `@bin` is
  legal only on a backend that resolves one name to several files. The traversal was closed by a
  change made for an unrelated reason, which is worth less than it sounds: a vulnerability closed
  by accident stays closed until somebody accidentally reopens it. What is built now is the
  structural version — `utils::bin_destination` is the one place a PATH destination is built from
  a name, all three download backends go through it, and `[guard] confine_bin` is the key.
- **SEC2 and VIII.2 collided, and the owner ruled.** SEC2 (19 July) requires a checksum on
  `web:`/`appimage:`/`github:`; VIII.2 (20 July) makes a hand-written `@sha256` on `github:` legal
  only when the line pins one format, and puts github's integrity in `locks/github.toml`. github
  is exempt from the checksum half; the HTTPS half still applies to it, on every redirect hop.

**Not verified this session:** anything needing Docker, the network path in any backend, and the
OS scheduler. **The SEC2 refusals were exercised against the real binary** (plain HTTP refused,
HTTPS-without-checksum refused, `@allow_http` alone still refused for the missing checksum), but
no *successful* download was performed — the hosts were deliberately unreachable.

## Session 2026-07-20 (fifth session) — Part IX finishes its documentation and W5/W12

Picked up the fourth session's owed list. **Green at each commit** (`cargo build --all-targets`
clean, `cargo clippy --all-targets` silent, `cargo test` all suites — run the command, do not
copy a number).

**1. The `vars` language is now in Part II.** It was fully built but lived only in Part IX
(proposed). Migrated into **II.6b** — the `NAME = VALUE` statement and the `$` sigil, IX.3, typed
values and the no-coercion rules, the three providers and the `[vars] source` selector, the
embedded standard library, once-per-invocation resolution frozen into a plan, and the tooling —
with a resolution **phase 0** written into II.7 and **V.51–V.54** for the why (typed values / no
coercion; the sigil and the future-fact collision; provider by filename with ambiguity refused;
a plan freezing its variables). Documentation only, no behaviour change.

**2. W12 completed — resolution carries each variable's origin.** The resolved set was
`name → value` with no record of where a value came from. Added a `VarOrigins` map produced
beside `Vars` by the *one* resolution core (`resolve_with_origins` shares `winning_defs` with
`resolve`; `load_vars_with_origins`/`resolve_vars_with_origins` sit beside the value-only forms).
The origin is the winning definition's line for a line file, the provider file for a script or
program. `linix vars` now prints *"set at vars:6"*. Verified against the binary on a scratch repo.

**3. W5 built — `check` notes unused variables.** `model/vars::referenced_names` statically
scans every `$name` in the model files; `check` lists any resolved variable absent from that set
as a note (never an error). Static on purpose: a variable used only in another host's
`when host == …` arm must count as used, which this host's resolution never reaches. Verified on
a scratch repo: with `role` referenced in a `when` and `gpu`/`unused_flag` not, only the latter
two are flagged.

**4. W13 built — the plan names variables that changed since the last sync.** Owner ruled the
run-level note over per-package attribution (decision recorded at W13). When a plan removes
anything, the preview prints the variables whose resolved value differs from the last successful
sync — the committed `vars` at HEAD (V.30: LiNix commits only on a successful sync), so the
baseline is stable and needs no second resolution of the live provider. Line-file provider only;
a script/program whose values do not commit (a clock/network var) is skipped rather than shown
as "changed" every run. `git::show_at_head`, `StateResolver::vars_at_last_sync`,
`Resolver::resolve_linefile_body` and `vars::diff` are the pieces; the render is
`print_vars_changed` in the sync path. The baseline read + diff is verified against a real git
repo (`vars_change_is_measured_against_the_committed_baseline`).

**Still owed — the Part IX tail (both need gating-side reference tracking):**
- **W11** (`why` explains *"$role is travel, set at vars line 6"*) now has its origin foundation
  (W12) but still needs the **gating side**: recording which variable-referencing `when`
  conditions admitted each reached statement, so `why` can name the ones behind a package. That
  is a change to the resolver's `walk`/`statements_with_gating`, threaded into the `__source`-style
  tags.
- **W8 messaging**: `activate`/`deactivate` naming a `when $var` block (they reason about host
  blocks only today). Entangled with the pre-existing `activate`/`deactivate` `when`-block
  messaging (2026-07-20 audit findings 2–3); take them together.

## Session 2026-07-20 (fourth session) — Part IX begins: typed values (W2, built)

The designated next-session work is Part IX (the `vars` language) at the position-4 ruling. This
session opened it. **Owner rulings taken at the start, recorded at their register entries below:**

- **Build full position 4 this pass** — the embedded programmable provider included, not deferred.
- **The embedded language is Rhai, under the neutral file extension `vars.linix`** — neutral so
  the engine can be swapped later without renaming anyone's files. (An implementation choice per
  IX.6, recorded here where it lands, not a spec ruling.)
- **Providers are chosen by filename, and multiple provider files may coexist; a `[vars]` selector
  in `preferences.toml` picks the active one. Two present and none selected is a loud error, never
  a precedence guess.** This answers the undefined gap in IX.2/IX.6 (how a machine picks a
  provider) and settles W6 toward "highly configurable": several sources allowed, one active.
- **Interpolating a list into a string value is an error naming the variable**; a number or
  boolean interpolates to its obvious text form. (The still-open half of W9 for scalars only.)

**Built this session — Stage 1, typed values (W2):** `vars` values are now the four JSON types —
string, number, boolean, list — not strings. `model/vars::Value` carries the type; one
`parse_literal` reads both a `vars` line and a `when` right-hand side, so `gpu = true` and
`when $gpu == true` agree by construction. The W2 coercion rules are enforced in `Value::equals`
/`Value::order` and in `config/parser::eval_when`:

- **No cross-type coercion.** `"1" == 1` is false, `true == "true"` is false — not an error, just
  not equal. Strings still compare case-insensitively, which is the behaviour detected facts have
  always had (`os == LINUX`) and the one recorded deviation from "no surprises".
- **Ordering (`<`, `>`, `<=`, `>=`) is numbers only**; comparing a string by order is refused by
  name, not answered (`"10" > "9"` would lie). These operators are new to `when` this session.
- **`in` tests list membership** with the same no-coercion equality; the right operand may be a
  bracket literal or a list-typed variable.
- **No truthiness (W3, adopted).** A bare `when $flag` is a parse error suggesting `$flag == true`,
  so `false`/`""`/`0`/`[]` never quietly differ.

Values that are exactly one reference (`alias = $tags`) inherit that variable's type; any other
value containing `$` is string interpolation and yields a string. `expand` (the `$var` walk into
`link:` targets and `@version=`) stringifies scalars and refuses a list by name.

**Built this session — Stage 2, provider selection + external provider.** A provider produces
`name → value`; the line file is one, an external program another. `[vars] source` in
`preferences.toml` (`config::VarsSettings`) names the active provider when several coexist
(`vars`, `vars.py`, `vars.linix`); two present and none chosen is a loud error, not a guess —
this answers the undefined gap in IX.2/IX.6 and settles W6 toward "several present, one active".
The external provider (`model/vars_provider.rs`) runs the program — interpreter inferred by
extension per IX.6's "run by LiNix" — hands it the facts as `LINIX_OS`/`ARCH`/`HOST`/`FAMILY`,
and parses a JSON object or `name=value` lines into typed vars; a non-zero exit carries the
program's stderr.

**Built this session — Stage 3, the embedded provider.** `vars.linix` (`model/vars_embedded.rs`)
is a Rhai script LiNix runs in-process, under a neutral extension so the engine can be swapped
without renaming files. **It is pure by construction** — a stock Rhai `Engine` has no file,
shell, clock or network access — so the script's only inputs today are `OS`/`ARCH`/`HOST`/`FAMILY`
and it must end in a map of the four types. The host powers IX.6 permits are a separate standard
library, **owner decision pending (Stage 4), not built.**

**Built this session — Stage 4, the `vars.linix` standard library** (owner ruling, 2026-07-20:
every power an external `vars.py` has, always-on, since it is a script committed to your own
repo). Registered on the Rhai engine: `now`/`today`/`weekday`/`hour`/`year`/`month`/`day`;
`sh("cmd")` (trimmed stdout, throws) and `sh_ok`; `read_file`/`path_exists`; `env`/`has_env`
(`env` is W7's escape hatch); `http_get` (off the async runtime via reqwest blocking); and
`parse_json`. Fail-loud split: a question returns a value, a fetch throws.

**Built this session — Stage 5, a plan carries its resolved variables** (IX.6/W4). The
interactive `sync` path already resolved once and applied that resolution. The gap was the
saved-plan `apply` path: it re-resolves for the drift check, which would re-run a clock/shell/
network provider and trip a spurious drift on every plan. `DesiredState` now carries its vars;
`plan`/`bundle` freeze them into the `SavedPlan`; `apply` resolves the drift check against the
plan's frozen vars (`StateResolver::with_vars`). Operations stay hash-protected; vars are
auxiliary.

**Built this session — Stage 6 tooling: W12, W8, W14.** `linix vars` prints each resolved
variable, its typed value and type, and the active provider. `when $role == travel { Travel }`
in `active` now works (W8's core) — it failed with "unknown when key" because `active` detected
its own varless facts; `parse_active_with` threads the run's facts. `linix diff` and the git
manifest views gained `vars*`, so a variable edit that changes the machine is visible in the
change view (W14).

**Green at this commit:** `cargo build --all-targets` clean, `cargo clippy --all-targets` silent,
`cargo test` all suites passing (run the command, do not copy the number).

**Owed, and tracked — the remainder of Part IX:**
- ~~**W2's Part II home and Part V entry** are still not written.~~ **DONE 2026-07-20 (fifth
  session):** the language is now in Part II as **II.6b** (the statement and sigil, IX.3, typed
  values and coercion, the three providers and the `[vars] source` selector, the embedded stdlib,
  once-per-invocation resolution frozen into a plan, and the tooling), a resolution phase 0 is
  written into II.7, and Part V gained **V.51** (typed values, no coercion), **V.52** (the sigil
  and the future-fact collision), **V.53** (provider by filename, ambiguity refused), and **V.54**
  (a plan freezes its variables). No behaviour changed — documentation only.
- **W5** (`check` reports unused variables as a note) and **W11** (`why` explains *"$role is
  travel, set at vars line 6"*) both need reference/origin tracking threaded through resolution —
  the resolved `Vars` is `name → value` with no origins, and gating/interpolation do not record
  which names they touched. Deferred as a separate, more invasive change.
- **W8/W13 messaging.** `when $var` in `active` resolves (built), and a variable change already
  goes through the guard by construction (vars → desired state → plan → guard, so a one-line
  `vars` edit that removes a hundred packages hits `max_removals`/`protected`). What is NOT built
  is the *explanation*: `activate`/`deactivate` naming a variable-gated block, and the plan output
  naming the variable as the cause of a removal. This is entangled with a pre-existing
  `activate`/`deactivate` `when`-block messaging gap (2026-07-20 audit findings 2–3) and should be
  taken with it.
- **Not verified on this box:** the `http_get` live-network path (only a refused-connection error
  is exercised) and external providers depending on an interpreter not installed here.

## Done 2026-07-20 (second session) — the audit's findings, closed

**Every numbered finding in the audit below is now fixed, plus the II.17 zombie keys, the
smaller list, and the three-`when`-readers item.** Two owner decisions were taken during the
session and are recorded at the bottom of this entry.

**Measured at the end of the session, by running the command:** `cargo build --all-targets`
clean, `cargo clippy --all-targets` silent, `cargo test` **735 passing / 0 failed**. *Do not
copy that number forward — it is the eleventh count in this document, and the previous ten
were all stale within a session. Run the command.*

### The seven findings

**1. Block-form options skipped every validation — closed, and it was the whole class, not
one call.** `validate_options` had one caller (the short-form header parse) and
`merge_options` inserted the body's keys afterwards unchecked. The fix is not a second call
site: `statement::parse` now wraps `parse_inner` and ends in one `validate(origin, &stmt)`,
and `merge_options` calls the same function after merging. **The same function also gained
the option vocabulary for `shim:`/`service:`/`link:`/`schedule:`**, which had none at all —
`shim:jq@sorce=…` used to parse clean. `model::schedule` now imports the grammar's
`SCHEDULE_OPTION_KEYS` rather than keeping its own list.

*Verified the way the audit found it — against the real binary, on a scratch repo:* every one
of the six lines it printed as silently accepted is now refused by name, in both forms, and
the valid lines still pass. Unit tests: a table driving short and block form through the same
six violations, plus the `@lease` case named on its own because it used to reach a real
expiry (S19).

**2 + 3. `activate`/`deactivate` now do what II.6 says.** This is the entry the audit caught
a ✅ burying, so: the Phase 2i entry claimed "in full", the audit said two items were open,
**the audit was right, and both are closed now.**
- `deactivate` removed top-level lines only, with the policy the owner reversed on
  2026-07-17 written into the comment, and still printed *"It is still activated by the
  `when …` block"* — the sentence II.6 requires to be unreachable. It now takes the name out
  of the top level **and out of every `when` block that applies to this host**, drops a block
  the removal empties and says so, and leaves a block for another host alone with II.6's
  sentence about why.
- `activate` overwrote `when` blocks silently. It now names every block it removed.
- The editing is a pure function (`model::profiles::remove_from_active` → `ActiveEdit`, and
  `blocks_in_active`), not more line-fiddling in the command, so all six behaviours are unit
  tested — including the two that are about *not* acting.

**4. `expand_vars` no longer skips the walk when no variable is defined.** The early return
on an empty set meant that with no `vars` file a `$role` survived as literal text and became
a path with a dollar in it. Tested in the direction that was missing: no `vars` file, `$role`
in a `link:`, error naming it.

**5. The second config resolver is gone.** `config_path_from_argv` hand-parsed `-c/--config`
and fell back to the deleted `config.toml`, ignoring `--config-dir`, `$LINIX_CONFIG_DIR` and
the settings file — so `[command_aliases]` never loaded for anyone off the default path, out
of a file that could not exist. Replaced by `preferences_path_from_argv`, which peeks the
flags (unavoidable: aliases expand before clap runs) and hands them to `app::locate::locate`,
the one resolution. **`config.toml` now has no functional reader.**

**6. The `[guard]` gate reaches every change path.** `apply` threw away the `Err` from
`compute_full_changes` with `if let Ok(…)`, so `deny_packages`, `pinned_only`,
`require_snapshot` and `deny_vulnerable` never blocked it; it never called
`enforce_installs`, so `max_installs` did not apply; and `rebuild` went straight to
`engine.sync` with no gate at all. All three closed. *The removal half was already sound and
still is.*

**7. `rebuild.rs`'s `backend:name` splitter is gone.** `name.split(':').next_back()` degraded
`web:https://host/x.tar.gz` to `//host/x.tar.gz` and never checked the prefix named a
backend. `Scope::Packages` now carries a parsed `Target`, split by `split_removal_target` with
the registry in hand. **The third splitter went with it**: `main.rs`'s upgrade path
reimplemented that function inline and now calls it.

### II.17 — the five that were alive are dead

- **`[schedules]` is deleted, and there is one schedule store.** *(Owner decision below.)*
  `schedule add`/`remove` write the `schedules` **file** and then sync, the way `install`
  writes a module and syncs (P1); `schedule list` reads it. `add_schedule`,
  `remove_schedule` and `sync_schedules` — the three methods that wrote the config table —
  are deleted, and so is `Config::schedules`. **Cron validation moved to
  `model::schedule::validate_cron`, called at parse time**, so a bad cron is refused naming
  the file and line rather than surfacing when the OS scheduler is handed the job; the
  scheduler calls the same function. The CLI flags are `--cron/--run/--notify`, matching the
  file's keys — they were `--command/--notification`, one vocabulary in two spellings.
- **`confirm_destructive` is deleted**, along with the upgrade prompt it drove and its line in
  both the shipped example and the generated default. The guard already judges removals; a
  second "are you sure" keyed off a config flag was the extra prompt II.10 does not list.
- **`config.snapshots` is deleted.** Its only live key was `auto_prune`, which is a second
  answer to a question `[retention.snapshots]` already answers — `keep_last = 0, keep_days =
  0` **is** "keep everything". `RetentionPolicy::prunes()` is now the gate, and `init -i` asks
  one question instead of two.
- **`github_token` is deleted from `Config`** and read from `$GITHUB_TOKEN`. II.1 says secrets
  are the environment only, never a file — and `preferences.toml` lives inside the git repo,
  so the key was a token in git.
- **`groups/` is out of the live help.** `bundle`'s restore instructions named it; they now
  name `modules/`, `profiles/` and `active`. The `git.rs` header naming `groups/` and
  `config.toml` went too, along with its paragraph about the deleted generation format.

### The smaller list

- **`strip_comment` no longer truncates a value at any `#`.** A `#` opens a comment at the
  start of a line or after whitespace; `@content=#!/bin/sh` is a value. `active` and
  `priority` use the same function now rather than three inline copies of `find('#')`.
- **`GuardScope` no longer names commands that do not exist.** `Rollback` was never
  constructed and is deleted. `Prune` was one label for two different commands — it now
  splits into `RemoveOrphans` and `PurgeUnmanaged`, so a refusal names what the user typed
  instead of printing *"prune refused"* to someone who cannot run `prune`. `Leases` →
  `ExpirySweep`, `Canary` → `upgrade --canary`, `Remove` → `uninstall`.
- **Three stale `--help` strings** (`module`'s "@module syntax", `init`'s "groups, modules,
  data dirs", `init -i`'s "config.toml and local.txt") and `--config`'s "Path to custom
  config.toml" all now describe what exists.
- **`tests/exhaustive_backend_suite.rs` is deleted.** Its first line was `// tests/
  mock_providers.rs`; it was a stale copy registering zero tests.
- **The last `cockpit` comment and every `GhostShell`/`test_ghost_shell_*` name are gone.**

### One `when` reader where there were three

New `config/grammar/gated.rs`: `active` and `priority` are the same file shape — bare names
plus `when` blocks — and were two hand-rolled block walkers with the same brace handling,
nesting limit and unclosed/stray-brace errors, written twice. Both now call `gated::read` and
keep only the rule that is theirs (a profile is Capitalized; first mention wins).

**The third reader stays, and that is a decision, not a leftover.** The grammar's own reader
handles modules and profiles, which hold *statements* and where a `when` may nest. `active`
and `priority` refuse a nested `when` on purpose — II.6 calls `active` "the one file you read
to know what is on, so it stays a list you can read at a glance." The audit listed the
divergence as a defect; it is one rule (`when` gates the lines inside it) applied to two
different kinds of file. **Recorded here so the next audit does not re-file it as a bug.**

### Owner decisions, 2026-07-20

- **`schedule add`/`remove` edit the `schedules` file** rather than being deleted. One store,
  two doors into it — the file stays the only source of truth, and a one-line CLI for a cron
  job stays available. *(Options offered: delete the mutating verbs, rewrite them onto the
  file, or delete the whole command.)*
- **Stop forcing LiNix's own commits unsigned.** `core/git.rs` passed
  `-c commit.gpgsign=false` on every invocation, which inverted II.13 and made
  `git commit -S` unreachable by construction. The override is removed; the user's
  `commit.gpgsign` decides. **The verification half is explicitly NOT built** — see below.

### Still owed after this session

- ~~**II.13's integrity check is not built.**~~ **RULED and BUILT 2026-07-21** (eighth
  session): the identity override is gone, `log`/`history` show what git says about each
  signature, and `rollback` refuses an unvouched-for commit under `require_signed_history`.
- ~~**`upgrade`'s whole-system mode (`app.upgrade()`, the native `apt upgrade` path) returns
  before `enforce_policy`.**~~ **RULED and BUILT (owner, 2026-07-20): route it through the
  gate.** It resolves the desired state and calls `enforce_policy` before it runs, like every
  other change path. `deny_packages` stays close to meaningless against "upgrade everything"
  and that is accepted; `require_snapshot` and `deny_vulnerable` are not, and they are the
  reason. *(Options offered: route it through the gate, honour `require_snapshot` only, or
  refuse whole-system upgrade whenever a policy is set.)*
- ~~**`Layout::lock_file()` still has zero callers**~~ **CLOSED 2026-07-21:** `github` asks
  `Layout` for the path. II.6's other two rows (resolved backend for a bare name, regex
  expansion) are still unbuilt, and II.6 now says so rather than stating them as fact.
- ~~**The `use`-cycle error still names only names**~~ **CLOSED 2026-07-21:** one renderer
  (`model::cycle`) writes II.7's shape for `use` loops at both layers and for `@requires`.
  The `@requires` *walk* is still Tarjan at plan time, and that is now recorded as a decision
  rather than a gap: the graph is packages, not files.
- ~~**`linix check` still does not reach what no active profile reaches**~~ **CLOSED
  2026-07-21:** it walks every module and profile, `use` included, so II.3's "parses
  everything on demand" and II.7's "catches cycles no active profile reaches" are both true.
- **99 comments in `src/` cite a `V.n` paragraph.** Not swept, on purpose: CLAUDE.md's rule is
  that a comment citing `V.n` *to explain a design* is narration, while one stating a
  constraint is not, and the two cannot be told apart by grep. A mechanical strip would be a
  large diff that deletes real constraints along with the narration.
- **Phase 6's six containers still have not been run** (no Docker on this box).
  `src/app/migrate.rs` is now `src/app/adopt.rs` and `Migrator` is `Adopter` — it was never a
  second implementation, it was `adopt` under a deleted command's name, and the word `migrate`
  is gone from the code with it (including a `source: "migrate"` arm in `why` that nothing
  writes).

**What this session did NOT verify:** the OS scheduler (no systemd/launchd/Task Scheduler
runner here — the line→config mapping, the file editing and the cron validation are covered,
the provisioning is not), the network path in any backend, and anything requiring Docker.

## Session summary — 2026-07-20 (third session): Parts VIII and X, most of the way

The session took Parts VIII and X from *proposed* to *mostly built*. Part IX (the `vars`
language) was deliberately left for a clean next session — only its two blocking owner rulings
(W2 types, and the position-4 provider model) were recorded, not implemented.

**Owner rulings this session**, each recorded at its register entry: D6 (checksums in `locks/`),
W2 (full JSON types, no coercion), the `upgrade` gate, D3/D3b/D4 wording, `GITHUB_TOKEN` over
`LINIX_GITHUB_TOKEN` (II.1 moved), K3 (rebuild snapshots and reverts), `setting:` reset-to-
default, and X.2 reverted as blocked on a download cache that does not exist.

**Built:** the `priority` options body (VIII.2, D7, D9); `locks/github.toml` recording the
resolved artifact and its hash (D6); one ownership-safe PATH deploy replacing three hand-rolled
symlinks (D4 corrected); `rebuild` snapshot-and-revert (K3); `why` explaining the format choice
(D14); `@version=` pinning and lock-first offline resolution (D1, D12); the `setting:` statement
with a GNOME adapter (X.4, K7); `linix reset` (X.3 level 3, K5); `clean-cache --all` (X.3 level
2, K16); and the `doctor` git line closing X.5's one gap (K8).

**Three bugs found and fixed in passing**, none on any list: all three download backends
destroyed a user's own `~/.local/bin/<name>`; `web:` on Windows recorded the wrong path; and
`appimage:` cleared its destination with the ownership-unaware removal helper.

**Not done, and why:** X.2 (`clean_cache_on_remove`) — no backend keeps a separable download
cache, so the preference has nothing to honour; it needs a download-cache layer first. K9 (the
git-less backup command) — owner left it unproposed. K15 (rebuild's plan saying "reinstall" not
"remove" in the engine's own progress) — needs the plan printer reworked, which the spec already
marks as not done; `rebuild`'s upfront plan already frames it. Part IX in full. The GitHub
network path, flatpak channel validation, and anything needing Docker remain unverified on this
box.

## Done 2026-07-20 (third session) — `priority` carries a backend's defaults (VIII.2, D7, D9)

The middle level of VIII.2's precedence existed in the spec and nowhere else: `formats` could
be written on a line or fall back to the detected default, and the `priority` file — the place
VIII.2 actually names — had no options concept at all. `Priority` was a `Vec<String>`.

**What was built**

- **The shared gated reader learned an options body**, behind a vocabulary flag. `priority`
  allows one; `active` does not, because a profile name answers one question and has nothing
  to configure. Inside a body a value is verbatim to end of line, so `#` is data (V.9) — the
  same rule the statement block bodies already follow, not a second one.
- **`Priority` stores each backend's body** and hands it out by name. First mention wins for
  the body exactly as it does for the order, so a matching `when` arm beats the plain line
  below it and the two cannot disagree.
- **The capability check moved to where both callers reach it.**
  `validate_artifact_options` now takes a backend and an options set rather than a
  declaration, so a body in `priority` is refused on the same grounds a line is: `formats` on
  `apt`, `channel` on `github`, an unknown format name. It was that or a second copy.
- **`to_spec` composes the three levels**, backend defaults first and the line's own options
  overwriting whole. This is the only place a declaration becomes a spec, so the precedence
  cannot drift between the imperative path and the file path.
- **The bare-line hole is closed.** `fd@formats=deb` could not be checked at parse time
  because the backend was unknown; the resolver now checks it once the backend is resolved,
  which is what the comment at the parse site always claimed happened.

**Also recorded, not built:** `to_spec` writes `__formats_from` (`line` or `priority (github)`)
so D14's `why` has a reason to print. **`why` now reads it** (`app/insight.rs`): on a backend
that selects artifacts it prints `formats:` — the order that applied and which of the three
levels set it, an absent tag meaning the built-in default. Backends whose ecosystem publishes
one artifact get no such line, gated on `selects_artifacts`.

**D7 and D9 are adopted** — see their register entries. Both owe a Part II home and a Part V
entry, and neither has one yet.

### The artifact lock (D6), and `channel` on flatpak

**`locks/github.toml` exists** — `core/artifact_lock.rs`, an `ArtifactLedger` of
`{version, asset, url, format, sha256}` keyed by the declaration's name. This is the D6 ruling:
the hash is generated content, recorded per machine, because one hand-written `@sha256=` cannot
cover an asset that differs per box.

- **The github backend writes it as it installs** and drops the entry on removal. A lock left
  behind after a removal would pin the next install to an artifact chosen for a declaration
  that no longer exists.
- **The alarm it buys:** the same asset of the same release, with different bytes than last
  time. No legitimate republish does that. It is an error naming both hashes, and deliberately
  *not* answered by selecting a different asset — that would turn a supply-chain warning into a
  silent substitution.
- **`@sha256=` is now legal only where the line pins exactly one format**, on a backend that
  selects between artifacts. The refusal says where the generated hash lives instead. On
  `appimage:`/`web:` nothing changes: those name one file already.

**`channel` reaches flatpak.** It parsed, validated, and was then ignored — the exact "an option
nobody reads is a line that does nothing" failure VIII.4 exists to prevent. Flatpak takes it as
the ref's branch (`app//branch`) rather than `--branch=`, because the install is batched and a
command-wide flag would apply one spec's channel to every package in the batch. `snap` gained
the test it never had.

### D1 and D12 — the release, and not calling the network to learn what you know

**D1 is built as recommended.** Unpinned stays `releases/latest`, which *is* GitHub's own
newest non-draft, non-prerelease release — filtering the full list here would be a second
definition of the same thing, free to drift from theirs. `@version=` is tried under both tag
spellings (`10.2.0` and `v10.2.0`); **both existing is an error naming both**, never a guess.
No prerelease option, per the register.

**D12 is built as recommended, and the ordering was the bug.** Every install called the API
before consulting local state, so a pinned, already-installed package burned a request on every
sync. Local knowledge now answers first: a pin, a lock entry, an install that matches both, and
no drift in `formats` or `@asset=` means **zero HTTP requests**. `sync` works on a plane.

The last two conditions are load-bearing rather than decorative: without them a pinned line
could never notice a changed `@formats=`, because no API call would ever happen to notice it
with — which would quietly reintroduce the bug artifact selection exists to close.

**Not verified:** the network path itself. The tag-URL shape, the 404-means-absent handling and
the deserialization of a tag-fetched release are compile-checked and reasoned from GitHub's API
contract, not observed. **A pinned line that is not yet installed costs two API calls**, one per
spelling, which is inherent to detecting D1's ambiguity.

### One way onto `PATH`, and it is not `shim:` (D4, corrected)

**There were four mechanisms for putting an executable on `PATH`**, and the discovery that
started this was that D4's instruction — "reuse `shim:`" — describes something `shim:` cannot
do. A shim is the linix binary under another name, re-dispatching by running the bare name
through `PATH`; aimed at an extracted binary that is not on `PATH`, it finds itself. **`shim:`
is a re-dispatch mechanism, not a deployment one.**

So `github:`, `web:` and `appimage:` each hand-rolled their own symlink, and **all three had
the same bug: they deleted whatever already sat at `~/.local/bin/<name>` without asking who put
it there.** `ShimManager` refuses exactly that (S4), so one shared directory had opposite
safety answers depending on which backend reached it first.

**Built:** one `deploy_executable` in `utils/file.rs`, beside `remove_deployed_path`, which is
its mirror. All three backends call it. A destination is LiNix's to replace when it is absent,
when it is a symlink pointing inside that backend's own artifact directory, or when it is the
exact path the backend recorded deploying last time — the last being what identifies a Windows
copy, which carries no pointer home. Anything else is an error naming the file, and the user's
binary survives.

**Two bugs fell out of the consolidation:** `web:` on Windows copied to `tool.exe` but recorded
`tool`, so its own removal path looked for a file that was never written; and `appimage:` used
`remove_deployed_path` on the destination, which is the *removal* helper and does no ownership
check at all.

*(Options offered: one shared helper with `shim:` left alone, extend `ShimManager` to cover
both, or fix the clobber in place and keep three implementations.)*

**`GITHUB_TOKEN`, and II.1 was the one that moved** (owner ruling, 2026-07-20). The spec said
`LINIX_GITHUB_TOKEN` and the code had always read `GITHUB_TOKEN`; the ruling kept the code's
name, because it is the one `gh` and CI already set and the value is unambiguously a GitHub
credential. *(Options offered: the conventional name, the namespaced name, or the namespaced
one with a fallback — the fallback refused as two paths for one question.)*

## Done 2026-07-20 (third session) — `setting:` (X.4, K7)

The one real gap X.4 named — desktop configuration that lives in a settings store, not a file
`link:` can write. GNOME's dconf and KDE's kconfig are not files, so `~/.config/i3/config` is
`link:` and "tap-to-click on, dark theme" is not.

**Built** as a fourth extra statement, `setting:SCHEMA/KEY @value=…`:

- **Grammar:** `Statement::Setting`, parsed by the same prefix table as `service:`/`link:`, and
  validated to name a schema, a key, and exactly one value — a line missing any of the three
  describes no state. `split_setting` is the one place `SCHEMA/KEY` is split, so the parser's
  refusal and the adapter's lookup cannot disagree.
- **Backend** `src/backends/setting.rs`, on the `service.rs` template: pure
  `read`/`write`/`reset`/`already_set` functions tested without a desktop, `detect_store`
  finding `gsettings`, and an `Installable` that reads before it writes so a settled sync runs
  nothing.
- **Removal resets to the schema default** (owner ruling), through the same `extras_lock` drift
  path `service:` uses — no per-key store of prior values.
- **No adapter is an error, never a silent write** (K7). Only GNOME is adapted; KDE refuses
  until `kwriteconfig`'s read-before-write is solved.

It inherits the model rather than extending it: `when` wraps it, two active declarations of one
key disagreeing is the II.7 error, and it is legal in modules and profiles (a `setting:` under
a `when` is a per-machine desktop config) — unlike `schedule:`/`vars`, which are file-bound.

## Audit 2026-07-20 — every claim in this document checked against the tree

> **CLOSED by the session above, which is dated the same day and ran after it.** Every
> numbered finding here is fixed; the false-claims table below is kept because it records
> what was wrong and where, but **the rows are answers to a state of the tree that no longer
> exists.** Read the entry above first.

**A full verification pass over Parts II, III, VII, VIII, IX and X at `e406924`.** Nothing below
was recalled: each row was run, grepped, or read at the cited symbol. Where a count is given, it
was counted today.

**Verified state:** `cargo build --all-targets` clean, `cargo clippy --all-targets` silent,
`cargo test` **718 passing / 0 failed** (650 lib + 17 bin + 51 integration). `linix --help`
starts. `linix check` runs and exits 1 with the `priority` refusal on a bare machine.

**The suite counts in this document are stale low, again.** "575 lib tests" (the artifact entry)
and "561 passing" (the R1–R23 entry) were both true when written. The artifact module's "59
tests" is now **66**. *This is the eighth measurement in this document to be wrong, and the
fourth in a row to be wrong in the direction of the tree being better than the sentence.*

### The findings that are bugs, not bookkeeping

> **All seven are fixed.** Each is answered by number in the session entry above.

**1. Block-form options skip every validation the short form enforces.** `validate_options`
(`config/grammar/statement.rs:538`) has exactly one caller — the header parse at `:430`.
`merge_options` (`config/grammar/mod.rs:310-334`) inserts the block body's keys straight into the
statement with no re-check. Confirmed by running the real binary against a scratch repo: the
identical violation errors in short form and passes clean in block form.

```
apt:nginx@requires=libfoo        ERROR "`requires = libfoo` is a bare name"   ← II.2, correct
apt:nginx { requires = libfoo }  OK: everything ... parses.                   ← same rule, silent
apt:jq@hold { version = 1.6 }    OK                        ← II.2's named contradiction, silent
apt:nginx { colour = blue }      OK                        ← unknown key, silently ignored
apt:nginx { lease = 2h }         OK                        ← S19's retired key, reachable again
apt:curl { formats = deb }       OK                        ← wrong-backend key, silently ignored
```

**This is the highest-severity finding in the pass**, because II.2 closes with *"silently
ignoring an option the user wrote is how a config grows lines that do nothing"* — and the block
form does exactly that for every key. It also reopens `@lease`: the comment at
`statement.rs:551-556` explains that a package which uninstalls itself is refused, and the block
form walks around the refusal. **The fix is one call**, but it will fail lines people have
written, so it is a change with a blast radius, not a tidy-up.

**2. `deactivate` implements the rule the owner reversed on 2026-07-17.** `app/profile.rs:177`
gates removal on `depth == 0` with a comment stating the *old* policy verbatim (*"Only top-level
lines are LiNix's to remove"*), and `:186-193` still prints *"Removed X from the list. It is
still activated by the `when …` block on line N."* — the sentence II.6 says must be **unreachable
by construction**. `ActiveEntry.on` exists for exactly this and is never consulted, so the
message fires for blocks that do *not* apply to this host, where it is wrong twice over: nothing
was removed, and nothing is activated here.

**3. `activate` overwrites `when` blocks and does not say so.** `profile.rs:102` prints only
`"active is now {names}"`. II.6 requires it to name every block it removed (S6: automatic and
silent are different things). No code reads the old body.

> **2 and 3 are not new findings — this document already recorded both**, in the 2026-07-17
> audit entry, under *"Two things are not fixed."* What is new is that **"Done in Phase 2i —
> `activate` does what II.6 says" claims it "answered the audit's `activate` finding in full."**
> It did not. The audit entry was right, the Phase 2i entry overwrote it with a ✅, and the code
> has not moved since. *This is the failure mode at the top of this document, caught in the act:
> a later, more confident sentence burying an earlier, more accurate one.*

**4. `expand_vars` silently skips expansion when no variable is defined.**
`model/resolve.rs:320`: `if self.facts.vars.is_empty() { return Ok(()); }`. With no `vars` file,
`link:~/.config/$role/init.lua` is left **literally unexpanded** instead of erroring — the exact
failure the `vars` entry says was designed out (*"A silently unexpanded `$rle` becomes a path
with a dollar in it and fails later, somewhere else, with no mention of the typo"*). The error
only fires when at least one variable happens to exist. Untested in either direction.

**5. `main.rs:273-285` is a second config resolver, and it points at a deleted file.**
`config_path_from_argv` hand-parses `-c/--config` out of argv and falls back to
`safe_config_dir().join("config.toml")` — the file the `preferences.toml` change deleted. Called
at `main.rs:59` for command-alias loading, it ignores `--config-dir`, `$LINIX_CONFIG_DIR` and the
settings file. So `[command_aliases]` silently never loads for anyone whose repo is not at the
default path, and the path it reads can never exist. **It is both the last functional
`config.toml` reader and a second implementation of a resolution the X.6 entry claims happens in
one function.**

**6. `apply` discards the `[guard]` gate.** `main.rs:2609`: `if let Ok(now_changes) =
compute_full_changes(app).await {` — an `Err` from `enforce_policy` is thrown away, so
`deny_packages`, `pinned_only`, `require_snapshot` and `deny_vulnerable` never block `linix
apply`. It never calls `enforce_installs` either, so `max_installs` does not apply. **`rebuild`
reinstalls without the gate too** (`main.rs:644` goes straight to `engine.sync`). CLAUDE.md's
*"every install/change path calls the `[guard]` gate; a guard on one command is a guard on
nothing"* is not currently true.

*The removal half is sound:* every removal path does reach `enforce`. I found no unguarded
removal.

**7. `rebuild.rs:115` splits a user-supplied name on `:` and trusts it.**
`name.split(':').next_back()` — so `linix rebuild web:https://x/y.tar.gz` degrades to
`//x/y.tar.gz`, and no backend prefix is ever validated. It is a C13 parser, in a file written
after C13 was declared closed.

### Claims in this document that are false

| claim | where | truth |
|---|---|---|
| *"II.1's file table does not list `vars`"* | `vars` entry | **False.** II.1 lists it (line 191). |
| *"`config.toml` is retired"* | `preferences.toml` entry | **Partly false.** 8 live references remain, one of them functional (finding 5). `args.rs:24` still advertises `--config` as *"Path to custom config.toml"*; `main.rs:3842` tells the user to run `config init` to write one; `fleet.rs:94` names it in an error. |
| *"a test asserts `config_root` stays `#[serde(skip)]`"* | `preferences.toml` entry | **False.** The `serde(skip)` is real (`config.rs:183`); **no test asserts it.** The structural guarantee the entry credits to a test is guarded by nothing. |
| *"the resolved backend for a bare name / regex expansion are recorded in `locks/`"* | II.6 | **False.** Neither is ever locked. `Layout::lock_file()` (`layout.rs:125`), the `locks/<backend>.toml` accessor, has **zero callers** — dead code. What exists is three files by hardcoded path: `locks/versions.json`, `locks/hooks.toml`, `locks/extras.toml`. |
| *"`linix lock <name>` / `lock --backend cargo`"* | II.6 | **False.** `Commands::Lock` takes no arguments. |
| *"the choice … is recorded in the lock"* (artifact selection) | II.2 | **False.** It is recorded in `GithubState` (`github.rs:20-31`), the backend's own state. Part VIII's own entry admits this; **II.2's adopted text does not**, and II.2 is the target state. |
| *"the choice and what it passed over are reported"* | II.2 | **Partial.** An install-time `info!` (`github.rs:245-259`), only when ambiguous. Not plan output, which is what II.2 and V.48 describe. |
| *"`channel` … the backends do not read it yet"* | artifact entry | **False for snap.** `backends/snap.rs:76-78` pushes `--channel`. True for flatpak only. |
| *"`schedule:` is unbuilt … the resolver never reads `schedules_file()`"* | Phase 2, lines 986-991 | **Stale.** `resolve.rs:303-305` reads it; `:516` enforces file context, with a test at `:982`. The warning the passage cites was deleted in the `rebuild` session. |
| *"Only `schedule:` is still unwired"* | S12 row, line 2421 | **Stale**, same reason. |
| *"the example file documents keys that no longer exist on `Config`"* | `preferences.toml` entry, "Still owed" | **Stale — that work is done.** All eight named keys are absent from `examples/preferences.toml`, and every key it does document maps to a real field. The stated rationale is also wrong: `config.rs:149` sets `#[serde(deny_unknown_fields)]`, so an unknown key is a hard error, not "silently ignored". *(The `aliases`/`command_aliases`/`fleet_hosts` half of that paragraph is still accurate.)* |
| *"Phase 0 … roughly 15% happened"* | 2026-07-17 audit | **Badly stale.** `groups_dir`, `line_declares`, `keep.txt`, `_active_profiles.txt`, `Commands::Prune`, `Commands::Migrate` are all gone; `local.txt` survives only in test fixtures and one stale help string. **`src/app/migrate.rs` is the one live row** — 702 lines, compiled, and the whole body of `handle_adopt`. |
| *"three of six paths have no reader"* | 2026-07-17 audit | **Stale.** Five of six read cleanly. `schedules` and `preferences.toml` both gained readers. The one real gap is `locks/`, and it is a different gap: a reader exists and nobody calls it. |
| *"`linix why` answers from the old model"* | 2026-07-17 audit | **Fixed.** Note the entry's own tripwire grep (`layout()\|Layout\|profiles_dir`) returns 0 and **always would** — `why` reaches the model through `StateResolver`, not `Layout`. **A tripwire that cannot go quiet is not a tripwire.** |
| *"`shim --source` is a live bug"* | VI.1 rows | **Dead by deletion.** There is no `shim` command; `create_shim` no longer takes a discarded source. |
| *"the pre-v7 `run-in-container.sh` was deleted"* | Phase 5 | **False.** The file is on disk (`docker/integration/`), alongside six Dockerfiles. Phase 6 says "five containers"; there are **six** (gentoo is opt-in, and ran 2026-07-22). |
| *"`teleport` … goes through `model/edit.rs`"* | Phase 2 checklist | **Stale text.** `teleport` is fully deleted — the grep is silent. The checklist still lists it as a writer. |

### II.17 — five things it says are deleted are alive

> **All five are dead as of the session above.** Kept as a record of what was found where.

| | |
|---|---|
| **`[schedules]`** | `config.rs:281`, written by `linix schedule add/remove` (`scheduler/mod.rs:81,141`), read at `main.rs:1983`. **This is two schedule stores**, both live — the config table and the II.1 `schedules` file (`resolve.rs:303`). The exact "two of everything" CLAUDE.md names as how this repo got into trouble. |
| **`confirm_destructive`** | `config.rs:239`, live at `main.rs:1190`, **and advertised to users** in the generated default preferences (`main.rs:3267`). |
| **`config.snapshots`** | `config.rs:277`. |
| **`github_token`** | `config.rs:222`, read at `backends/github.rs:487`. II.17 says this moves to the environment; II.1 says secrets are *"the environment only. Never a file."* |
| **`groups/`** | Gone from the model, still named in live user-facing help: `bundle.rs:211` tells users to copy `groups/`. |

Everything else in II.17 is genuinely gone — commands, flags, syntax, files and code all check out.

### Smaller, and each cheap to close

> **Most are closed** (see "The smaller list" above). Still open, and now tracked in "Still
> owed": the `use`-cycle error's origins, `@requires` cycles being separate machinery,
> `linix check` not reaching unactivated profiles, and the 99 `V.n` comments — which were
> deliberately not swept. The `when`-has-three-readers row is now two readers and one
> deliberate difference, explained above.

- **No option-key validation for `shim:` / `service:` / `link:`.** `shim:jq@sorce=cargo:jq`,
  `service:nginx@enabld=true` and `link:/a/b@targt=/c` all parse clean. Same defect as finding 1,
  different door.
- **`linix check` does not reach what no active profile reaches**, though its own doc comment and
  II.3 both say it parses everything on demand. A `use` cycle in an unactivated profile is never
  seen.
- **`strip_comment` (`grammar/mod.rs:148`) cuts at the first `#` on a statement line**, so a
  short-form option value containing `#` is silently truncated. Only the block form is safe — the
  mirror image of finding 1.
- **The `use`-cycle error names only names** (`module 'a' ends up using itself: a -> b -> a`).
  II.7 asks for every file and line in loop order. The per-edge origins are never collected.
- **`@requires` cycles are caught by separate machinery** (Tarjan in `sync/planner.rs:59-84`),
  not "the same error" II.7 promises: different type, different wording, and it surfaces at plan
  time rather than resolve time.
- **`when` has three readers** — the grammar's, plus hand-rolled ones in `profiles.rs:214-294`
  and `priority.rs:28-99` — and they diverge: `active` and `priority` refuse nested `when`;
  modules and profiles allow it. "One rule, everywhere" is one rule and three implementations.
- **Three `backend:name` parsers for user input**: `config/parser.rs:160`, `main.rs:863` (which
  reimplements `split_removal_target` rather than calling it), and finding 7.
- **`GuardScope` carries dead labels.** `Rollback` is never constructed; `Prune`, `Leases` and
  `Canary` name commands that no longer exist, so a refusal can print *"prune refused"* to a user
  who cannot run `prune`.
- **`git commit -S` integrity (II.13) is not built, and is inverted**: `core/git.rs:69` passes
  `-c commit.gpgsign=false` on every invocation, so LiNix's own commits are guaranteed unsigned.
- **`sha256` is a recognised package option key** (`statement.rs:439`) with no entry in II.2's
  table.
- **`--allow-mass-purge` is the flag; II.11 still specifies `--i-really-mean-it`** — the R1 sweep
  renamed it and II.11 was not updated.
- **`purge-unmanaged` is also refused in `schedules`**, not just `rebuild`
  (`schedule.rs:61`). Correct, and undocumented — K13 names only `rebuild`.
- **Three stale `--help` strings**: `module` advertises "(@module syntax)", `init` says
  "(groups, modules, data dirs)", and `init -i`'s doc says it writes "config.toml and local.txt".
  All four names are deleted grammar or deleted layout.
- **One `cockpit` comment survives** the R1 sweep (`core/git.rs:192`), and
  `tests/shell_lifecycle_tests.rs` still names `GhostShell` in an assertion message and
  `test_ghost_shell_*` in four function names.
- **`tests/exhaustive_backend_suite.rs` and `tests/mock_providers.rs` are near-duplicates**
  (215 and 236 lines of the same fixtures) and **register zero tests between them**. The
  "exhaustive" one is the stale copy: it lacks the II.1 repo scaffolding and the S11 sandbox
  comment the other gained.
- **99 comments in `src/` cite a `V.n` paragraph.** P6 and V.42 call that narration.

### What checked out, and is worth not re-verifying

`family`/`os-release` (7 tests, `ID_LIKE` before `ID`); the `vars` engine (19 tests) including
IX.3's ungated walk, the sigil, cycles, `$$`, the digit rule and both file-context refusals;
`rebuild` end to end (11 tests, K1/K2/K13, the hostile-order integration test, and all three
claimed gaps still genuinely gaps); the artifact selector (66 tests, closed vocabulary,
platform-before-formats, the four-level tie-break, `@asset=all` erroring by name, `@bin` turning
the guess off, `score_asset` gone from every `.rs`); X.6's `path`/`edit`/precedence/`--set` and
the settings-file-outside-the-repo test; II.3/II.4/II.5 in full — set math, ordering,
subtraction-always-wins, the layering rule, and II.5's teaching error verbatim; II.7's
contradiction and dated-line rules; `priority`'s "not listed = not used at all"; SEC4, SEC5 (with
its recorded deviation) and SEC6 as built; SEC1–SEC3 as deferred; and `teleport`, `clean`,
`switch`, the generation family and the `status`/`diff` alias panic all confirmed gone.

**Direction of staleness this pass: the tree is better than the document almost everywhere.**
The 2026-07-17 audit section is now mostly describing bugs that are fixed, and reading it as
current would cost someone a day re-fixing them. The genuine residue is small and is listed
above. *The exception is the ✅ direction, which failed the same way it always has: Phase 2i's
"in full" buried an accurate finding that is still accurate.*

## Done 2026-07-20 — `when family` means the distribution (owner ruling)

**`family` was `std::env::consts::FAMILY` — "unix" or "windows" — and every example in this
document written as `when family == debian` was therefore false.** It had never matched, on any
machine, since the key existed.

**Ruled (owner, 2026-07-20): both questions stay answerable, and each key answers one.**
`os` is the kernel (`linux`, `macos`, `windows`); `family` is the distribution (`debian`,
`fedora`, `rhel`, `suse`, `arch`, `alpine`), read from `/etc/os-release` and falling back to
the OS name where there are no distributions to tell apart. **The old meaning is deleted, not
kept beside the new one** — `os` already answered it, so preserving it would have been the
second spelling of one fact. *(Owner: "NEVER worry about existing users — there are none.")*

`ID_LIKE` is consulted before `ID`, so a derivative resolves to the family that actually decides
the artifact: Linux Mint is `debian`, Rocky is `rhel`. Seven tests cover the parse, including
the derivative and the unknown-distribution cases.

**This is what makes VIII.2's default format order real.** It was written as "Debian family →
`deb`" against a `family` that could never say `debian`, so the table described a branch nothing
could reach.

## Done 2026-07-20 — X.6: finding your files

**Built: `linix path`, `linix edit`, `--config-dir`, and LiNix's own settings file.**
`src/config/settings.rs` and `src/app/locate.rs`; 19 tests.

- **The settings file holds one key and the parser enforces it.** An unknown key is refused by
  name and told where it belongs (`preferences.toml`, in the repo). K11 said the refusal should
  be the parser's job rather than discipline, and it is.
- **Precedence is `--config-dir` → `$LINIX_CONFIG_DIR` → settings file → default**, resolved in
  one function that every command goes through, so `linix path` describes the run it is part of
  rather than a separate guess.
- **`linix path` prints one line** so `cd $(linix path)` works; `--explain` says which of the
  four sources won and where the settings file is. `--set DIR` stores it.
- **`linix edit [FILE]`** opens the repo or a file in it, and refuses anything that climbs out —
  otherwise it is an arbitrary-file editor that happens to live under a package manager.

**Found by running it, not by reading it: the settings file was landing inside the repo.**
The obvious spelling — `<config dir>/linix/settings.toml` — collides with the *default repo*,
which is `<config dir>/linix`. So on a default install the file that says where your repo is
would have been committed to git and shared across a fleet, **carrying one machine's absolute
path to every other** — the per-machine hand-maintained state II.1 exists to forbid. It is now
`<config dir>/linix.settings.toml`, and **a test asserts the settings path is not under the
default repo**, because the next person to tidy that name will not otherwise know why it is odd.

**Not built from X.6:** K12's symlink case is untested.

## Done 2026-07-20 — `config.toml` is retired; `preferences.toml` is the file (II.1)

The NO-LEGACY half of X.6. `config.toml` was not in Part II.1's file list at all, yet it was
what actually held LiNix's behaviour — and it **held `config_root`**, a key naming the location
of the directory it lived in. `preferences.toml` was in the list and had no reader. Two files,
one of them undocumented and one of them fictional.

- **One file, at `<config_root>/preferences.toml`**, which is what `Layout::preferences_file()`
  had been returning to nobody since it was written. `Config::config_file` is now
  `preferences_file` and is filled from that layout.
- **`config_root` is `#[serde(skip)]`.** It cannot be set in the file, and **a test asserts it
  stays that way** — the ordering paradox is closed structurally, not by a doc note. The
  resolution order is `--config-dir` → `$LINIX_CONFIG_DIR` → settings file → default, and
  `load_and_merge_config` now runs it *before* opening the preferences file rather than after.
- **`linix config path` and `linix config edit` are deleted**, not deprecated. `linix path` and
  `linix edit` answer those questions, and after the rename the two pairs answered the same
  question about the same file. What `config edit` did better — create from the template if
  absent, re-parse on save so a typo surfaces at the edit rather than at the next unrelated
  command — moved into `linix edit`, which applies it when the target is the preferences file.
- **A second `default_editor()` in `main.rs` is deleted**; `locate::editor_command()` is the one.
- **`bundle`'s special case is deleted.** It copied the config file separately *because* the
  file lived outside the repo; the recursive copy of the root now covers it.

**`examples/config.toml` → `examples/preferences.toml`.** Its header pointed at
`~/.config/linix/config.toml`, a path that is now wrong in both halves.

~~**Still owed here:** the example file documents keys (`cache_ttl`, `prune_scope`,
`protect_imperative`, `bloatware_file`, `remove_bloatware`, `prune_on_sync`,
`[hostname_packages]`, `[managed_files]`) that **no longer exist on `Config`** — every one of
them is silently ignored on load.~~ — **Retired 2026-07-20 by audit: this was already done when
it was written.** None of the eight keys is in `examples/preferences.toml`, and every key it
does document maps to a real field. The rationale was wrong too: `config.rs:149` sets
`#[serde(deny_unknown_fields)]`, so an unknown key is a hard parse error, not a silent ignore.

**Still owed, and this half is accurate:** `Config` carries `aliases`, `command_aliases` and
`fleet_hosts`, which II.1 does not mention — and four keys II.17 says are deleted are alive
(`[schedules]`, `confirm_destructive`, `snapshots`, `github_token`; see the audit). Reconciling
the struct with Part II is its own pass and is not done.

**And `config.toml` is not fully retired.** Eight references remain, one functional:
`main.rs:273-285` resolves a config path by hand and falls back to `config.toml`, so
`[command_aliases]` never loads off the default path. **The claim above that "a test asserts
`config_root` stays `#[serde(skip)]`" is false — no such test exists.**

## Done 2026-07-20 — `vars`, first half (Part IX; owner ruling: position 3)

**RULED (owner, 2026-07-20): position 3 — derived values — "if it is not so hard".** It is not:
interpolation with dependency ordering and cycle detection is a small, wholly pure engine. What
was *not* built is an expression language — there are no operators, no functions, no
conditionals in values. A value may name another variable and nothing else, which is the honest
reading of 3 that does not become Nix.

**Also ruled: all three hook dialects stay** — Lua, Rhai, and shebang-to-anything. So the
engine choice that would have forced deleting `mlua` or `rhai` does not arise, and `tera` (a
template engine, a different job) is untouched.

**Built: the `vars` file, `$name` in `when`, and derived values.** `src/model/vars.rs` is the
resolution engine — 19 unit tests, no I/O; `Resolver::load_vars` reads the file; `HostFacts`
carries the resolved set and `value_for` answers `$name` from it.

- **IX.3 is enforced as a property of the file, not of the machine.** A name defined only
  inside a `when` block is an error *even on a host where that block does not match* — checked
  against every definition the document contains via a new ungated walk. Getting this wrong
  would make one repo valid on the laptop and broken on the desktop, which is the exact failure
  IX.3 exists to delete. **A test asserts the miss case**, because the hit case passes either way.
- **The sigil holds (IX.4).** `$os` and `os` are different questions; a test asserts a variable
  named `os` cannot shadow the detected fact.
- **Two matching `when` blocks that disagree name both lines** (II.7.5); two that agree are
  redundant, not wrong.
- **Cycles name the whole loop** (`a -> b -> a`), per V.45.
- **A variable name cannot start with a digit**, so `awk '{print $1}'` in a value is the shell
  text it looks like rather than an error about an undefined `1`. Found by writing the test
  that asserted the opposite and disbelieving it.
- **`$$` is a literal `$`** — without it there is no way to write a dollar sign at all.
- **An unknown reference is an error, never left as literal text.** A silently unexpanded `$rle`
  becomes a path with a dollar in it and fails later, somewhere else, with no mention of the typo.
- **File context is enforced both ways:** a `NAME = VALUE` outside `vars` is refused (it would
  make `$role` depend on which profile you activated), and a package line inside `vars` is too.

**Resolved once per invocation (IX.6):** `resolve_model` loads vars before any `when` is
evaluated and hands them to `HostFacts`. Nothing re-resolves them mid-run.

**Not built, and each is load-bearing:**
- ~~**The script and executable providers.**~~ **Stale — corrected 2026-07-21 (seventh
  session).** All three kinds exist (`vars`, `vars.linix`, `vars.<ext>`) in
  `model/vars_provider.rs` and `model/vars_embedded.rs`, and resolution selects between them.
- ~~Position 3 stops at `vars` itself.~~ **Wired in the same session.** `Resolver::statements`
  expands `$name` into option values, `link:`/`shim:`/`service:` names and `repo:` specs, once,
  after `when` gating and before anything reads a value — so the prober, the merge and the
  backends never learn that variables exist. **A `schedule:`'s `run` is deliberately left
  alone**: it is a command line, and `$` there belongs to the shell that will execute it.
  A line this host never reached is never expanded, so an unused `when` arm cannot fail on a
  variable irrelevant to this machine.
- **W1–W14 remain formally void.** This lands W3 (no bare `$flag`), W7 and W10 by construction;
  the rest are untouched and must still be re-asked.
- ~~**`plan` does not show variables as a cause of change (W13).**~~ **Built 2026-07-21
  (seventh session)** — `plan` prints the same note `sync` does, under the same rule.
- ~~**`init` does not scaffold a `vars` file.**~~ **Built 2026-07-21 (seventh session)** — a
  comments-only file, so no name is invented.
- ~~**`expand_vars` returns early when no variable is defined.**~~ **Stale — corrected
  2026-07-21 (seventh session).** An empty variable set no longer skips the walk: with no `vars`
  file at all, `$role` is the same error it is when the file exists and the name is misspelled.

## Done 2026-07-20 — `rebuild` (X.1, K1, K2, K13)

**Built: `linix rebuild [PKG…] [--backend N] [--all]`.** `src/app/rebuild.rs` holds the pure
half — scope selection, batching and ordering — with 11 unit tests; `handle_rebuild` in
`main.rs` holds the applying half. Part II gains II.11b; the reasoning is V.49.

- **Batch per backend (K1), foundation first.** `needs_root()` is the foundation test, so there
  is no second list of "system backends" to keep in sync with the registry.
- **A scope is required (K2).** A bare `rebuild` prints the three forms and exits non-zero.
- **`schedules` refuses `run = rebuild` (K13)**, by first word, so `run = sync --locked` is
  unaffected.
- **Removal and reinstall are two `sync` calls per backend**, not one graph — see V.49.
- **Protected packages, declared-but-not-installed packages, and names nobody declared are
  dropped and printed**, each with the sentence explaining it.

**The ordering rule X.1 said was owed is now written down — and X.1's own reasoning for it was
wrong.** Blast radius argues for the foundation batch going *last* (if `apt` strands first the
machine has no shell). The ruling stands on dependency direction instead. V.49 records both,
because the next person to read "foundation first, so a strand lands furthest from boot" will
correctly conclude it is backwards and may reverse the rule.

**An integration test asserts the ordering against the registry this host actually builds** —
every `needs_root()` backend before every one that is not, from a deliberately hostile input
order. It was mutation-checked: inverting the comparator fails it (`choco needs root but was
ordered after mise`), so it is not passing vacuously.

**Two pre-existing bugs found by exercising the K13 refusal, both about `schedule:`:**

1. **`check` never validated schedule lines at all.** `schedule_config` — which is where `cron`
   and `run` are checked — ran only in `apply_schedules`, at provisioning time. So a schedule
   missing its `cron` passed `check` cleanly and failed later, on a file `check` had already
   called good. `check` claims to parse everything the active profiles reach; now it does.
2. **The resolver warned that every `schedule:` line does nothing.** *"`schedule:` is not
   applied by `sync` — the scheduler owns it, and that wiring is not built yet."* S21 wired
   `apply_schedules` as II.7 phase 4 **on 2026-07-17** and the warning was never removed, so
   LiNix spent three days telling users their working schedules were inert. Deleted.

**And a documentation bug in two places:** both the spec (`schedule:tidy { run = clean }`) and
`schedule.rs`'s own module docstring showed an *inline* block form. The block form is multiline
— a line ending in `{`, then `key = value` lines, then `}` — so both examples were syntax that
does not parse. Corrected to the short form and the real block form.

**Not built:**
- ~~**K15 is the real gap.**~~ **Built 2026-07-21 (seventh session)** — the summary is told which
  run it is narrating, so a rebuild's counters read `Reinstalled` and `Removed to reinstall`.
- ~~**K3 is answered thinly.**~~ **Stale — corrected 2026-07-21 (seventh session).** `rebuild`
  takes its own `PreRebuild` snapshot and restores it when a reinstall fails, with a distinct
  message for the case where the rollback itself failed.
- **K14 (no git commit) is untested** — nothing was added, but nothing asserts it either.

## Done 2026-07-20 — artifact selection (Part VIII, first half)

**Built: `formats`, `asset`, `bin`, `channel` — parsed, validated, and wired into the `github`
backend.** `src/backends/artifact/` is the new module: `format` (the closed vocabulary and the
detected default order), `platform` (does this file run here), `pattern` (`@asset=` globs),
`discover` (the executable inside an archive), `capability` (which backends the keys are legal
on), `options` (reading them off a resolved spec) and `select` (the engine). 59 unit tests *(66
as re-counted 2026-07-20)*, none
of which touch the network — the selector is given an asset list and returns a choice, so every
rule is testable without a release to download.

**`score_asset` is deleted, not wrapped.** Its three defects are written up in V.48; the one
worth repeating here is that it had **no tie-break**, so between two equally-scored assets the
winner was whatever order the API returned — the same declaration installing different files on
two machines. **That was live on `HEAD` and no test covered it**, because nothing in the suite
had an asset list to be ambiguous about.

**Two things found while building, neither of which was on any list:**

- **`when family == debian` has never worked** — `HostFacts::family` was
  `std::env::consts::FAMILY` ("unix"/"windows"), so II.2's examples and VIII.2's default-order
  table both described a branch nothing could reach. Reported rather than fixed on the spot,
  because it changes what a `when` block means; **ruled the same day and now done — see the
  `when family` section above.**
- **The Debian default would have broken installs.** With `deb` first on Debian, the backend
  would have downloaded a `.deb` and handed it to `extract_archive`. Installing a system
  package is D5 (ownership: `dpkg -i` puts it in apt's database, where apt can then upgrade it
  out from under LiNix) and is unbuilt. **The backend now declares what it can install and the
  order is narrowed to it**, so the default falls through to `appimage`/`tarball`/`binary`
  exactly as "a later entry is a fallback" already means — and an *explicit* `formats = deb`
  gets a named error instead of a confusing extraction failure.

**One deviation from the D3 ruling's letter, recorded rather than absorbed.** The ruling says
shortest filename wins. The implementation checks **specificity first** — an asset naming this
machine (`fd-linux-x86_64.tar.gz`) beats a shorter silent one (`fd.tar.gz`) — and falls to
shortest only among equally specific candidates. **The D3 case itself is unaffected**
(`fd_10.2.0_amd64.deb` and `fd-musl_10.2.0_amd64.deb` are equally specific, so shortest still
picks the plain one). It is a strict improvement on the heuristic, and it is written here
because it is not what was ruled.

~~**`@asset=all` parses and selects but does not install.**~~ **Built 2026-07-21 (seventh
session)** — the state model and the lock both hold lists now, and the deployed-name rule was
ruled by the owner. See that session's entry.

~~**Not built from Part VIII:** the `priority`-level `formats` block (D7), `channel` on the snap
and flatpak backends, the lock file half of VIII.2.~~ **All stale — corrected 2026-07-21 (seventh
session).** D7 is built (`priority.rs`); `channel` is read by both snap and flatpak; the resolved
asset, url, format and hash are in `locks/github.toml`. *D14 was already noted built.*

**Suite: 575 lib tests *(650 as re-counted 2026-07-20; 718 total)* + integration all passing, `cargo build --all-targets` clean,
`cargo clippy --all-targets` silent, `linix --help` and `linix check` run** (measured
2026-07-20). *Green covers the new module properly — the 59 tests are the evidence for the
selection rules. It says nothing about the network path, which has no test and was not run
against a real release.*

## Done 2026-07-19 — R1–R23, the Phase 5 docs, F4/F5, S7, S11, G3

**R2, R3 done; R4 was already done and the entry was stale.** Each entry in Part III carries what
landed. The one with teeth is closed: **`teleport` no longer exists**, so the guard bypass it
carried (its own `StableDiGraph` into `Transaction::execute()`, no `enforce` call anywhere on the
path) is gone by deletion rather than by adding a guard call to a command nobody needed. `grep
-rni teleport src/ tests/` is silent.

**R4 is the second time this session a Part III entry described a tree that had moved on** — it
named `GenerationCommand::Rollback` and `rollback_to()`, neither of which exists; Phase 4 deleted
the generation command family when history moved to git. Recorded as ALREADY TRUE, not as done —
**writing ✅ on work nobody did is how Phases 0 and 1 got their false marks.**

**R1 and R6–R16 done — the voice sweep.** 149 log lines lost a self-branding `Component:` prefix,
the theatrical verbs went with them, and the pure-status lines were demoted to `debug!` so a normal
run is quiet. `cockpit` is `history`, `GhostShell` is `EphemeralShell` and no longer overwrites the
user's `PROMPT_COMMAND`, `--i-really-mean-it` is `--allow-mass-purge`, "Flight plan" is "Planned
changes", and the marketing adjectives are gone from `--help`, the crate docs and every log line.

**Three of those entries were wrong about their own scope, in the direction that costs a reader
time:** R6 named one `emoji()` call site and there were three, R10 named two `[dry-run]` sites and
there were three, and R7's backend counts ("50+" in `args.rs`, "33+" in `lib.rs`) were stale *and
disagreed with each other*. **Each fix is recorded on its entry, including the two judgement calls
that could reasonably have gone the other way** — keeping backend-name log prefixes while stripping
LiNix's own, and keeping `✓`/`✗`/`★` as information-carrying symbols under R9's no-emoji rule.

**R5, R11, R17–R23 done — the correctness batch.** Each entry carries what landed. The shape of it:

- **R19 was the largest and changed a trait.** `Upgradable::clean_orphans` both listed and removed
  in one call, which is precisely why nothing could be previewed; it split into `list_orphans()`
  (names, no side effects) and `clean_cache()`. `clean` is gone, replaced by `remove-orphans`
  (preview → guard → confirm → remove exactly what was shown) and `clean-cache`.
- **Four backends had been cleaning caches while reporting orphan removal** — `mise prune`,
  `pnpm store prune`, `nix-collect-garbage`, and `xbps-remove -Oy` (where `-O` is the *cache* flag;
  xbps orphan removal is `-o`). One command name covered two different operations depending on
  which backend answered.
- **Three entries described code that no longer exists** (R18's `rollback_to`, R22's
  `app/generation.rs`, and R4 earlier) — all Phase 4 casualties. **R18 is the one that mattered:
  the reported bug was already fixed, and a worse one had taken its place underneath.** Rollback
  checked the manifests out *before* the confirmation gate, so a non-interactive run without
  `--yes` rolled the files back and then refused to converge the machine.

**Two incidents from this session, recorded because a clean commit hides both:**

- **The first draft of `remove-orphans` probed for a capability by calling the destructive
  function.** To decide whether a backend could remove orphans it cannot list, the code called
  `clean_orphans` and checked for `Unsupported` — performing, in order to ask permission, the exact
  removal it was asking permission for. Caught before commit and replaced with
  `has_native_orphan_removal`. **The command whose entire purpose is "do not remove without asking"
  removed without asking, in its own first implementation.**
- **`src/main.rs` was truncated to 0 bytes mid-session** by an edit script whose output file was
  opened for writing before the replacement expression was evaluated; the expression raised, and
  the truncation had already happened. Roughly two hours of uncommitted work in that one file was
  lost and redone from the session's own history. **The cost was bounded only by the commits at
  R2/R3 and R1/R6–R16** — the argument for the spec's "commit at every major step" (rule 6), paid
  in full rather than in theory.

**Found while verifying, not in any R entry: `linix --help` panicked on every debug build.**
`status` carried `#[command(alias = "diff")]`, and Phase 4 added a real `diff` command; clap's
debug assertions abort on the duplicate. **Every CLI invocation of a debug binary died before
`main`, and the suite stayed green throughout, because nothing in it runs the binary.** The stale
alias is deleted. *This is rule 11 in its plainest form — 561 passing tests over a program that
could not start.* Three help strings were stale in the same place and went with it: `install`'s
"Imperatively", `rollback`'s `--package`/`--with-config` (flags it no longer has), and `profile`'s
advertisement of `switch`, which the owner ruled dead.

**Phase 5 docs are done (2026-07-19).** README and CHANGELOG were rewritten rather than patched —
see "Not started, and owed" below, which is now retired. Writing them found S22 (a phantom
package from an empty-result banner) and one wrong assumption of my own about module/profile
indirection, both recorded there.

**Phase 5 is closed except for the container work.** R1–R23, the README/CHANGELOG rewrite, F1,
F4 (goal), F5, G3, H2, P6, S7, S11 and S20/S21 are all done and each carries its evidence. **What
is left in Phase 5 is G2 and the harness's multi-backend sweep, and both need Docker, which this
box does not have** (`docker` is not on PATH) — so they were not attempted rather than written
blind. Phase 6's five containers are untouched for the same reason. **That is the honest blocker:
the remaining Phase 5/6 work is not hard, it is unrunnable here, and writing container shell that
has never been executed is precisely the "unverified is not done" trap this document is about.**

**F5 is done and F4's goal is met by deletion — read F4's entry before believing it, because the
mechanism it names does not exist.** The stale backend counts ("50+" / "33+") are gone from
`--help`, the crate docs and the README; the generated count lives in `linix doctor`, which
already builds the registry.

**Still open: SEC1–SEC3, which remain deferred**, the `--help`-queries-the-registry half of F4 if
the owner wants it literally, and Phase 6's containers (untestable here — they need Docker).

**Suite: 561 passing, 0 failed; `cargo build --all-targets` clean; `cargo clippy --all-targets`
silent; `linix --help` and the new commands run** (measured 2026-07-19, after R1–R23). *Green says
only that nothing covered broke. **R1 is verified by the greps in its entry, R17 by running the
real binary against a real `package.json`, and R19 by reading the trait — not by this number**,
which was 561 and green while `--help` could not start.*

## Done 2026-07-19 — the SEC4/SEC5/SEC6 batch

**The one thing the owner had cleared to land ahead of the deferred security pass, landed.**
Each entry in Part III's security section carries what was built; the shape of it:

- **SEC4** (ssh option injection) and **SEC6** (module-name traversal) are built exactly as
  ruled. **SEC5** is built with one deviation from the ruling's letter — `Snapshot.id` did not
  become a `u32`, because that field is shared with three providers whose ids are genuinely not
  numbers; the parse moved to the Windows boundary instead. **The property the ruling asked for
  holds; the mechanism named in it does not exist. Its entry says so** rather than reporting the
  ruling as implemented — which is the failure mode this document exists to stop.
- **SEC1, SEC2 and SEC3's confirmation were NOT touched.** They are the deferred pass, and it is
  still owed.
- **Two red tests on HEAD, found by running the suite rather than reading about it** — see the
  status line below. Both were stale tests, and one of them (`locks.json`) was a site Phase 4's
  own "all sites updated" claim had missed.

~~**Not done, and next in Phase 5:** R1–R23 are entirely unstarted — a grep for `Kernel:
Commencing`, `--i-really-mean-it`, `Flight plan:` and `src/app/teleport.rs` all still hit.
R2 (delete `teleport`) is the one with teeth: it is a second transaction engine that **bypasses
the guard**, so it is a safety item wearing a tidiness label.~~ — **R1–R23 are all done as of 2026-07-19; see the
R1–R23 section at the top of Part VII.** All four greps are now quiet.

## The state at `HEAD` (2026-07-17)

- **68 commits** since `d49d28c`. *(The "49" that stood here was stale by 19 commits — an
  adversarial audit on 2026-07-17 ran `git rev-list --count d49d28c..HEAD`. The header drifted
  behind the tree it heads.)*
- ~~**522 tests passing, 0 failing.**~~ — **the "0 failing" was false, and it is the first time
  this document has been wrong about a number it could have checked by running one command.**
  On 2026-07-19 a plain `cargo test` on untouched HEAD had **two failing tests**, both stale
  rather than newly broken, and both fixed in the SEC4–6 commit:
  - `create_shim_refuses_to_clobber_a_file_linix_did_not_deploy` — the guard is correct; the
    **test** built its victim file at `bin/jq` while `create_shim` targets `bin/jq.exe` on
    Windows, so it exercised a path the code never touches. A Unix-shaped test on a Windows
    box, red since whenever this repo was last run on Linux.
  - `test_locked_mode_version_conflict_enforcement` — wrote its fixture to
    `config_root/locks.json`, **the path Phase 4 deleted** when it moved version pins to
    `locks/versions.json`. Phase 4's own entry claims "all read/write/doctor/help sites
    updated"; the test was a site nobody counted.
  **Both are the NO-LEGACY failure in test form** — a test still naming what the code stopped
  doing — and neither would have been found by reading. **Now: 560 passing, 0 failing,
  `cargo clippy --all-targets` silent** (measured 2026-07-19). *(Treat any single count as a
  tripwire, not a target — and note that this line's predecessor proves a count can be stale
  in the one direction "green means nothing" does not cover: it was not green.)*
- *Those two numbers tell you nothing about the line below them, and never could — every false
  ✅ in this document was green when it was written (rule 11). They are here because a **red**
  suite would be worth reporting, not because a green one is progress. **The 2026-07-17 audit
  found four more false claims and the suite never moved off 0 failing.***
- ~~Phase 0 ✅ · Phase 1 ✅~~ — **both false. See the audit immediately below.** ·
  **Phase 2 — the cliff is jumped; the command surface remains** · ~~Phases 3–6 not started~~ —
  **also false, and this one drifted *downward*.** Part III below marks a dozen Phase 3–5 items
  DONE with commits behind them (guard consolidation `a757bfb`, install ceiling, the II.12 hook
  ledger `2993c6c`, snapshot-age S2, F1 `d2472e3`/`c571b19`, H2 `c8c37b3`, P6 `e118e10`), and the
  2026-07-17 audit verified them real in the code — not self-graded ✅. Phases 3–5 are **partly
  built, not "not started"**; Phase 6 is untouched. A header that says nothing past Phase 2 began
  would send a cold reader to redo committed work.

## Audit: the ✅ that are not true (last run 2026-07-17, second pass)

> **MOSTLY STALE as of 2026-07-20 — do not act on this section without checking the audit at the
> top of Part VII.** Re-verified item by item: `linix why`, `linix init -i`, Phase 0's deletions,
> the missing readers, and VI.1's two rows are all **fixed**. Two things survive: `activate`/
> `deactivate` (see the correction on the Phase 2i entry) and `src/app/migrate.rs`, which is
> alive at 702 lines. **This section now describes mostly-repaired code, and reading it as
> current costs a day re-fixing what is fixed** — the same cost as a false ✅, in the other
> direction.

**Phases 0 and 1 were marked ✅ before this section existed, and neither survives a grep.**
Checked by hand against the working tree, not recalled. **The suite is green and will stay
green through all of it: green means the old model still works, which is precisely the
problem.**

> **The second pass (2026-07-17) re-ran every command in this section and found the section
> itself was wrong in both directions.** One entry was **fixed and still filed**
> (`_active_profiles.txt` — with a retirement grep that could never have gone quiet), and
> C13's evidence had rotted so far that **four of the six sites it named did not exist** and its
> "Fixed when: 1" would have driven someone to delete a working validator. **Meanwhile it had
> missed three live ones** — `linix why`, `init -i`, and `activate`.
>
> **Phase 2's own account was the trusted one — "written from the work" — and it is where two of
> the three new findings were hiding.** `linix init` is ticked *"verified by running it"* and the
> wizard writes the old model; the `local.txt` row says `line_declares` is deleted and it is
> live with passing tests. **"Written from the work" is the same claim as "the tests pass": an
> observation standing in for the plan (rule 11).** Trust this section's *commands*. Do not
> trust its *prose*, including this paragraph.

> ### How to use this section — run it, don't read it
>
> **Every finding below is a command and the answer that means it is fixed.** At each phase
> change: **run the commands. Do not read the prose and believe it.** This section was
> written on a tree that is already moving, and by the time you read it some of it may be
> false — **that is the intended failure mode.** A finding that no longer reproduces is
> **not** a finding to argue with: **delete the entry, in the same commit as the fix.**
>
> When every entry is gone, **delete this whole section and restore the ✅** — to Phase 0 and
> Phase 1 in Part III, to C13 in VI.2, and to the status header at the top of this document.
> Those four places are where the false ✅ lived; they are where the true one goes.
>
> **Do not add findings here that are merely unbuilt.** This section is only for **work this
> document claims is finished and is not** — the failure that costs a reader their trust in
> every other ✅. Unbuilt work goes on the phase checklists, where it is honest.
>
> *(Why this is a ritual and not a note: this document has now been wrong about its own state
> every single time anyone checked — **always in the direction that flatters the tree**, and
> **every one was found by a grep, none by a reading.** That is the whole argument for making
> this a command you run rather than a paragraph you trust. See "A warning about this
> document" below.)*

### ~~Phase 1 ✅ / C13 "done" — one `backend:name` parser~~ — C13 CLOSED (verified 2026-07-17, adversarial re-audit)

**This entry is retired.** Every non-validating `backend:name` splitter is gone. The last one,
`lease set` (`main.rs:1410`), went with the retired `lease` command in Phase 2s — after an
earlier pass of this entry called it "the last skipper," which was true when written and stale
within the hour. Verified now against the tree:

```
grep -rn "split_once(':')" src/ | grep -v "^src/parsers/"
```
**7 hits, zero skippers.** Each remaining site either validates against the registry before
trusting a prefix (`grammar/statement.rs:328`/`:187`, `config/parser.rs:94`, `main.rs:686`), is
a name-half helper on an already-resolved key (`model/resolve.rs:522`), or is a comment
(`grammar/statement.rs:561`). This agrees with VI.2's C13 row (DONE, Phase 2r/2s) — the two had
disagreed, this one was the stale side.

**Phase 1's ✅ is a separate question and is NOT restored here.** C13 was only one of Phase 1's
findings; this entry retires the parser count, not Phase 1. Whoever restores Phase 1's ✅ must
confirm the II.2 grammar-rule findings (2q, S19) are all closed too, independently of C13.

### Phase 0 ✅ — "delete everything in II.17". Roughly 15% happened.

| marked deleted | actually | evidence |
|---|---|---|
| the `-g` model | ~~the model it anchored is not gone~~ — **now gone (Phase 2r), verified 2026-07-17.** `Config::groups_dir`/`modules_dir` are deleted; `grep -rn groups_dir src/` returns **one doc comment** (`insight.rs:399`), no field. *(The old
`policy.rs:3` citation died with `policy.rs` in the Phase 3 guard consolidation; re-grepped 2026-07-17.)* **`config_root()` is no longer `groups_dir.parent()`** — it returns the `config_root` field directly (`config.rs:416`) with the never-resolve-to-CWD guard. This row's old claim was stale in the "worse than reality" direction; it is retired. | `config.rs:416` |
| `local.txt` (V.1) | ~~alive in 10 files~~ ~~**dead as of Phase 2e**~~ — **the write is dead; the readers are not, and this row named a deletion that did not happen.** `add_package_to_local`, `remove_package_from_local`, `remove_package_from_manifests`, `get_user_group_file` and `ManifestEngine::add_to_local` are genuinely gone, and `model/edit.rs` replaces them — **S9 really did die** (`edit.rs:378` parses via the grammar; test at `:669`), **but of `edit.rs`, not of `local.txt`.** `line_declares` is **NOT deleted**: `insight.rs:418`, live, called at `:447`/`:463`, with passing tests at `:695`. `main.rs:3616` still wrote `local.txt` in `init -i` (being removed in the current working tree). **See the `linix why` entry above — this row is how it hid.** | `insight.rs:418` |
| `keep.txt` (V.6) | ~~alive~~ — **dead as of Phase 2e.** It was never *read*: the whole `RESERVED_MANIFEST_NAMES` / `is_reserved_manifest` mechanism existed only to keep one file out of a crawl. Mechanism and all four exclusion sites deleted. | fixed |
| `_active_profiles.txt` | ~~still written on every `activate`~~ — **dead as of Phase 2f.** `materialize()`, `compose()` (the second profile engine) and `RESERVED_MANIFEST` are deleted; `ProfileManager` runs on the model and `activate` edits one file, `active`. 657 lines -> 348. | fixed |
| `prune` (V.34) | **partly fixed in Phase 2h.** `prune_on_sync`, `prune_scope` and `protect_imperative` are deleted, and sync removes drift by definition. `snapshot prune` stays — V.34 says deleting the command leaves exactly one meaning of the word ("delete old history"), and that is it. `auto_prune` is snapshot retention, the same one surviving meaning. | fixed |
| `migrate` | **696 live lines**, called by `adopt`. Renamed, not deleted. ~~`migrate.rs:283` still tells the user to *"run `linix migrate` again"*~~ — **that message is gone (Phase 2k rewired `adopt` onto the new model);** `grep -rn "linix migrate" src/app/migrate.rs` is empty. The file **grew** (606→696) doing that rewiring, so this is not the pending deletion it reads as — it is live, working code the plan no longer wants to delete. | `main.rs:2153` |
| `clone` (V.33) | ~~a CLI command~~ — the command was gone, but **`fleet::clone` + `install_one` were still `pub` with zero callers**: Phase 0 deleted the flag and left the implementation. Deleted in Phase 2e. | fixed |
| ~884 marketing comments | **≈3,700 comment lines remain** in ≈41,600 lines of `src/` (~8.9%). **The count proves nothing on its own and is not the finding** — this one cannot be greped, so judge it by reading. The finding is that **the new `model/` files are writing spec-narration comments as the rule against them is being implemented**: `model/layout.rs:9` *"the fix for the shape of Monday's bug"*, `:135` *"which is Monday's bug (V.1)"*, `model/resolve.rs:162` *"the cost II.4 accepts knowingly"*. **V.42 is being broken by the code that cites V.42.** | `model/layout.rs:9,135` |

**Each row is one grep.** Fixed when each returns nothing:

```
grep -rn  "groups_dir" src/                                                  # ≈84
grep -rn  "config_root" src/                                                 # ≈25
grep -rln "local.txt" src/                                                   # ≈11 files
grep -rn  "_active_profiles\|RESERVED_MANIFEST" src/ | grep -v ^src/model/   # ≈7
grep -rn  "keep.txt" src/                                                     # live
grep -rn  "prune_on_sync\|auto_prune\|Prune" src/cli/ src/config/             # live
wc -l src/app/migrate.rs                                                      # ≈606, called by adopt
```

> **The counts are `≈` on purpose — do not treat them as a target, and do not "correct" them.**
> They drifted while this audit was being written (`config_root` 23→25, `local.txt` 10→11,
> minutes apart) because **more than one agent is committing to this tree.** A count is a
> tripwire, not a metric. **Only `0` means anything here.** If a number is merely different,
> the finding stands; it is retired when the grep is **quiet**, and never on the strength of
> the number having moved.

Delete each row as its grep goes quiet; restore Phase 0's ✅ when the last one does.

### Phase 2 — the II.1 layout: three of six paths have no reader

`Layout` defines the paths; only `priority` and `active` are actually read. **Zero callers
outside `layout.rs` and its own tests** for:

- `preferences_file()` — `layout.rs:84`. Nothing reads `preferences.toml`. II.10's refusals
  and V.43's nine guard rules have no source.
- `schedules_file()` — `layout.rs:69`. Live scheduling still reads `config.schedules` from
  `config.toml` (`scheduler/mod.rs:101`).
- ~~`locks_dir()` / `lock_file()` — `layout.rs:74-79`.~~ **Partly retired 2026-07-17:
  `locks_dir()` now has one real caller — `main.rs:3448`, which creates the directory in
  `scaffold_dirs`. Creating it is not reading it:** live locks still read the old
  `groups_dir.join("locks.json")` (`sync/resolver.rs:52`), so `lock_file()` remains unread and
  Phase 4 still owns this. **Two paths to one fact, again.**

```
grep -rn "preferences_file\|schedules_file\|locks_dir" src/ | grep -v model/layout.rs
```
**Now: one hit (`main.rs:3448`, `locks_dir`). Fixed when: a real caller for each of the three**
— which is the inverse of every other entry here, so read the result carefully. **A missing
name in that output means it is still broken**, and a name appearing does not mean it is read
— `locks_dir` is proof: it appears, and nothing reads a lock.
This entry is unusual: unlike the rest of the audit it is *unbuilt work, not a false ✅* —
it sits here only because Phase 2 is where it belongs and it is easy to mistake a path
helper with a passing test for a working file.

### `linix why` answers from the old model, and cannot see the new one (found 2026-07-17) — **FIXED**

**Fixed in Phase 2j. The diagnosis was right, including that it outranked C13.** `why` now
asks the resolver — `StateResolver::resolve_model()` — instead of re-reading the files with
its own parser. That is the point: `why` answers *"where is this declared?"*, which is the
question the model exists to answer, so a second implementation here was always going to be a
second answer, and it was the one a user reaches for when they already distrust the state.

- `scan_declarations`, `line_declares` (the 9th `split_once(':')` parser) and
  `interpret_source` are **deleted**, along with the test asserting `@module:` behaviour for a
  syntax this document deleted.
- **The answer is now a file and a line**: `at modules/dev.txt:2 (module:dev, profile:Work)`,
  taken from `__source` and `__scopes`. Verified against the binary on a II.1 repo — the exact
  case it previously could never see.
- **A `why` that cannot read your files now says so** rather than reporting "declared
  nowhere": `declarations_of` returns `Result` and the error names the file and line. Reporting
  a broken config as an absent declaration is the same failure in a quieter voice.
- **"Declared nowhere" is now an answer, not a blank**: *"in no active file — the next `sync`
  will remove it"*. That is drift, and saying it without saying what it means is how a true
  sentence still misleads.
- A lapsed line is reported as lapsed (II.16), not as the reason a package is present.

**The rest of the finding stands and is the next action.** `insight.rs` was one of nine files
holding `groups_dir`/`modules_dir`; 77 references remain across the tree.

**This is the new "most dangerous artifact", and it outranks C13.** Phase 2e's row below says
`line_declares` is **deleted**. It is alive at `app/insight.rs:418`, with its own passing unit
tests at `:695-705` — including `assert!(!line_declares("@module:htop", …))`, a test asserting
the behaviour of a syntax this document deleted.

```
grep -c "layout()\|Layout\|profiles_dir\|active_file" src/app/insight.rs      # 0. Fixed when: not 0.
```
**`insight.rs` does not contain the word `Layout`.** `why` (`insight.rs:477`) calls
`scan_declarations` (`:436`), which crawls `config.groups_dir` for `*.txt` and
`config.modules_dir` for **`*.module.txt`** (`:459`). II.1 modules are `modules/<name>.txt`
(`model/layout.rs:102`). **So `linix why` can never see a II.1 module** — wrong suffix — and it
never opens `profiles/` or `active` at all.

**Why this is the worst one in the section:** resolution runs on the new model (Phase 2d), so
`why` is now the command that *answers the question the new model exists to answer* — "where is
this declared?" — by reading the model that no longer decides. It does not error. It prints a
confident, sourced, wrong sentence, and **it is the command a user reaches for precisely when
they already distrust the state.** A false ✅ makes a reader skip work; this makes the tool lie
to a user who is checking.

**It is not a one-off.** `grep -rln "config\.groups_dir\|config\.modules_dir" src/` → **9 files**;
`services.rs`, `bundle.rs` and `config/manifest.rs` also score **0** for any Layout reference.
"The seam held" is true of `src/backends/`, `src/core/` and `src/parsers/` — the seam was never
what these were on the wrong side of.

### `linix init -i` writes the old model — the ✅ verified one of two paths — **FIXED 2026-07-17**

**Fixed in Phase 2h.** `interactive_init` calls `scaffold_repo`, the `local.txt` write is
gone, and the three prompts for deleted settings are gone with the settings. **The lesson is
kept below verbatim, because it is the part worth keeping**: *"verified by running it"* was
the strongest evidence claim in this document and it was still partial — it ran the path the
author was thinking about.

Phase 2's checklist marks `linix init` done: *"Verified by running it: it produces a repo that
resolves."* True — of `linix init`. **`handle_init` (`main.rs:3429`) has two paths**, and
`interactive_init` (`:3457`) `return`s at `:3434` **before `scaffold_repo` (`:4294`) — the
only thing that writes `priority`, `active` and a profile — ever runs.** It wrote
`groups_dir/local.txt` (`:3616`) and interactively prompted for `prune_on_sync`, `prune_scope`
and `auto_prune`: three settings II.17 deletes and "the next action" is deleting now.

**Being fixed in the working tree as this was written** — the `local.txt` write is already gone
from the uncommitted diff. **Kept anyway, because the lesson is not the bug.** *"Verified by
running it"* is the strongest evidence claim in this document and it was still a partial
verification — it ran the path the author was thinking about. **Rule 11 is usually read as being
about `cargo test`. It is not: it is about any observation standing in for the plan.** A wizard
is a second path, and a second path is a second implementation.

### `activate` does not do what II.6 says (found 2026-07-17 — **being fixed in flight**)

> **Status at the time of writing: the working tree already fixes most of this.** `activate`
> takes `add: bool`, `-a`/`--add` exists, `Switch` is deleted, `parse_active` reads `when`
> blocks, and II.6's empty-names refusal is in the code verbatim. **The owner's four decisions
> (above) match what was built** — they were made from this entry and landed within the hour.
> **Retire the rest of this entry when that commit lands. Two things are not fixed:**
>
> 1. **`activate` overwrites a `when` block and does not say so.** `profile.rs` prints
>    `active is now {names}.` and never names the block it deleted. **Automatic is not silent
>    (S6)** — the owner chose "no refusal", not "no receipt".
> 2. **`deactivate` implements a rule the owner has since reversed.** `profile.rs:189` prints
>    *"Removed {} from the list. It is still activated by the `when {}` block…"* — **that is
>    the behaviour II.6 no longer specifies.** `deactivate` must now remove the name from the
>    matching block too, so that sentence is unreachable by construction. **It was built
>    correctly against the spec as it read at the time; the spec moved.** The replacement
>    message is for the *other* case only — a block that does not apply to this host, which is
>    left alone. `model/profiles.rs:459`'s comment says it exists to support the old sentence;
>    **the machinery it feeds is still needed, for the opposite purpose.** Do not delete the
>    block-awareness — re-point it. *(Related, unjudged: these go
> to `info!`/`tracing`, not stdout. II.6 says the commands **print** what they touched. Whether
> a user with default settings ever sees them is untraced.)*

Not marked ✅ — **worse: II.6 was written as a specification of existing behaviour and it
describes the opposite of the code.**

| II.6 says (`:345`) | `app/profile.rs` does |
|---|---|
| `activate NAME…` — *the file becomes exactly this list* | **adds.** `:78-96` reads `active`, pushes what's missing, writes it back. This is `activate -a`. |
| `activate -a NAME…` — *adds* | **no `-a` flag exists.** `cli/args.rs:334-338` takes `profiles: Vec<String>` and nothing else. |
| — | **`profile switch NAME` (`args.rs:737`, `profile.rs:123`) is the set form**, one name only, and appears nowhere in this document. |

`args.rs:332` documents `activate` as *"add each to the active set"* — **the CLI help and II.6
contradict each other in the same repo.** The refusal at II.6:360 (*"activate needs a profile
name…"*) does not exist; `#[arg(required = true)]` prints clap's generic error. `-r` is
genuinely gone (verified).

**And `active` cannot hold a `when` block, which II.6:330 shows it holding.**
`write_active` (`profile.rs:257-269`) rebuilds the file from a flat `Vec<String>` — **any
activate/deactivate silently destroys every block in it** — and `parse_active`
(`model/profiles.rs:194`) rejects any line with more than one word, so `when host == laptop {`
is a hard error: *"`when host == laptop {` is not a profile name"*. The II.6 example file does
not parse. **Decide which is right before building the verb, not after.**

### VI.1 "killed by this design — no work needed" — two rows are live bugs (found 2026-07-17)

**VI.1 is a list of finished-claims and belongs under this section's rule.** *"Killed by this
design"* is only true once the design is **built**; two rows are killed by designs that are
**not started**, so the bug is live and filed as needing no work — the exact failure a false ✅
causes, wearing a different word.

- **`linix shim --source` — marked "(verified)". It is a live bug.** `cli/args.rs:100` makes
  `--source` **required**; `main.rs:2883` → `context.rs:612`
  `pub async fn create_shim(&self, binary_name: &str, _source_spec: &str)` — **the underscore
  is the bug.** The user is forced to supply a value that is discarded, so
  `shim rg --source cargo:ripgrep` and `--source apt:nonsense` do the same thing. What kills
  it is II.16 (shims as declared lines) — **unbuilt**, and S4 routes the shim path to Phase 3,
  **not started**.
- **E12 `confirm_destructive` gates the wrong thing.** **`module` overwrite fixed in Phase 2k**
  — II.8 says destroying a file you wrote is a plain refusal plus `--force`, and it is now
  exactly that, wired to nothing. **The other two sites are live**: `main.rs:966` and `:1118`
  gate `install`/`uninstall` confirmation, which is the II.10 guard's job — Phase 3, not
  started. `confirm_destructive` itself is in II.17's delete list and dies with them.

**Also contradictory:** E4 says two snapshot retention engines need no work; **S2 routes its own
fix to "Phase 4 (one retention engine)"** — the document says both that the consolidation is
unnecessary and that a bug waits on it. *(A check for the second engine found only one — both
`snapshot.rs:455` and `generation.rs:262` feed `core::RetentionPolicy`. E4 may simply be stale
wording rather than a real pair. **Decide it; do not leave it readable both ways.**)*

**The rule this earns:** VI.1's rows must name the phase that kills each bug, and a row whose
phase is unstarted is **not** "no work needed" — it is *work scheduled elsewhere*, which is a
different sentence with a different reader response.

### The shape of all of it

**The new model was built alongside the old one, and the old one was never removed.** That is
the "two ways to do one thing" disease Phase 2 explicitly forbids — *"do not run two models
behind a flag"* — arrived at by accretion rather than by decision, which is why no one chose
it and no test objects.

**The most dangerous artifact is not a false ✅ — it is a comment asserting a deletion that
did not happen.** ~~`model/profiles.rs:24` states there is no `_active_profiles.txt` and no
materialization. `app/profile.rs:347` writes it on every `activate`.~~ — **retired 2026-07-17:
the comment came true.** Phase 2f deleted the writer; `grep -rn "_active_profiles\.txt" src/`
now returns that comment and nothing else. **The example is retired; the rule it teaches is
not: trust neither the ✅ nor the comment. Grep.** The replacement example is one line down,
and it is worse, because a comment can only lie about one file — **`linix why` lies about the
whole model** (below).

> **The retirement grep in this section was broken and could never have gone quiet.**
> `grep -rn "_active_profiles\|RESERVED_MANIFEST"` matches the *test name*
> `the_seam_carries_what_the_active_profiles_reach` (`sync/resolver.rs:466`) — the `_active_profiles`
> substring sits inside `what_the_active_profiles_reach`. A fixed entry read as unfixed forever.
> **Anchor a retirement grep on the filename (`_active_profiles\.txt`), never the bare stem** —
> a tripwire that cannot go quiet is not a tripwire, it is a permanent false finding, and this
> section's whole ritual is "run it and believe the result."

**Recommended order, given the above:** finish the deletions **before** II.8, not after.
Every verb added on top of the six non-validating parsers, `groups_dir`, and `local.txt` is a
verb that has to be rewritten when they go — and II.8 is the biggest surface in the plan.

## The one thing to understand before continuing

**The model now runs resolution.** `StateResolver::resolve_desired_state` builds the II.1
`Layout` from config, loads `priority`, runs `model::Resolver`, and probes bare names. The
seam signature is unchanged, so `src/backends/`, `src/core/` and `src/parsers/` never
noticed. `tests/e2e_tests.rs` now drives module → profile → resolve → plan → execute →
registry on the new model.

**The branch did not stay red, and that is not a warning sign** — the seam held. What broke
was six tests written against the deleted model (`@module:`, `.module.txt`, the `groups_dir`
crawl, `manifest:` tags). Each was ported to the II.1 layout with its guarantee intact, not
deleted.

**Two things the wiring changed that the plan did not anticipate.** Both are in VI.2:

- **S13 (fixed).** Probing had to move *between* reading the files and merging them. Keyed
  `?:ripgrep` until probed, a bare `ripgrep` never met an explicit `cargo:ripgrep`, so rule
  5 silently did not apply to bare lines and both were installed. `model::Resolver` now
  splits: `statements()` → the caller probes → `with_bare(answers)` → `collect()`.
  `resolve()` still does both, for callers that need no probe.
- **`__scopes` is new, and `__source` changed meaning.** They were one tag doing two jobs,
  which is how `upgrade --module dev` came to be matched against a filename. `__source` is
  now `file:line` — what II.8's messages and every error need. `__scopes` carries
  `module:dev;profile:Work` — what `--module` / `--profile` match on. The model writes it
  because it is the only thing that knows profile→module. `collect` keeps **every** origin
  of a merged package, not just the winner's: the loser's scope is not thereby untrue.

## Where Phase 2 is holding (2026-07-17, after Phase 2v/2w)

**The Phase 2 checklist is complete.** Every box under "Phase 2's remaining checklist" is
now `[x]`. What landed this session, each committed and verified against the binary or with
tests, clippy silent throughout (≈521 tests):

- **S12 done** — all three ordering phases: `repo:` before packages (`apply_repositories`),
  packages, then `shim:`/`service:`/`link:` after (`apply_dependents`), in declaration order.
  `watch` runs the same three phases. (2o–2p)
- **Grammar's last II.2 gaps closed** — `@until` refused on present lines; `link:` no longer
  eaten by set-math precedence; the option-key whitelist (S19) confirmed. (2q)
- **Old-model teardown** — `config/parser.rs` 437→215 (the `@module:`/`groups/` crawl gone);
  `Config::groups_dir`/`modules_dir` fields replaced by one `config_root`; the retired `lease`
  command deleted; the `config.toml` backend-selection cluster (`enabled_backends`,
  `hostname_backends`, `backend_priority`, `default_backend`) deleted and `search`/`rollback`/
  `repo`/planner-drift routed through the `priority` file. (2r–2t)
- **Cycle errors name the cycle** (V.45) — the planner reports `apt:foo (file:line) -> apt:bar
  -> apt:foo` via Tarjan SCC. (2u)
- **II.8 read-only surface** — `status`/`list`/`plan` reviewed; `unmanaged` is E6-fixed; the
  two missing verbs `check` and `absent` were built. (2v)
- **S14** — `init` no longer lists `service`/`link` in the generated `priority`. (2w)

**Everything found here is now folded into the plan — nothing is left as loose prose:**

1. **S20 — drift for extras → Phase 4.** A `service:`/`link:`/`shim:` line the user *removes* is
   not reconciled away; it needs a durable applied-extras ledger to diff against, which is the
   same state machinery `locks/` introduces. Forward direction (apply) is done. Tracked in VI.2.
2. **S21 — `schedule:` wiring + its file-context rule → Phase 5.** The resolver never reads the
   `schedules` file; `schedule:` only lands in `extras` and `sync` warns. Wiring it (read the
   file, provision via `SchedulerManager`, enforce "only in `schedules`") is one Phase 5 job.
   Tracked in VI.2.
3. **The green-harness exit → Phase 5/6.** Resolved in the Phase 2 exit note above: the harness
   rebuild belongs to Phase 5, so the gate is carried there rather than run against a harness
   that asserts the old surface. The model-side of Phase 2 is complete and verified.
4. **Grammar-comment accuracy — the "II.2's full list" comment is FIXED (Phase 2x);** the
   `schedule:` file-context half is folded into S21 (it cannot be checked until `schedule:` has
   a home).

**So Phase 2's *build* is done.** The two remaining functional gaps (S20, S21) are genuinely
later-phase work by their dependencies, not Phase 2 corners cut — each is tracked with its
reason and its phase.

## Done in Phase 2l — `uninstall`, and the symmetric pair is symmetric again

**`uninstall` obeys P1 (S15 closed).** It removed the package first and edited the file
after; now the file edit IS the command and sync carries it out — so the removal goes through
the guard, the plan and the counts like every other removal, rather than reaching for the
backend directly and asking the guard on the side. `install` and `uninstall` are the
symmetric pair V.39 describes again.

**Both of II.8's `uninstall` rules are built, and verified against the binary:**

- *"jq is still declared in module `gaming`, which isn't active. It will come back if you
  activate Gaming."* — deleting the line you can see while an identical line waits in a
  module you forgot about is a package that returns the next time you switch profiles.
- *"steam isn't declared, so there's nothing for it to come back to. Did you mean a plain
  uninstall?"* — `--temp` on something undeclared.

**`uninstall --temp=2h` is a suspension now (II.16, V.37).** It writes
`absent:cargo:ripgrep@until=2026-07-17T16:17` and the model does the rest: a dated line beats
an undated one (II.7 rule 6), so the module that wants it loses until the date passes, and
then wins again. **Verified end to end** — the package is not wanted while suspended and is
wanted again once the date is in the past. "Take the game away until the weekend" works, with
no timer and no sweep: the same dated-line machinery `install --temp` uses, pointed the other
way.

Bare `--temp` (restore on shell exit) stays outside the model on purpose (II.8): a shell
session is not a declaration, so it writes no file — and it now calls the guard, which it
did not before (C1).

**Found: S19** — `@lease=2h` still works by hand, and it is the one option key that can
uninstall your package, on a path C3 says bypasses the guard.

## Done in Phase 2o — repos, the first ordering phase

**`repo:` lines are applied (S12, partly).** They resolved and were dropped; now `sync`
resolves the whole `DesiredState` and `App::apply_repositories` runs the `repo:` lines FIRST
— add the repository, then refresh that backend's index — before the package plan. This is
II.7's ordering, and it is the one that decides something: a package from a PPA cannot
install until the PPA is added and `apt update` has seen it. Verified against the binary: the
add and the refresh log before the flight plan.

**The owner decided how a `repo:` names its backend (V.47):** explicitly, like a package line
— `repo:apt:ppa:deadsnakes/ppa`. A repository belongs to one manager, and guessing runs the
wrong system command. A bare `repo:ppa:...` is a parse error, and a backend not in `priority`
is refused in the file (V.15) — both caught before any command runs.

**~~Still owed on S12~~ — done in Phase 2p (below):** `shim:`, `service:` and `link:` as the
*dependent* phase, applied AFTER packages. `schedule:` belongs to the scheduler, not the sync
ordering, and stays owed.

## Done in Phase 2p — dependents, the third ordering phase (S12 done)

**`shim:`, `service:` and `link:` are applied AFTER packages (S12 closed).** `App::apply_dependents`
is the mirror of `apply_repositories`: it walks `DesiredState::dependents()` — the extras that
lean on a package — and dispatches each. A `service:` becomes a `service`-backend install
(`systemctl enable`, `sc config`, `rc-update add` — whatever the host's init is); a `link:`
becomes a `link`-backend install (symlink / rendered template / decrypted secret / managed
content); a `shim:` deploys through `ShimManager`. They run in **declaration order**, so a
config `link:` written above the `service:` that reads it keeps that order.

**Why a distinct phase and not part of the package plan:** each one presupposes a package —
a shim wraps a binary that must already be on disk, a service enables a unit a package just
installed, a link writes the config a package expects. So they wait for the whole package
plan to finish; they cannot be interleaved. `DesiredState` grew `dependents()` (the three
after-package extras, in order) and `has_dependents()`, and `sync`'s "System matches" exit
now checks the latter — a config that is all `service:`/`link:`/`shim:` and no package
changes is still work, and previously would have exited "nothing to do".

**Verified against the binary.** A repo + package + `service:` + `link:` + `shim:` + `schedule:`
config, `sync --dry-run`: `schedule:` warns by file and line, the Flight plan installs the
package, THEN service → link → shim preview — in that order, after the packages. A
dependents-only config previews the three and does *not* say "System matches"; a truly empty
config does. Tests: `dependents_are_the_after_package_extras_only` and
`a_config_with_no_extras_has_no_dependents` (model), `dependents_only_config_resolves_and_applies_the_dependent_phase`
(integration, hermetic). 528 pass, clippy silent.

**`watch` runs the full ordering too.** The unattended reconcile resolved packages-only
(`resolve_desired_state`), so it silently skipped *both* phase 1 (repos) and phase 3
(dependents) — an unattended machine would never add a PPA or enable a `service:` until
someone ran `sync` by hand. It calls `resolve_model` + `apply_repositories` +
`apply_dependents` now, the same three phases as `sync`.

~~**Still owed (not S12's forward direction — one new, smaller item):** *drift for extras*.~~
**Done.** The teardown landed with the applied-extras ledger (S20); `status` learned to preview
it on 2026-07-21 (seventh session), which was the last piece.

## Done in Phase 2q/2r/2s — the grammar's last gaps and the old-model teardown

**2q — the last two unenforced II.2 grammar rules.** The 2026-07-17 audit found three II.2
rules whose implementation was a comment, not a check. One (`@versionn` key whitelist) was
already fixed by S19. The other two are now enforced: **`@until` is refused on a present line**
(`validate_options` takes `absent: bool`, threaded from `parse`'s `absent:` branch, and points
you at `@expires`), and **`link:` no longer parses as set math** — the expression check yields
to any typed-statement prefix, so `link:C:\Users\me\.vimrc` (full of `\`, which
`looks_like_expression` fires on) is a `Statement::Link` again. Tests for both.

**2r — the old-model crawl is gone.** `config/parser.rs` 437 → 215 lines: the whole
`@module:`/`groups/` crawl (`ManifestLine`, `identify_line`, `parse_group_file`,
`filter_conditional_lines`, `load_all_packages`, `write_group_file`) deleted with its 7 tests;
kept `HostFacts` + `eval_when` (the model's `when` engine) and `split_removal_target` (the one
colon-parser that validates against the registry). **`Config::groups_dir` and
`Config::modules_dir` fields deleted** — one `config_root` field replaces them, `config_root()`
returns it directly (was `groups_dir.parent()`) with the never-resolve-to-CWD guard kept, and
all 29 call sites (locks, policy, generation restore, `watch`, the doctor) moved onto it.
Binary verified against a bare `config_root` with no `groups/` subdir.

**2s — the retired `lease` command is deleted.** II.16 replaced leases with dated lines and
S19 stopped `state.add` reading `@lease`; the imperative `lease set`/`lease list` was the last
of the concept, and `lease set` was the last non-validating `split_once(':')` parser (C13). Gone:
`Commands::Lease`, `handle_lease`, `LeaseArgs`/`LeaseCommand`, `StateRegistry::update_lease`.
`install --temp`'s help now says `@expires`, not `@lease`.

## Done in Phase 2j/2k — the commands that still read the deleted model

**`why` asks the model** — the audit's worst finding, answered in full above.

**`adopt` writes `modules/adopted.txt` (II.9).** It wrote `migrated_<timestamp>.txt` into
`groups_dir`: a folder nothing reads any more, under a name that made **the second `adopt`
declare every package twice** — which the resolver then refuses as a contradiction (II.7 rule
5). One file, overwritten, so adopting again answers "the machine as it is now" rather than
"the machine plus history". It goes through the editor, so `use adopted` reaches the active
profile and says so.

**`module` runs on II.1.** It listed `*.module.txt` — a suffix II.1 does not have — so
`module list` printed **nothing** on a real repo, and `module show`/`create` addressed files
the resolver never reads. Now `modules/<name>.txt`, and **the folder decides**: `module list`
asks `ModuleLoader::available()`, so a `README.md` in `modules/` costs nothing (II.3).

**E12 is fixed for `module` (II.8).** Overwriting a module you wrote was gated on
`confirm_destructive` — a setting about *package removals* deciding whether your file
survives, which is one prompt meaning two unrelated things. It is now a plain refusal plus
`--force`, like every other tool, and wired to nothing.

**`bundle` copies your repo, whole.** It copied `groups/` and `modules/` by name, so under
II.1 it silently left `profiles/`, `active`, `priority`, `locks/` and `preferences.toml`
behind — a bundle that restores half your declarations is worse than one that fails.

**Found, not fixed: S18** — `auto_lock_checksums` rewrites your module files on every sync,
and defaults to true. It is Phase 4's thread (a checksum is a lock), and it is the last
caller of `ManifestEngine` and the last real reason `groups_dir` exists.

## Done in Phase 2i — `activate` does what II.6 says, and `active` gained `when`

> **CORRECTED 2026-07-20 by audit. This entry said it answered the audit's finding "in full."
> It did not, and the audit entry it overwrote was right.** Two of that entry's items were
> still open at `e406924`: `activate` did not name the `when` blocks it removes (S6), and
> `deactivate` still implemented the rule the owner reversed on 2026-07-17 — it edited
> top-level lines only (`profile.rs:177`) and still printed *"It is still activated by the
> `when …` block"*, the sentence II.6 requires to be unreachable.
>
> **Both are fixed in the session dated 2026-07-20 at the top of Part VII (findings 2 and 3),
> and this time the behaviour is unit tested** — `remove_from_active` is a pure function, so
> "a block for another host is never touched" is an assertion rather than a claim. **The
> correction is left standing above the paragraph below**, because the paragraph is what a
> reader would otherwise believe, and the failure mode this document keeps catching is a later
> confident sentence burying an earlier accurate one.

Answered most of the audit's `activate` finding; the details are in that section, marked
FIXED. The shape of it: **II.6 described the opposite of the code**, the CLI help contradicted
II.6 in the same repo, and `profile switch` — the set form under a second name — was not in
this document at all. `activate` sets, `activate -a` adds, `switch` is deleted, and the empty
list gets II.6's refusal instead of clap's.

**`active` holds `when` blocks.** It was the one file that broke II.2's "one rule, everywhere"
— `parse_active` rejected any line with more than one word, so **II.6's own example file was a
hard error**, and `write_active` rebuilt the file from a flat list so any activate/deactivate
silently destroyed every block in it. `read_active` now gates them, `deactivate` edits
top-level lines only, and a name a block also holds is reported rather than silently ignored.

## Done in Phase 2h — the guard, and sync becoming sync

**Two live safety bugs, both contradicting II.10's "nothing overrides". S16 and S17 in
VI.2.** `--allow-mass-removal` cleared every objection including protected and OS-essential;
`[guard.enforce_on]` was a config key that switched the guard off per command. **The first
had a test asserting the broken behaviour** — the bug was written down as an expectation,
which is why it survived review. Both fixed, with the tests II.10 actually implies.

**`prune_on_sync`, `prune_scope`, `with_prune` and `protect_imperative` are deleted, and
sync removes drift because that is what sync IS (V.34).**

- `prune_on_sync` made "does sync converge?" a setting. A sync that does not remove drift is
  not sync; it is `prune` with the install half amputated, which is the thing V.34 deleted.
- `prune_scope = "system"` was **the setting V.21 forbids in so many words**: flip it and a
  routine sync deletes software LiNix never installed. That is `purge-unmanaged` — **a
  command you type, not a mode you inherit** — which is why deleting the mode and building
  the command had to be one change. Deleting it alone would have removed a real feature.
- `protect_imperative` guarded against a bug that no longer exists. An imperative install
  had no line, so it read as drift the moment it was recorded; it has a line now
  (`modules/imperative.txt`), so it is declared like everything else.

**`purge-unmanaged` is built (II.11).** The ratio, not the count (V.20 — on Alpine a count
misses 14-of-14); `max_removals` does not apply because it catches accidents and this is
deliberate; protection and OS-essential still do; snapshot first, automatically, and **"THERE
IS NO UNDO FOR THIS" when none can be taken**; the whole list, every line, because the pain
is the feature; typed confirmation.

**Guided setup lost four questions that stopped existing**: "should sync remove drift?" (it
is drift removal), "how aggressive?" (that is `purge-unmanaged`), "protect imperative
installs?" (they are declared now), "preferred default backend?" (that is `priority`,
detected). A question whose answer LiNix can work out is homework (V.41).

## Superseded — the note this section carried before

This section previously warned that making sync remove drift by default was "the shape of
this repo's flagship bug". **That was overcautious, and reading V.21 and II.7 carefully is
what corrected it.** Sync removes *what it manages and you stopped declaring* — bounded by
the registry, not by the machine. The flagship bug was the opposite shape: `-g` moved the
wish list while the registry stayed put, so owned-but-unwished read as drift (V.1). The
dangerous thing was never sync converging; it was `prune_scope = "system"`, which let sync
reach outside what it owns. That is now impossible rather than merely off by default —
which is what V.21 means by "not safe by default, but safe permanently".

## Done in Phase 2f — set math, and the last of the old profile engine

**`app/profile.rs` was the last whole copy of the old model.** 657 lines that still write
`_active_profiles.txt` on every `activate`, and still hold `compose()`: a second, complete
profile-composition engine duplicating `model/profiles.rs`. Its `active` file is in the wrong
place (`profiles/active`, not the config root) and its profiles are `<name>.profile`, not
II.5's Capitalized `profiles/<Name>`. Nothing about it agrees with the model that now runs
resolution.

It is now 348 lines and runs on the model. `materialize()` is gone (a materialised copy is a
second place the same fact lives), and `compose()` — a second complete profile engine — is
gone with it.

**II.4's set math is implemented, and the decision behind it is V.46.** It was specified and
never built: `ProfileLoader::resolve` handled only `use`, and `evaluate_expression` had no
caller outside its own tests. It now works end to end — verified against the binary, not just
tests. `exclude`, `intersect`, `-pkg` and full expressions like `(Work | gaming) & security`,
with II.4's fixed order: gather, narrow, subtract, and **subtraction always wins** whatever
order you wrote the lines in.

**V.46 predicted a cost that turned out not to exist, and the entry says so.** The prediction
was that set math would cost a package its module name. It does not: atoms map back to the
**statements** they came from, so a package keeps its `Origin`, its file, and its module.
`upgrade --module editors` still finds `vim` through an `exclude`.

Also settled there: **`include` is gone** (V.46). `use` already meant union, and two words for one
operation is the disease this design cures, sitting inside the spec. `include x` is an error
that says `use x`.

## Phase 2's remaining checklist

- [x] Wire `model::Resolver` into `StateResolver`.
- [x] **`local.txt` and `keep.txt` are deleted, and S9 with them.** `model/edit.rs` is the one
      writer now: `install`, `uninstall`, `forget`, `teleport`, `service enable/disable` and
      the package-manager hook all go through it, and it parses every line with the grammar
      rather than splitting on `:`. `--into` exists (II.8). The three landing modules exist,
      and the first write to one adds `use <name>` to the active profile and says so.
- [x] **`linix init` writes the II.1 repo** — `priority` generated from what this machine
      actually has (V.41), ordered by V.14's one real rule, with its reason in the file (P5).
      Verified by running it: it produces a repo that resolves. **S14 fixed (Phase 2w):**
      `starter_order` now drops `service`/`link` (dependent statements, not managers, never
      priority-gated). `web`/`github`/`appimage` stay — the model refuses an explicit `web:…`
      unless `web` is listed (V.15), so excluding them would break those specs; S14's lumping
      them in was imprecise.
- [x] **S12 — done (Phase 2o + 2p).** `repo:` applies before packages (`apply_repositories`),
      `shim:`/`service:`/`link:` apply after (`apply_dependents`), in declaration order. The
      extras that used to be dropped at the seam are now walked off `DesiredState::extras`
      around the package graph. Only `schedule:` still warns by file and line — the scheduler
      owns it. Follow-ups (extras-drift, `watch`) noted in "The next action, precisely".
- [x] **`local.txt` and `_active_profiles.txt` are deleted** (Phase 2e, 2f). S9 died with
      `local.txt`. *Corrected: Phase 2e's commit said `line_declares` was deleted; that was
      true of `config/parser.rs` and **there was a second copy in `insight.rs`** — deleted in
      Phase 2j. "Deleted" means the grep is empty, not that the copy you were looking at is
      gone.*
- [x] **The `config.toml` backend-selection cluster is deleted (Phase 2t).** `priority`
      (II.6) replaced four settings that said one thing between them; the model read the file
      but `search`, `rollback` scope, `repo`'s default, and the planner's drift gate still read
      the config fields. Now all read `priority`: `App::priority_backends()` feeds `search` and
      `rollback`; `ChangePlanner::with_enabled(..)` scopes drift so a backend you stop listing
      is left alone, not reaped (imperative paths/tests default to "all"). Deleted:
      `enabled_backends`, `hostname_backends`, `backend_priority`, `default_backend`,
      `effective_enabled_backends`, `is_backend_enabled`, `default_priority`, and their tests.
      *(No global `-b` flag exists — the planner comment claiming one was stale.)* Other stale
      `config.toml` example keys (`bloatware_file`, `[managed_files]`, `[hostname_packages]`)
      are Phase 5 docs debt, not live fields.
- [x] **The old-model crawl is deleted (Phase 2r).** `config/parser.rs` 437 → 215 lines:
      `ManifestLine`, `identify_line`, `parse_group_file`/`parse_group_str`,
      `filter_conditional_lines`, `load_all_packages`, `write_group_file` — the whole
      `@module:`/`groups/` crawl the model replaced — all gone, with their 7 tests. Kept
      `HostFacts` + `eval_when` (the model's `when` engine, 5 live refs) and
      `split_removal_target` (the one `split_once(':')` that consults the registry, so *not* a
      C13 non-validating parser). **`Config::groups_dir` and `Config::modules_dir` fields are
      gone too** — a single `config_root` field replaces them, `config_root()` returns it
      (CWD-guard kept), and all 29 call sites moved to `config_root()`/`config_root().join(..)`.
      *Remaining C13 surface:* the grammar is the one **statement** parser now, but a couple of
      colon-splitters survive for non-statement jobs (`main.rs:679` a `requires` target,
      `ecosystem.rs:275` a `name:ver`) — neither reads a backend name, so neither is the C13 risk.
- [x] **E6** — `unmanaged` is one function now, *"what `adopt` would adopt"* (Phase 2m).
      **E7** fixed with it: adopt takes protected packages; only OS-essential is held back.
- [x] **Ordering phases: repos → index refresh → packages → dependents (Phase 2o + 2p).**
      Applied by `sync` around the package graph, not inside `core/transaction.rs` (the seam
      keeps `core/` ignorant of repos and extras). Verified against the binary.
- [x] **Cycle detection error text (V.45) — done (Phase 2u).** Detection was already right in
      all three places; only the messages were owed. The two `use` loops (`profiles.rs`,
      `modules.rs`) already name the path (`a -> b -> a`). The planner's was *"Circular
      dependency detected in graph construction."* — naming nothing — and now names the
      mutually-dependent packages **and their file:line**, `apt:foo (modules/dev.txt:3) ->
      apt:bar (…) -> apt:foo`, via Tarjan's SCC (self-loops handled separately). As predicted,
      `PackageSpec` needed no `Origin` field: `options["__source"]` is `file:line` and the
      planner already read it. Tests for the 2-cycle and self-loop cases.
- [x] **The II.8 command surface is built.** `install` (P1 order, `--into`, `--temp` ->
      `@expires`), `forget`, `teleport`, `service enable/disable`, the hook, `purge-unmanaged`,
      `activate` / `activate -a` / `deactivate` / `why`, `uninstall` (P1 order, both II.8
      warnings, `--temp` -> `absent:@until`), `module`, `adopt`, `bundle`. **Read-only verbs
      reviewed against II.8 (Phase 2v):** `status`/`list`/`plan` are read-only and correct,
      `unmanaged` is E6-fixed (*"what adopt would adopt"*), and the two II.8 verbs that did not
      exist — `check` (*"parse everything, report errors"*) and `absent` (*"every absent: line
      in force, and its module"*) — were built. Verified against the binary: `check` flags a
      backend missing from `priority`; `absent` lists each absent spec with its `__source`.
      *(The retired `lease` command was deleted in 2s.)*

## Decisions the owner has made — do not re-open

| | |
|---|---|
| **V.43** | Keep every guard refusal, including the three orphaned `policy.toml` rules (`pinned_only`, `require_snapshot`, `deny_vulnerable`). II.10's "five" was wrong. |
| **S6** | `sync` heals **automatically**. Asking permission to fix drift asks permission to do sync's own job. Automatic ≠ silent: it must say what it did, and the removal still goes through the guard. |
| **S8** | Keep `undo`. Keep its path check (renamed to say it guards the snapshot-read path). Delete the false global claim. Restore must state it rolls back the whole filesystem before asking. |
| **II.6 verbs** (2026-07-17) | **Three verbs, as II.6 already said: `activate` SETS, `activate -a` ADDS, `deactivate` REMOVES.** The code had `activate` adding and the CLI help documenting it that way — **the spec was right and the code was wrong.** Not a re-opening: II.6 was already correct, the audit found the drift. |
| **`profile switch`** (2026-07-17) | **Dies.** Once `activate` sets, `switch NAME` *is* `activate NAME` with a worse name and a one-name limit. It was the set form only because `activate` had wrongly taken the add form's job. **Two ways to do one thing** (P1). |
| **`when` in `active`** (2026-07-17) | **`active` holds `when` blocks.** `when` gates every other file (II.2); `active` being the exception was an accident of `parse_active` rejecting any multi-word line — which made **II.6's own example file fail to parse.** One rule, everywhere. |
| **`deactivate` vs blocks** (2026-07-17) | **`deactivate` removes the name from the top level and from every `when` block that applies to this host** — empty blocks go with it, and it says so. **Reverses II.6's old *"it is still activated by the `when` block on line 4"* bullet**, which described a verb that removed the line and left the thing on. **A block that does not apply to this host is never touched**: nothing there is active, so there is nothing to deactivate, and `active` is a shared file — editing another host's block from this one changes a machine you are not at. It says why and changes nothing. **This is the one place `deactivate` edits a block and `activate -a` does not: adding has a choice of where to put the name; removing does not.** |
| **`activate` vs blocks** (2026-07-17) | **`activate` overwrites the file, blocks included.** It is the set form; it sets, and a block is part of what the file says. **`activate -a` and `deactivate` never touch a block** — they are the surgical pair, and that asymmetry is why `-a` exists. **It does not ask** (declarative: overwriting the list is the command's job) **but it does not do it silently** (S6) — it names every block it removed. *Asking and reporting are not the same thing, and the argument against the first is not an argument against the second.* |

## A warning about this document

**Every "(verified)" / "(measured)" fact in this spec that has been checked has been wrong —
ten for ten, always under-reporting.** They were measured against an older tree. Corrected
so far: the comment count (139 → 884 + 32 false), both good-comment exemplar citations, the
parser count (5/3 → 8/6 → **9/6**, wrong twice, the second time by a re-measurement that was
itself called "(re-measured)"), the backend count, and — the expensive one —
**"`reconcile_shims` is written and never called (verified)"**, which was false and was the
sentence hiding a bug that made every `sync` delete the user's own files out of
`~/.local/bin` (S1). **The 2026-07-20 audit added three more**: the artifact module's "59 unit
tests" (66), "575 lib tests" (650), and "561 passing" (718). *Note that these three were each
written on the day they were measured. A count in this document is stale within one session,
which is the argument for citing the command rather than its output.*

**And the audit's own "718 passing" was stale by the end of the same day (735).** That is
eleven for eleven, and the eleventh was written by the pass whose whole purpose was to catch
stale numbers. **Stop writing the number.** Any sentence here that needs a test count should
name `cargo test` and let the reader run it; the count in the status line is kept only because
a reader wants one glance at whether the tree is green, and it is wrong by the next session
either way.

**The ✅ markers fail the same way, and worse.** Phases 0 and 1 were both marked complete
while untrue (2026-07-17 audit, Part VII) — under a rule, already written at the top of this
document, that says *"never describe unverified work as done."* **The rule was not enough.**
A wrong measurement makes a reader over-confident; **a wrong ✅ makes a reader skip the work
entirely**, and nothing downstream will object, because the code it should have deleted is
still there passing its tests.

**The bias was one bias, in two costumes: this document flatters the codebase.** Measurements
understate the mess; status markers overstate the progress.

> **The 2026-07-17 second pass produced the first counter-examples, and the rule has to change.**
> `_active_profiles.txt` was **fixed and still filed as broken**; C13's skipper count was **six
> and is three**; `resolver.rs:212` was cited as a non-validating parser and **no longer parses
> anything**. In each, the **tree was better than the sentence** — the first time that has ever
> happened here.
>
> **This is not good news and it is not permission to relax.** The old rule ("assume the tree is
> worse") was a *heuristic that let you skip checking*, and it has now failed in both directions,
> which is the only thing that could have shown it was always the wrong kind of rule. **The
> document is not biased pessimistic or optimistic. It is stale**, and stale drifts whichever way
> the tree moved. **The replacement rule is not a direction, it is a refusal to guess: no claim
> here is evidence of anything. Run the command.** Note which way this one broke — a *fixed* entry
> read as broken costs someone a day re-fixing it, and a *stale* count read as a target
> (**"Fixed when: 1"**) would have had them delete a working validator to hit a number.

**Line numbers rot faster than claims — and this very example rotted again.** V.29 cites
`planner.rs:407-426` for `spec.requires`; **this document corrected that to `:431`, and it is now
`:384`.** The *reasoning* is still correct. **The correction needed a correction inside one
session** — which is the argument, not a footnote to it. Treat a citation as a place to start
looking, never as proof — **grep the symbol, don't trust the line.** Every cycle-detection
citation on Phase 2's checklist has rotted the same way (`planner.rs:323` → `:276`,
`profiles.rs:62` → `:88`, `modules.rs:146` → `:165`); the claim they support — *"already caught
in all three places, do not rewrite"* — **re-verified true.**

**Four tests have now encoded a bug as an expectation** — asserting the broken behaviour so
review saw a green suite and moved on: `enforce_refuses_without_opt_in_and_proceeds_with_it`
(the guard letting `--allow-mass-removal` delete python3, S16), the `prune`/`protect_imperative`
gate tests (sync configured not to converge), and `protected_packages_are_never_adopted` (E7).
**A test named for the behaviour is not evidence the behaviour is right** — it is evidence
someone wrote the code and the test together, believing the same wrong thing. When a test's
name asserts a Part II rule, check it against Part II before trusting it.

**Verify any Part II–VI citation against `HEAD` before you act on it.** Part V's *reasoning*
has been consistently right and is worth following; its *measurements* are not.

## Bugs found while implementing

**S22 — an empty-result banner parsed as a package (found 2026-07-19, fixed).** `pixi global
list` on a machine with nothing installed prints `No global environments found.`, and
`ecosystem::pixi_list` took the first token of every line — so LiNix reported a package named
`No` in the `pixi` backend. It showed up in `linix status` as unmanaged drift, which means
**`adopt` would have written `pixi:No` into a manifest and `purge-unmanaged` would have tried to
delete it.** The fix is in the shared `is_noise_line`, so it covers every parser that takes a
first token, not just pixi: a line reading as prose ("no …", ending in a period, more than two
words) is not an identifier. Covered by `an_empty_result_banner_is_not_a_package`, which also
pins that a real package named `nodejs` still parses.

**Found by running the tool while writing its README** — not by a test, and not by reading. The
suite was green with the phantom package in it, because no test runs a real backend that has
nothing installed.

**S23 — a format legend parsed as two packages (found 2026-07-21, fixed).** `nimble list
--installed` on a machine with nimble but no nimble packages prints the *shape* of its output:

```
Package list format:
{PackageName}
└── @{Version} ({CheckSum})[Special Versions (if any)] ({InstallPath})
```

`linix list` reported `nimble:{PackageName}` and `nimble:└──`. This is S22 exactly one manager
later, and S22's filter did not catch it because the banner is not a sentence: the fix adds two
more rules to the same shared `is_noise_line`, so every first-token parser gets them — **a line
whose first token opens with `{` or `<` is a placeholder**, and **a line starting with a
tree-drawing character is decoration**. `winget`'s real `ARP\Machine\X64\{GUID}` names still
parse, because the brace is not the first character.

**Found the same way S22 was: by running `linix list` on this machine, not by a test.** The
suite was green with both phantoms in it. Every parser test feeds output someone typed into the
test; no test asks a real manager that has nothing installed what it prints.



**S1–S11 in VI.2 → "Found during implementation".** Each is assigned to the phase that owns
the mechanism. Four were live defects already fixed (S1 shim deletion, S10 tests writing to
the real data dir, and two parser bugs); the rest are scheduled. Add to that table rather
than to a commit message — a bug recorded only in a commit message is a bug nobody will find.

## Not started, and owed

~~`README.md` (28k) still documents `-g`, `prune`, `clone` and `migrate` — **all four deleted in
Phase 0.** `CHANGELOG.md` likewise. Both are Phase 5 (docs), and both are cleanly separable
from the code work if a second session ever runs in parallel.~~ — **DONE 2026-07-19. Both were
rewritten, not patched.**

The README described v5/v6: `prune`, `clone`, `migrate`, `generation`, `teleport`, `shim`, the
`-g` flag, a grammar that no longer exists (`@module:`, `group:`, `include:`, `when … end`,
`-pkg`), paths that no longer exist (`groups/local.txt`), and — the one that mattered — **"sync
never removes anything by default", which is the exact opposite of the model.** A patch pass
would have left a document that was wrong in a subtler way, so it was rewritten against the
real `--help` and a real `linix init`. Every command it names was checked by running
`linix <cmd> --help`; **it carries no backend count**, because the count is platform-dependent
(43 registered on the Windows box this was written on) and a typed number is the thing that has
gone stale seven times in this document. It points at `linix doctor` instead.

The CHANGELOG's `[Unreleased]` section advertised a "v7 feature wave" of things that are now
deleted — `lease list`, `managed strict`, the `groups/keep.txt` keep-list, `cockpit`,
`generation rollback --with-config`, `local.txt`. All four of those commands answer GONE to
`--help`. It was replaced with an honest v7 section (the model, safety, what was removed and
why, what was fixed); the released 6.0.0 / 5.0.0 history is real history and was left alone.

**Writing the README found a live bug and a wrong assumption of my own.** The bug is S22 below
(an empty-result banner parsed as a package). The assumption was mine, and it is worth recording
because it is the documentation version of every false ✅ in this file:

**The README's opening example was wrong, and I wrote it from the spec rather than from the
tool.** It showed a module file and a `sync` that installed it — but **a module in `modules/`
is inert until an active profile `use`s it**, so the example as written installs nothing.
`linix check` says `0 present` and gives no hint why. I found it only because I ran the quick
start end to end in a scratch config instead of trusting what I had just written; the corrected
README calls the indirection out explicitly and points at `check` as the way to see it.

*Part II says this plainly (II.3: a module is a list of lines; II.4: profiles choose). I read
it, summarised it, and still produced an example that does not work — which is the exact
failure mode rule 9 describes, in prose instead of a ✅.*

---


---

## Round 7 — the coverage round (2026-07-30)

The owner's instruction was one sentence: *build the test and harness for all of it, to make
sure it really works.* This is `Q4` being executed rather than cited.

**The number was worse than any document said.** Cross-referencing both harnesses' canary and
exemption tables against the union of the two registries — 60 distinct backends — **20 had never
completed a real lifecycle in any harness, and 12 were in neither table of either one.** No
coverage and no stated reason. Nothing could have reported this before, because each sweep
audits only its own registry: `winget`, `choco` and `psresource` are absent from the Linux
registry entirely, so the question *"is winget lifecycled anywhere?"* was asked by nothing at
all. An excuse on the only harness that can run a backend looks exactly like coverage.

**Two defects fell out of building the harness, and neither could have fallen out of reading.**

`psresource` was compiled on Windows only — `pub mod psresource` still carried
`#[cfg(target_os = "windows")]` — which also made it the one OS-native backend that *could not*
appear in the argv table, since the row would not compile on the platforms that need it most.
`registry.rs`'s own doc comment records that this class was fixed on 2026-07-26. It was fixed for
`mas` and for `apt`. `psresource` survived, with no argv check off Windows and no lifecycle
anywhere: two blindfolds on one backend, each of which reads like the other one's job.

And the generic dependency parser took the first word of every non-empty line. The first real
`zypper` run in this project's history could not install a single package: `zypper info
--requires jq` opens with two progress lines and prints a paragraph of prose, so the parser
returned twenty-five "dependencies" of which four were real — among them
`---------------------------`, `x86_64`, `150.4` and `you`. The planner adds every dependency as
an install node and then asks *that* node for its dependencies, which returned the same words,
and the sweep died on a `requires` cycle between three adverbs. Because `sync` syncs the whole
model, that one manager failed every other backend's lifecycle in the same image.

*Both were sitting behind a green suite of 1,581 tests. Neither is subtle. What was missing was
not cleverness — it was a container with openSUSE in it.*

**What the round is careful about.** The new `primary_manager_image()` table is a claim about
runs the process making it cannot see, so it is verified on the run of the image it names: no row
may excuse a backend on the strength of a sweep nobody performs. And every exemption added this
round is *detected* — `choco` when the shell is not elevated, `psresource` when the host has no
PSResourceGet cmdlets, `zfs` when `modprobe -n zfs` says the kernel has no module. `Q17` makes
that the rule, because an assumed skip is a check nobody ever revisits, which is `Q4`'s
disclaimer wearing harness clothes.

*One instrument lied during the round and was caught: an ad-hoc CRLF scan reported nine
contaminated shell scripts, and all nine were clean — the pattern had degraded to matching the
letter `r`. `run.sh`'s version writes a real CRLF probe and asserts it detects it before trusting
itself, which is the entire difference between the two.*

**And then the privileged image ran.** `btrfs`, `lvm` and `zfs` had been excused from every
harness on the strength of one sentence — *"a snapshot provider, not an install target"* — which
is not what the code does. Given a real filesystem to install into, both installable ones failed
on the first attempt, for two different reasons and neither of them subtle.

`btrfs:` could not be written at all. The validator's list of backends whose name is legitimately
a path — `link | web | github | appimage` — omitted the one whose name is *most* literally a
filesystem path, so `btrfs:/mnt/data/vol` came back as `Path traversal detected in name`. It is
the psresource shape again: a list that names most of its members, and the missing one is the
member nobody could test.

`lvm:` still cannot be written, and that one is not the builder's to fix. It requires `@size`;
II.2's option table does not permit it; **the backend's own error message instructs the user to
write a line the parser refuses.** Part II says both things — its storage paragraph says in plain
words that *"a volume has a size and a mountpoint"* — so this is `Q18`, OPEN, and rule 4 applies.

*The harness fails on it, by name, every run, and stays that way until it is ruled. Removing the
option would have made the sweep green in one line and hidden a defect in the program behind a
change to the test.*

Two things about running containers that cost an hour between them: a container borrows the
host's kernel but **not its module files**, so `modprobe btrfs` searched an empty `/lib/modules`
and the harness truthfully reported "this kernel has no btrfs" about a kernel that has it. And
`sh` reads a script incrementally — editing a bind-mounted harness while a container is running
it shifts every byte offset after the read head, and any run overlapping such an edit has to be
thrown away. Both were mine.
