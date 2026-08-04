# The decision register — all 144, two of them open
**One file, six features, one entry waiting to be confirmed or reversed.** Every decision this design forces lives here, with its
status. The registers used to sit at the tail of six proposal parts and **none of them recorded
whether they had been answered**, so the same question could be argued twice and a question
already settled in code could be re-opened by anyone reading the register instead of the tree.

**A recommendation is not a ruling.** Where an entry carries one, it is the author's reading and
nothing more; the owner decides. When a decision is ruled, rewrite its entry as the rule, put the
rule in [Part II](target-state.md) and its reason in [Part V](why.md), **and update this file's
index in the same commit** — the index went 59 entries out of date because that last clause was
not in this paragraph.

## The five statuses

| status | means | what it needs | count |
|---|---|---|---|
| **OPEN — blocking** | Unanswered, and the feature cannot be built without it. | A ruling. | **0** |
| **OPEN** | Unanswered, and something can still be built around it. | A ruling, eventually. | **1** |
| **BUILT, NEVER RULED** | Nobody ruled — but code shipped that implements the recommendation. | Confirm or reverse. Reversing costs a change now and more later. | **1** |
| **ANSWERED** | The owner ruled, or another decision closed it. | Nothing. Kept because later work cites it. | **140** |
| **PARKED** | Deliberately not asked yet, and its `Status:` line says **`waits on <what>`**. | Nothing *until that arrives*. | **2** |

**Three of these five statuses now describe nothing, and they stay.** The categories are not
decoration: *OPEN — blocking* refills the moment a new feature is proposed, and *BUILT, NEVER
RULED* refills the moment somebody implements a recommendation before it is put to the owner.
Deleting an empty category is how the next one goes unnoticed.

**`PARKED` needs nothing *until the thing it named arrives*.** D15 sat parked on D5 for a week
after D5 was ruled and built, because "needs nothing" was read as a permanent property of the
entry rather than a claim with a condition attached. A parked entry is a scheduled re-read, and
the schedule is whatever it says it waits on.

**So the condition is now checked, not just written.** A parked entry's `Status:` line must carry
`waits on <what>`, and `scripts/decision-count.sh --check` fails if that clause is missing, or if
it names a decision that has since been ANSWERED. A condition naming an event out in the world
(D16 waits on someone hitting the case) is allowed and left alone — no script can see that
arrive, and saying so is better than a clause that reads as checkable and is not. **The totals
were checked for a week while the register was wrong; a checker that verifies the arithmetic and
not the claims is half a checker.**

**A status is a claim about the tree, and the "In the tree today:" line in each entry is dated at
the moment it was written.** Several of them are now stale in the direction that matters least —
they say *nothing exists* for work that has since shipped. Where an entry's build state matters,
`plan.md`'s ordered list is the checked answer; this file's job ends at the ruling.

## What each feature's questions cost if answered late

The six registers each opened with a note on *when* their questions get expensive. Grouping by
status loses that, so it is kept here:

- **D1–D17, artifacts.** D1–D6 block the backend outright. **D7–D10 are grammar shape: cheap
  now, expensive after the first real `formats` line exists in somebody's repo.** D11–D14 are
  behaviour over time. D15–D17 are parked on purpose.
- **W1–W14, `vars`.** W1–W5 blocked implementation and are all closed. W6–W10 are scope and
  grammar; W11–W14 behaviour and tooling.
- **K1–K16, N1–N7, T1–T5, U1–U39.** *Blocking* means one thing in all four: **this cannot be
  built without an answer, because two reasonable implementations differ.** **U27–U38 are the
  extension-surface round (XIII.23–XIII.36): opening the snapshot/rollback layer, macOS/BSD
  filesystems, declared storage objects, custom health checks, the "as open as Lisp" set
  (parameterized modules U32, generated declarations U33, a REPL U34, user verbs U35), and the
  three more closed provider-lists the code review found — init systems (U36), notification
  channels (U37), and secret decryption (U38). None blocks. They share one mechanism (XIII.33: a
  declared provider, argv from a file, capability-by-declaration) and one line (XIII.32: open a
  surface only where the added thing is data LiNix cannot hear, never behaviour it cannot see).
  **Direction (owner, 2026-07-25), then rulings (owner, 2026-07-26):** the direction settled the
  *whether* for the provider-list surfaces — U27, U30, U31, U36, U37, U38 — and left the *how*
  and the *safety order* open. **Those were all ruled the next day, and so were the four the
  direction had excluded.** U33 in particular went the other way from XIII.32's refusal: the
  owner ruled that generated declarations **are** built, behind their own config key, off by
  default — which amends U3 and U4. Nothing in the U-series is open.

---

## Index

**One is open — `Z1`, raised 2026-08-03.** All 144 are accounted for: **140 ANSWERED, 2 PARKED, 1 BUILT NEVER RULED, 1 OPEN** — and this line
is no longer typed by hand. `scripts/decision-count.sh --check` counts the entries and fails if
any number written in this file or in `SPEC.md` disagrees with the count; it runs in CI on every
push. Three figures inside this one file used to contradict each other and a fourth in `SPEC.md`
contradicted all three.

> **This index was the worst-drifted thing in the spec, and it drifted in the file whose whole
> job is to stop drift.** Until 2026-07-26 it carried three tables — *Open and blocking — 13*,
> *Open, not blocking — 46*, *Answered — 43* — that nobody updated as the rulings landed between
> 2026-07-23 and 2026-07-26. So it advertised **59 open questions over 59 entries that each said
> ANSWERED four lines further down.** Anyone who read the index instead of the entries would
> have re-opened a settled question, which is the precise failure this file was created to end.
> **An index is a claim about the file underneath it, and this one was checked against nothing.**

**This index says whether a question is decided. It does not say whether the ruling is built** —
that is the ordered list at the top of [`plan.md`](plan.md), and it is deliberately the only
place that answers it. Two lists of build state is the two-of-everything disease applied to our
own documentation.

**The date column** is the entry's own `Status:` date where it carries one, and the first date in
the entry body otherwise (the oldest entries were answered by being built and record the date
there). It is when the question stopped being open, not when the code landed.

### D — artifact selection and channels (Part VIII) — 18

| | question | answered |
|---|---|---|
| **D1** | What is "the release"? — built as latest non-draft, non-prerelease; `v` prefix tolerated. | 2026-07-23 |
| **D2** | How is a format recognised from a filename? — built as extension match plus `binary`. | 2026-07-23 |
| **D3** | Two assets, same format — RULED 2026-07-20: shortest name, `@asset=` glob, `@asset=all`. | 2026-07-20 |
| **D3b** | Download-only artifacts: what is the option called, and does `check` count one as software? | 2026-07-24 |
| **D4** | What installing a tarball does — RULED 2026-07-20: extract, find, shim, `@bin=`. | 2026-07-20 |
| **D5** | A `.deb` installed by `github:` — does apt own it or does LiNix? | 2026-07-24 |
| **D6** | `@sha256` per machine — RULED 2026-07-20: checksums live in `locks/`, generated. | 2026-07-20 |
| **D7** | Does a `formats` block enable the backend — ADOPTED and built: yes. | 2026-07-20 |
| **D8** | May a `when` block appear inside an options body? | 2026-07-24 |
| **D9** | A line's `formats` replaces the backend's — ADOPTED and built: replace, both seams. | 2026-07-20 |
| **D10** | Where the closed vocabulary lives — built as one table in `artifact/format.rs`. | 2026-07-23 |
| **D11** | The default format order is detected, so a LiNix upgrade can silently change it. | 2026-07-24 |
| **D12** | Network, GitHub rate limits, and whether `sync` works on a plane. | 2026-07-24 |
| **D13** | Changing a `channel` — refresh, or remove and reinstall? | 2026-07-24 |
| **D14** | Does `why` explain which of the three levels chose the artifact? | 2026-07-24 |
| **D15** | `.flatpak`/`.snap` assets in a release. D5 answered the ownership half; **re-PARKED on a measurement** — what snapd actually does to a sideloaded snap, which decides it outright. | PARKED |
| **D16** | libc variants (`gnu` vs `musl`) — CLOSED by D3's ruling. | PARKED |
| **D17** | What does `github:re:…@formats=` mean across repos with different assets? | 2026-07-24 |

### W — user-defined `when` variables (Part IX) — 14

| | question | answered |
|---|---|---|
| **W1** | The sigil — built as `$role`, never bare. | 2026-07-23 |
| **W2** | Are values typed — RULED 2026-07-20: full JSON types, no coercion. Built. | 2026-07-20 |
| **W3** | Is a bare `$flag` a condition — ADOPTED and built: no, it is a parse error. | 2026-07-20 |
| **W4** | Where `vars` loads in resolution — built: once, before any `when`. | 2026-07-20 |
| **W5** | What `check` does with an unused variable — built: a note, from a static scan. | 2026-07-20 |
| **W6** | One `vars` file or a directory — built as one file; `vars.d/` ignored. | 2026-07-23 |
| **W7** | The undetectable variable — ANSWERED by the provider model: `env()` is the hatch. | 2026-07-20 |
| **W8** | Do variables work in `active` — built, including every path that edits your files. | 2026-07-20 |
| **W9** | Interpolation outside `when` — stay banned? | 2026-07-24 |
| **W10** | May a variable reference another variable? | 2026-07-24 |
| **W11** | Does `why` explain a variable — built as a gate chain. | 2026-07-20 |
| **W12** | A command to print resolved variables — built: `linix vars`. | 2026-07-20 |
| **W13** | Does changing a variable hit the guard — RULED 2026-07-20: yes, plus a run-level note. | 2026-07-20 |
| **W14** | Does `vars` belong in `diff` — built: yes, the line file and every provider file. | 2026-07-20 |

### K — rebuild, caches, desktops, backup (Part X) — 20

| | question | answered |
|---|---|---|
| **K1** | `rebuild`'s granularity — RULED 2026-07-20: batch per backend, foundation first. | 2026-07-20 |
| **K2** | What is `rebuild`'s default scope? Is a bare `linix rebuild` an error? | 2026-07-24 |
| **K3** | A failed reinstall after a good removal — RULED 2026-07-20: snapshot and revert. | 2026-07-20 |
| **K4** | Is `clean_cache_on_remove` every backend, or only the ones whose file LiNix knows? | 2026-07-24 |
| **K5** | A level-3 reset with a config repo — built as refuse unless `--force`. | 2026-07-23 |
| **K6** | Does LiNix learn per-backend group syntax (`pacman -S plasma`)? | 2026-07-24 |
| **K7** | Which desktops `setting:` adapts to — built as GNOME only, KDE refused by name. | 2026-07-23 |
| **K7b** | The `setting:` key syntax — built as the statement form, not a backend prefix. | 2026-07-23 |
| **K8** | How a git-less LiNix announces it — built on the affected commands plus `doctor`. | 2026-07-23 |
| **K9** | Is the backup command `bundle` — RULED 2026-07-22: yes, plus `restore DIR`. Built. | 2026-07-22 |
| **K10** | `linix edit` and `linix path` — built as two commands. | 2026-07-23 |
| **K11** | May the settings file hold more than the repo path — built as no, parser-enforced. | 2026-07-23 |
| **K11b** | Where that file lives — built in the platform config dir. | 2026-07-23 |
| **K12** | Is a symlink at the default config path still supported? | 2026-07-24 |
| **K13** | Does `rebuild` appear in `schedules` — built as refused by name. | 2026-07-23 |
| **K14** | Does `rebuild` produce a git commit — built as no, and asserted by no test. | 2026-07-23 |
| **K15** | Does `plan` distinguish a rebuild's removals — built: `Reinstalled`, never `Removals`. | 2026-07-21 |
| **K16** | Does `clean-cache --all` need the guard — built as no; `reset` does. | 2026-07-23 |
| **K17** | How does `setting:` reach a store nobody wrote an adapter for? | 2026-07-23 |
| **K18** | Should LiNix use a backend's own atomic swap where one exists (nix, rpm-ostree)? | 2026-07-24 |

### N — `firewall:` (Part XI) — 7

| | question | answered |
|---|---|---|
| **N1** | Is a declared perimeter exclusive (undeclared rules are drift) or additive? | 2026-07-23 |
| **N2** | What happens when the change would close the SSH session running it? | 2026-07-23 |
| **N3** | Which adapters ship — and is one adapter enough to justify the backend at all? | 2026-07-23 |
| **N4** | Is `default/incoming` a statement or a preference key? | 2026-07-24 |
| **N5** | What does removing a firewall rule restore? | 2026-07-24 |
| **N6** | What if a config declares both `firewall:` lines and a `link:` to the ruleset? | 2026-07-24 |
| **N7** | Does `watch` revert firewall drift unattended, or only report it? | 2026-07-24 |

### T — secrets (Part XII) — 7

| | question | answered |
|---|---|---|
| **T1** | `backup_once` leaves a plaintext copy of the previous secret forever. | 2026-07-23 |
| **T2** | Nothing stops `@target=` writing a plaintext secret back inside the git repo. | 2026-07-23 |
| **T3** | What does a missing hardware token look like — prompt, hang, or error? | 2026-07-24 |
| **T4** | May an unattended `watch` tick decrypt with a touch-required key? | 2026-07-24 |
| **T5** | Is the plaintext 0600 at creation, or chmod'd after? And on Windows? | 2026-07-23 |
| **T6** | Must there be a way to opt out of `backup_once`, or bound how many pile up? | 2026-07-23 |
| **T7** | Runtime injection of secrets into process memory — reopened. | 2026-07-24 |

### U — the next round (Part XIII) — 38

| | question | answered |
|---|---|---|
| **U1** | Where does a custom backend definition live — the repo, or machine-local? | 2026-07-23 |
| **U2** | Is a custom backend a full peer of a built-in (repos, orphans, dependencies)? | 2026-07-24 |
| **U3** | What does removing an `exec:` line mean when a script has no inverse? | 2026-07-24 |
| **U4** | Is `exec:` a licence to put a shell script where a backend belongs? | 2026-07-24 |
| **U5** | Does `setting:` get a Windows registry and a macOS `defaults` adapter? | 2026-07-23 |
| **U6** | Does this document mark its Linux-only guarantees (snapshots, rollback)? | 2026-07-24 |
| **U7** | Is a health check per-package or per-sync? | 2026-07-24 |
| **U8** | Is the removal preview a flag or a new verb? | 2026-07-24 |
| **U9** | Do the ten status commands collapse into one `linix check`? | 2026-07-24 |
| **U10** | Where does a backend's bootstrap live — `priority` or the definition file? | 2026-07-24 |
| **U11** | Does `watch` imply `--locked`? | 2026-07-24 |
| **U12** | Does `try` reuse the Phase 6 images, or build from a base the config names? | 2026-07-24 |
| **U13** | Does `@runs=always` exist? | 2026-07-24 |
| **U14** | Is sharing wanted, and what makes a vendored module safe to run? | 2026-07-24 |
| **U15** | Where do LiNix-level event hooks live, and are they per-machine? | 2026-07-24 |
| **U16** | May a custom backend's `binary` be an absolute path? | 2026-07-24 |
| **U17** | Is `linix eval`'s JSON versioned from the first release? | 2026-07-24 |
| **U18** | Are grouped backends with per-group priority worth building at all? | 2026-07-24 |
| **U19** | Is LiNix acting for a user or for the machine? (`HKCU` vs `HKLM`) | 2026-07-24 |
| **U20** | Is a language server wanted, and may it be a second implementation? | 2026-07-24 |
| **U21** | Is the exit-code table settled once, up front? | 2026-07-24 |
| **U22** | Does the dotfiles tree link files, or whole directories? | 2026-07-24 |
| **U23** | What happens when a dotfile destination already holds the user's own file? | 2026-07-24 |
| **U24** | Is a `.age` file inside the dotfiles tree a secret to decrypt? | 2026-07-24 |
| **U25** | One dotfiles tree, or several? | 2026-07-24 |
| **U26** | Is BSD supported, and what does `when family` answer there? | 2026-07-24 |
| **U27** | Is the snapshot/rollback layer opened to a registry + config-driven providers? | 2026-07-26 |
| **U28** | One snapshot provider or several, chosen by capability not list order? | 2026-07-26 |
| **U29** | Is APFS the macOS safety net, and is its restore `Live` or not? | 2026-07-26 |
| **U30** | Declare storage objects (zfs/lvm/btrfs) as a family — and does the guard cover destroying one? | 2026-07-26 |
| **U31** | Should health checks be an open vocabulary — a user-declared check command? | 2026-07-26 |
| **U32** | Do modules take parameters (the macro), and are parameter types checked? | 2026-07-26 |
| **U33** | Are generated declarations — a config that runs a program to produce state — wanted at all? | 2026-07-26 |
| **U34** | Is `linix repl` worth a second entry point, or is `eval \| jq` enough? | 2026-07-26 |
| **U35** | May a user name a new verb, strictly as a composition of built-ins? | 2026-07-26 |
| **U36** | Are init systems a declared-provider kind (s6/dinit/runit/Shepherd), or stays a closed enum? | 2026-07-26 |
| **U37** | Are notification channels their own declared kind, or is an event hook the answer? | 2026-07-26 |
| **U38** | Is secret decryption a declared-provider kind, and behind which T-series rulings? | 2026-07-26 |
| **U39** | When a manager installs by one string and removes by another, which one is the declaration? | 2026-07-26 |
| **U40** | Does a command LiNix runs write to your terminal, or to LiNix? | 2026-07-27 |
| **U41** | What does a rollback do when the guard refuses one of its compensating removals? | 2026-07-27 |
| **U42** | Do the overlapping command clusters get consolidated? | 2026-07-27 |
| **U43** | How much does an ordinary run say about itself? | 2026-07-27 |

### Q — the production-readiness round and the grading rounds after it — 16

*Not a proposal part. These are the questions the readiness assessment forced — behaviour a
user notices, or a published contract — raised because `CLAUDE.md` requires a ruling for them
and the answers were in no file. `Q` was picked by checking every other prefix first: the
review document had reused `U1`–`U3`, which are real register IDs belonging to other questions,
and that collision is exactly what this namespace exists to avoid.*

| | question | answered |
|---|---|---|
| **Q1** | Does a failed install leave its line in the file? — RULED: withdraw it when it can never succeed. | 2026-07-27 |
| **Q2** | Is a package manager you never installed "critical"? — RULED: no, absent is its own state. | 2026-07-27 |
| **Q3** | What does a mistyped command exit with? — RULED: 1, and the table stays at four codes. | 2026-07-27 |
| **Q4** | Are unverified backends labelled "experimental"? — RULED: **no.** They are tested, and nothing ships until they are. | 2026-07-27 |
| **Q5** | Does `@unverified` reach past the backends that download? — RULED: **yes** — a manager that verifies a signature itself (`helm`) takes it too. | 2026-07-28 |
| **Q6** | May a definition in `adapters/backends.toml` take a built-in's name? — RULED: **yes, and only by saying so** — `overrides = true`. | 2026-07-28 |
| **Q7** | Does the removal guard cover the resources a declaration puts in place, or only packages? — RULED: **the same rules**. | 2026-07-28 |
| **Q8** | Should the security refusals return the documented refusal code? — RULED: **yes, all of them exit 3**. | 2026-07-28 |
| **Q9** | Should a verb taking a backend name refuse one that does not exist? — RULED: **yes**, `install`'s message everywhere. | 2026-07-28 |
| **Q10** | Should `mix` install Hex before installing an archive from it? — RULED. | 2026-07-29 |
| **Q11** | What should `opam:` do on a machine with no opam switch? — RULED. | 2026-07-29 |
| **Q12** | Should the sweep fail when its real coverage collapses, and against what? — RULED: **a ratchet**, threshold to the builder. | 2026-07-28 |
| **Q13** | Should `asdf:` add the plugin a declared tool needs? — RULED: **yes**, off the argv. | 2026-07-29 |
| **Q14** | What should `@unverified` do on a tool version with no flag to turn verification off? — RULED: **accept in silence**; the tool does not verify at all. | 2026-07-30 |
| **Q15** | Should a command whose product is a file at a path the user named honour `--dry-run`? — RULED: **yes, except `plan`**. | 2026-07-30 |
| **Q16** | Is a bare grammar keyword (`link`, `when`, `absent`) a package name? — RULED: **no, it is a parse error**; `list:NAME` still means the package. | 2026-07-30 |
| **Q17** | How does a backend that mutates the real machine get its first real lifecycle? — RULED: **install and uninstall it, on the developer's own box**; and privileged containers are allowed for the storage backends. | 2026-07-30 |
| **Q18** | The storage backends read options II.2's table does not permit, so `lvm:` cannot be written at all — which half of Part II is wrong? — RULED: **the table.** The keys are added, scoped to the backends that read them. | 2026-07-31 |
| **Q19** | A changed `@quota` or `@size` did nothing on the next sync — RULED 2026-07-31: it resizes, and shrinking needs `@allow_shrink` on the line. | 2026-07-31 |
| **Q20** | A changed `@classic` did nothing either — RULED 2026-07-31: same answer. Relaxing confinement converges; narrowing it is refused, because only remove-and-reinstall can. | 2026-07-31 |
| **Q21** | Is converging-on-change a property of *every* option, or of the five that happen to have it — RULED 2026-07-31: every option, proved per option. | 2026-07-31 |
| **Q22** | A config file saved by a Windows editor starts with a byte-order mark, which became part of the first name — refuse it, or read the file? — RULED 2026-07-31: **read it.** The mark is stripped where text enters a parser. | 2026-07-31 |
| **Q23** | A package name that begins with `@` — every scoped npm package — was read as an option list and could not be written at all. — RULED 2026-07-31: **the leading `@` is part of the name.** | 2026-07-31 |
| **Q24** | An uninstall sat 76 minutes on a child that had finished its work and never returned; nothing outside the DAG bounded a command at all. BUILT 2026-08-02: a bound on **silence**, `command_idle_timeout_secs = 900`. The number is unruled. | — |

*Q7–Q13 were absent from this table while their entries below said ANSWERED — the index drift
this file exists to prevent, found on 2026-07-30 by adding a row to it.*

### Z — the readiness audit — 2

*`docs/GRADE-2026-08-03.md` drove a real binary against the five questions the audit was
commissioned to answer and found twelve defects. Ten were internal correctness and were built
without asking. Two are not mine: one is a legal choice and one changes a published verb name.*

| | question | answered |
|---|---|---|
| **Z1** | There is no `LICENSE` file and no `license` key in `Cargo.toml`, for a tool with an install script and a `self-upgrade` verb. Which licence? | **OPEN** |
| **Z2** | `lock` and `unlock` touch unrelated files and are not inverses; `unlock` can cause package churn. Rename it, or give `lock` a real inverse? | **ANSWERED** — both verbs name their axis (`versions`/`backends`/`scripts`/`all`), and every path that moves a version re-records it |

### Y — the efficiency pass — 8

*Not a proposal part. `docs/INEFFICIENCIES.md` audited every place in the tree slower than it has
to be and marked the findings a user would notice as needing a ruling; the owner ruled the lot on
2026-08-02 — "as parallel as possible, as efficient as possible, as fast as possible, restructure
if it takes that". These four are the parts of that with visible behaviour. The rules are in
II.19 and the reasons in V.115–V.118.*

| | question | answered |
|---|---|---|
| **Y1** | One `apt install` per package, measured at 12,465 ms against 3,161 ms for one command — does LiNix batch? — RULED: **yes**, per manager, per wave, bounded, with rollback still per package. | 2026-08-02 |
| **Y2** | `max_parallel` bounded sockets by core count — one knob or two? — RULED: **two.** `network_parallel` (16), and `upgrade` fans out across the managers that contend with nothing. | 2026-08-02 |
| **Y3** | `search` had no per-backend deadline, so one slow registry set the whole runtime — may a read give up? — RULED: **yes, and it says which backend.** | 2026-08-02 |
| **Y4** | A ~51-second Windows restore point ran as a silent barrier before every mutation — RULED: it **starts first, is joined last, and announces itself.** | 2026-08-02 |
| **Y5** | Nothing said *which* manager a run waited on, so "why was that slow" could only be answered by timing the managers by hand outside LiNix — RULED: **`--timings`**, off by default, on stderr, reporting wall clock against summed child time. | 2026-08-03 |
| **Y6** | Every run re-asked every manager the same question about a machine nothing had touched — may an answer outlive its run? — RULED: **yes, opt-in.** `installed_cache_secs`, 0 by default, dropped on every mutation and by `clean-cache`, bypassed by `--no-cache`. | 2026-08-03 |
| **Y7** | `winget list` reports names with spaces in them and "a package name is one word" refused them, so a name LiNix printed was a name LiNix could not be given — RULED: **quote it.** `winget:"ARP\Machine\X64\Mozilla Firefox"`. | 2026-08-03 |
| **Y8** | Nine managers started 5.4 s into a 9.1 s `check drift` and the run was idle before they did — why did it not overlap? — RULED: **ask every manager the run will ask, at once.** Not slower children: unasked ones. 9.1 s → 3.9 s, 2.7× → 5.4×, same report. | 2026-08-03 |

---

# The blocking round — thirteen questions, all ruled

*These could not be built around: two reasonable implementations differed. Ruled 2026-07-23/24.*

## D3b

**Status: ANSWERED — ruled 2026-07-24.**

**In the tree today:** Nothing. No mode enum in `backends/artifact/`; `core/artifact_lock.rs:321` mentions download-only artifacts in a comment only.

**D3b — download-only artifacts (owner, 2026-07-20, raised with D3).** A `github:`/`web:` line may
ask LiNix to **fetch an artifact without installing or managing it** — the `web:`-shaped case.
The two modes are different declarations and must not be one key wearing two meanings:

- **managed (default)** — LiNix installs it, owns it, and **removes it when the line leaves the
  modules and profiles**, through the ordinary plan and guard.
- **download-only** — LiNix fetches it to a known place and stops. It is still declared, so it is
  still removed when the declaration goes; what it is *not* is installed, shimmed, or on `PATH`.

*Owed:* the option's spelling, and whether a download-only artifact appears in `check` as
software (it is not software, so probably not). Recorded as **D3b, open**, not assumed.


**RULED (owner, 2026-07-24): yes, a distinct download-only declaration — and it is the DEFAULT when a thing cannot be installed.** `web:`/`github:` may fetch an artifact without shimming it or putting it on PATH; it is still removed when the line goes. When LiNix has no way to install the fetched thing (no shim target, no archive binary), download-only is what it does by default rather than failing. A separate meaning, not one key wearing two.

**RULED, NOT YET BUILT (2026-07-24).** A distinct download-only declaration, and the default when a fetched thing cannot be installed. Queued: it changes how `web:`/`github:` behave (a new mode in the install path), which is a semantic change to a core backend rather than an additive one. The ruling — a separate meaning, still removed when the line goes, download-only by default when uninstallable — is settled for when it is built.
---

## D5

**Status: ANSWERED — ruled 2026-07-24.**

**In the tree today: BUILT (2026-07-26).** `github:`/`web:` hand a `.deb` to `dpkg -i` and an `.rpm` to `rpm -U` (`backends/artifact/system_pkg.rs` is the one place that argv is built); the lock records `installed_by`/`system_package` (`ArtifactLock`), removal routes back through the recorded manager (`dpkg -r`/`rpm -e`), and `installed_but_unmanaged` + `adopt`'s discovery subtract those names so `check` does not double-count and `purge-unmanaged` defers. `installable_here` now accepts a `.deb` only where `dpkg` exists and an `.rpm` only where `rpm` does — otherwise the line falls through to download-only. Unit-tested (argv, lock round-trip, dedup accessor, installer gating); only the live `dpkg -i`/`rpm -U` round-trip is deferred to a real apt/rpm box.

**D5 — A `deb` installed by `github` — who owns it?** `dpkg -i` puts it in apt's database. Now
`apt` can upgrade it out from under LiNix, `linix check` may see it twice (once as a github
declaration, once as an apt-visible package), and the removal path has to know which tool to
call. **This is the "two of everything" failure at the package level**, and `purge-unmanaged`
(II.11) will have an opinion. *Recommendation:* the lock records the installing backend and
that backend owns removal; `check` must not double-count. Needs a real test against a real apt
box, not a mock.


**RULED (owner, 2026-07-24): the installing backend owns it.** When `github:`/`web:` installs a file (a `.deb` handed to `dpkg`, etc.), the lock records which backend installed it, and that backend owns removal, upgrade and dedup — `check` does not report it twice, and `purge-unmanaged` defers to the recorded installer. This is the existing per-backend ownership (every managed package carries its backend); the `github:`-installs-a-`.deb` capability itself is separate and unbuilt, but the ownership rule is settled for when it lands.

**BUILD DIRECTIVE (owner, 2026-07-26): build it now, test the live install later.** No longer held back for an apt box. The capability — `github:`/`web:` handing a `.deb` to `dpkg -i` / an `.rpm` to `rpm -i`, the lock recording the installing backend, `check`'s dedup, and `purge-unmanaged`'s deference — is built now and unit-tested (argv construction, lock round-trip, dedup logic, purge deference). Only the live `dpkg -i` / `rpm -i` round-trip is deferred to a real apt/rpm box; it is exercised there, not claimed here.
---

## K2

**Status: ANSWERED — ruled 2026-07-24.**

**In the tree today:** Nothing in `app/rebuild.rs` requires a scope.

**K2 — What is `rebuild`'s default scope?** `--all` on a bare `linix rebuild` is a very large
default for a command whose failure mode is an unbootable machine. *Recommendation:* require a
scope; a bare `rebuild` errors and lists the forms.


**RULED (owner, 2026-07-24): warns, then rebuilds all.** A bare `linix rebuild` does NOT refuse — it rebuilds every declared package, but WARNS loudly first, because the failure mode is software missing from a machine and `--all` is a large thing to reach by pressing enter. The warning is the safeguard the built-to-recommendation used a refusal for; the owner chose warn-and-proceed. The old `bail` is replaced.
---

## K4

**Status: ANSWERED — ruled 2026-07-24.**

**In the tree today:** No `clean_cache_on_remove` key exists anywhere in `src/`.

**K4 — Is `clean_cache_on_remove` per-package on every backend, or only where LiNix knows the
artifact?** LiNix knows the file for `github:`/`web:`/`appimage:` (it is in `locks/`). For apt
or pacman it needs a new per-backend capability. *Recommendation:* download-backends only,
documented as such in the key's own description — a preference that silently does nothing on
most backends is worse than a narrower one that is honest.


**RULED (owner, 2026-07-24): download-backends only, plus a user cache pointer and common-location search.** `clean_cache_on_remove` acts only where LiNix knows the file (download backends: `github:`/`web:`/`appimage:`), documented as such in the key. ADDITIONALLY (owner): the user may point LiNix at a cache directory, and LiNix searches the common cache locations (`~/.cache`, `/var/cache`, XDG, each manager's own) so it can find and clean an artifact it did not download itself.

**RULED, NOT YET BUILT (2026-07-24).** `clean_cache_on_remove` (download backends only) + a user cache pointer + search of common cache locations. Queued: `clean_cache_on_remove` does not exist yet, so this is a new option plus a cache-search capability, not an additive tweak. The ruling — download-backends only, honest about doing nothing elsewhere; user may point at a cache; LiNix searches `~/.cache`/`/var/cache`/XDG — is settled for when it is built.
---

## U1

**Status: ANSWERED — by 7a's approval, and BUILT 2026-07-23.**

**In the tree today:** `Layout::custom_backends_file()` — the config repo, and the only path any
loader reads.

**U1 — Where does a custom backend definition live?** Today `~/.config/linix/custom_backends.
toml`, machine-local, never in git — so a repo that uses `paru:` breaks on every machine but
the one where somebody hand-wrote the file. *Recommendation:* the config repo, as a
first-class file beside `priority` and `schedules`, with the machine-local path kept **only**
if there is a case for a definition that must not travel — and if there is not, deleted in the
same change rather than left as a second place to look. **The consequence that makes this a
decision and not an obvious fix:** a definition in the repo is argv that a shared repo can
execute, which is II.12's supply-chain surface. It must inherit the hook trust model, not a
new one.

**BUILT 2026-07-23, both halves.** The file is `<config_root>/custom_backends.toml`, read
through `Layout` like every other repo file; the machine-local path is deleted rather than kept
as a fallback, so `grep -rn "custom_backends" src/` finds one loader. **And it inherits the hook
model rather than getting one of its own**: the file's sha256 lives in `locks/hooks.toml` under
`backends:custom_backends.toml`, `linix lock` approves it, and an unapproved or edited file
registers **nothing** and says why. The check is at load rather than at the sync gate on purpose
— a registered backend is reachable from `search` and `list`, which no sync guards.

**One identity for the whole file, not one per definition.** A per-backend identity would let an
edit that *adds* a `[[backend]]` pass unnoticed, and adding one is the whole attack.

**Not decided here:** U2 (is a custom backend a full peer) and U16 (may `binary` be a path). U16
became reachable the moment `binary` existed, so it is refused for now — a definition naming
`/opt/vendor/thing` works on one machine, which is the property this entry moved the file to
fix — and the refusal says so rather than resolving the path.

---

## U3

**Status: ANSWERED — ruled 2026-07-24.**

**U3 — What does removing an `exec:` line mean?** Every other statement's removal undoes
something. A script has no inverse. *Recommendation:* an optional `@undo=` command; without it,
removing the line removes only the record, and `plan` says so in those words rather than
implying a revert that will not happen.

**RULED (owner, 2026-07-24): as recommended.** An optional `@undo=<command>`; without one,
removing an `exec:` line drops the lock row and nothing else, and `plan` says so in those words
rather than implying a revert that will not happen. A script has no inverse, and inventing one
would be LiNix claiming to undo something it cannot.

**AMENDED by U33 (owner, 2026-07-26): this ruling stands.** U33 lets `exec:` install software, but
does not give it an inverse — so an `exec:` that installs still leaves the software behind when the
line goes unless `@undo=` is written, and `plan` still says so. The removal contract here is
unchanged; only the *content* an `exec:` may carry widened.

---

## U9

**Status: ANSWERED — ruled 2026-07-24, and BUILT.**

**U9 — Do the ten status commands collapse into one?** *Recommendation:* yes, one `linix check`
with sections and narrowing flags; `heal` stays separate because it acts. Old names deleted in
the same change (P2), not aliased.

**RULED (owner, 2026-07-24): yes — "make it intuitive and easy" — and the repairs move to
`heal`.** Six commands are gone: `status`, `doctor`, `unmanaged`, `absent`, `conflicts`,
`audit`, folded into `linix check` with seven sections (`config`, `drift`, `unmanaged`,
`absent`, `conflicts`, `health`, `security`). Deleted, not aliased: an alias is the second way
to do one thing, kept alive.

**A section is a positional argument, not seven flags.** `linix check health` reads as a
question and `linix check --health` reads as a modifier; the ruling asked for intuitive, and
that is the difference. An unknown section is refused with the legal list printed, from the same
table the parser reads — so the error cannot drift from what is accepted.

**The default output is a verdict per section, each naming the command that acts on it** — P8:
a report whose next step is the reader working out what to run has done the easy half.

```
ok  config      42 package(s) declared
->  drift       3 to install, 1 to remove
                   run `linix sync`
->  unmanaged   103 package(s) LiNix does not manage
                   run `linix adopt`
```

**`doctor --fix` is gone, and its three repairs are `heal`'s** (owner, asked and answered
2026-07-24): creating the II.1 directories, reconciling `locks/versions.json`, refreshing a
stale backend index. That is the whole dividing line the ruling rests on — **`check` looks,
`heal` acts** — and it is why `heal` survives the collapse. A command that both diagnoses and
repairs is one you cannot run to find out whether you want a repair.

**Two things the entry did not say, decided while building:**

- **A `config` section that fails stops the sections that depend on it.** Drift, absent and
  conflicts are all read off a resolved model; reporting "0 drift" from a model that failed to
  resolve would be a clean bill of health computed from nothing.
- **A `security` section that cannot reach the advisory database reports that, not "clean".**
  The network being down is a gap in the report, never an absence of advisories.

**7i's exit condition is met:** `grep -rn "Commands::\(Status\|Doctor\|Unmanaged\|Absent\|
Conflicts\|Insight\|Metrics\|Audit\)" src/` is silent, and `heal` survives.

---

## U14

**Status: ANSWERED — ruled 2026-07-24; safety story below.**

**U14 — Is sharing wanted, and what makes a vendored module safe to run?** Vendoring puts
someone else's files in your repo, and once `exec:` exists those files can contain a verb. The
defence on offer is that it lands as a reviewable diff, which is a real defence and a weak one —
nobody reads the whole diff. *Recommendation:* decide the safety story before deciding the
feature. The candidates are: vendor everything but refuse to run an `exec:` that arrived this way
without an explicit per-module opt-in; or vendor modules but never backend definitions and never
`exec:`; or do not build it. **This is blocking because building the convenient version first
and the safety story afterwards is how supply-chain incidents are written.**


**RULED (owner, 2026-07-24): build it.** Sharing is wanted, and a module may be referenced by a GitHub or other URL. A vendored module that carries code the repo can run (an `exec:` verb, a backend definition) needs an **opt-in to run**, and a flag or key must be able to force it. The precedent is the II.12 approval ledger and its siblings (`--allow-mass-removal`, `--replace-existing`, `@allow_http`, `@unverified`): refuse the dangerous thing by default, require one deliberate act to permit it. A vendored `exec:` is therefore approved the way every other script the repo runs is — `linix lock`, which means a human looked — and until then it does not run. **Still to design before building: how a URL reference is written (this changes `use takes a name, never a URL`, V.x), and whether a URL-vendored backend definition is allowed at all or only modules.**

**BUILT, 2026-07-24 (`linix add`).** `add <source>` vendors a source's shareable files (`modules/`, `adapters/`, `scripts/`) into the repo as a reviewable diff; `profiles/`, `active` and `priority` are left behind (the other machine's choices). Sources: `github:owner/repo`, any git URL, a raw file URL, a local path. A name collision is refused and named (`--force` overwrites). Vendored code (`exec:`, adapters) arrives UNAPPROVED and II.12 holds it until `linix lock` — `--trust` locks in the same step. A stranger's path that escapes the repo (`../../.bashrc`) is dropped by `safe_relative`; symlinks are not followed. Verified end to end: a vendored `exec:` refuses to run until approved.
---

## U19

**Status: ANSWERED — ruled 2026-07-24.**

**U19 — Is LiNix acting for a user or for the machine?** Today: implicitly, whoever ran the
command — which the Linux backends mostly agree with by accident, and which the Windows registry
adapter (7e) cannot use, because `HKCU` and `HKLM` are a choice with no default that is right
for both. Three candidate answers, and they are not equally good:

1. **LiNix is per-user, and system-wide is just what some managers happen to do.** Simplest, and
   it is roughly today's behaviour made explicit. It cannot express *"this setting applies to
   every account on this box"*, which is most of what a Windows or a shared Linux machine wants.
2. **`@scope=user|system` on the statements where it can vary** (`setting:`, `link:`, `shim:`),
   with a per-backend default. Precise, and it puts the question in front of the user at the one
   moment they know the answer. Costs a new option key on three statement kinds.
3. **The config repo declares its scope once**, at the top, and every line inherits it. One
   decision per repo rather than per line — and a machine that needs both then needs two repos,
   which is the wrong shape.

*Recommendation:* **2**, with a default per statement kind that matches what the underlying store
does anyway (`gsettings` → user, registry → `HKCU`, `apt` → system). **Answer this before 7e is
written**, because whatever the registry adapter picks becomes the convention by precedent and
then spreads to macOS `defaults`.

**RULED (owner, 2026-07-24): option 2 — and writing the default is not an error.** `@scope=user`
on a store whose default is already user is accepted and means exactly what it says. A
configuration is allowed to state a thing it also gets for free: saying it out loud is how a
reader learns the answer without going to look it up, and refusing it would punish the person
being explicit.

**BUILT 2026-07-24, on the rule that nothing may silently ignore it (P7).** The key is accepted
on `setting:`, `link:` and `shim:` and refused on statements where the question does not arise
(`service:` is the init system's business, `schedule:` the timer's) — a key that means nothing
where it is written is a key that gets written there and quietly does nothing. A misspelling is
refused with both legal values named, because a typo that read as "the default" would be a line
that looks like a decision and behaves as if nobody made one.

**Where it is honoured, and where it is refused:**

- **`setting:`** — a `[[setting_store]]` row may carry `system_read`/`system_write`/
  `system_reset` beside the per-user three. A store that has them runs *different commands* per
  scope; a store that does not (`gsettings`) **refuses `@scope=system` by name** rather than
  writing the per-user value under a line that says every account. The read-before-write check
  reads in the same scope it will write, or it would compare two different settings and call
  them equal.
- **`shim:`** — refused for `system` today: LiNix deploys shims only into this account's
  `~/.local/bin`, and a per-user shim under a line saying every account is the wrong answer
  quietly.
- **`link:`** — accepted and carried; the destination is already explicit in `@target=`, so
  there is nothing for it to change until ownership/permission handling exists.

**This unblocks 7e.** The registry adapter writes `HKCU` by default and `HKLM` under
`@scope=system`, and macOS `defaults` inherits the same convention — which is what the entry
said had to be settled before the first adapter was written.


**RULED (owner, 2026-07-24): explicit per-line scope, default user.** Option (c): `@scope=user|system` on the statements where it can vary (`setting:`, `link:`, `shim:`), which is already built. The owner asked for a concrete default, and it is **user** — `Scope::resolve(written, Scope::User)`: an unspecified scope is per-user (HKCU, gsettings, `~/.local/bin`), and machine-wide (HKLM, `/etc`) requires writing `@scope=system`. Least privilege: changing every account's state is the deliberate case.
---

## U22

**Status: ANSWERED — ruled 2026-07-24.**

**U22 — Does the dotfiles tree link files, or directories?** One symlink at `~/.config/nvim`
is a single operation and takes the whole directory hostage: everything the application later
writes there — caches, session files, a plugin lockfile — lands inside the git-tracked config
repo, and `bundle` then hands it to whoever the backup goes to. Linking each *file* leaves the
directory the user's and puts nothing in the repo that was not put there deliberately, at the
cost of walking the tree every sync and of one ledger row per file. *Recommendation:* per
file. **The consequence that makes it a decision rather than an obvious fix:** per-file linking
cannot express *"this directory is entirely mine"*, which is what a `nvim` config under version
control usually is — so if the answer is per-file, the directory case needs its own spelling
later rather than being reachable by accident.

**RULED (owner, 2026-07-24): per file, as recommended.** One symlink at `~/.config/nvim`
takes the whole directory hostage: every cache, session file and plugin lockfile the application
later writes lands inside the git-tracked repo, and `bundle` then hands it to whoever the backup
goes to. Linking each file leaves the directory the user's and puts nothing in the repo that was
not put there deliberately.

---

## U23

**Status: ANSWERED — ruled 2026-07-24.**

**U23 — What happens to a destination that already holds the user's own file?** `link:`
answers this one file at a time with `backup_once`. A tree asks it forty times on the first
sync of a new machine, which is precisely the machine where the home directory is full of
files a distribution's defaults put there. Silently backing up forty files is not a preview,
and refusing on the first collision leaves the sync half-applied. *Recommendation:* the plan
lists every colliding destination **before** anything is written and the run is refused until
the user says which way; `--adopt-existing` (or whatever it ends up called) is the one-word
answer for "back them all up". This must be settled before the walker is written, because a
tree that half-links is worse than one that does not run.

**RULED (owner, 2026-07-24): as recommended, plus an explicit bypass.** The plan lists every
colliding destination *before* anything is written and the run is refused until they have been
seen — silently backing up forty files is not a preview, and refusing on the first collision
leaves the sync half-applied.

**And there is a flag to proceed anyway** (owner's addition): the common case on a fresh machine
is that every colliding file is a distribution default nobody edited, and making the user
acknowledge forty of those one at a time is a refusal that teaches people to bypass refusals. The
flag is explicit and per-run; it is never a config key, because a machine that always bypasses
this is a machine where the check does not exist.

---

## U24

**Status: ANSWERED — ruled 2026-07-24.**

**U24 — Is a `.age` file in the tree a secret?** XII's decrypt mode is an option on a `link:`
line, and this statement has no per-file options by construction. Either the extension decides
(magic, and magic that silently writes plaintext), or encrypted files are simply not this
statement's job and stay on explicit `link:` lines. *Recommendation:* the second — **the tree
never decrypts.** T2 is already an open finding about plaintext landing in the config repo, and
a folder walker that decrypts by filename convention is the same failure with more surface.

**RULED (owner, 2026-07-24): the tree never decrypts.** An `.age` file in the dotfiles tree
is copied as the ciphertext it is. Deciding by file extension is magic, and magic that silently
writes plaintext; secrets stay on explicit `link:` lines where `@decrypt=` is written down.

---

## U26

**Status: ANSWERED — ruled 2026-07-24.**

**U26 — Is BSD a supported platform, and if so what does `when family` answer there?** (XIII.22.)
Two questions, and only the second is blocking: registering `pkg`/`pkg_add` is ordinary backend
work that can happen whenever, but **`when family` has no answer on a BSD today and silently
returns the else branch**, so a config that is correct on Linux is quietly wrong there rather
than refused. *Recommendation:* decide the identifier before either backend is written —
`freebsd`/`openbsd`/`netbsd` as families beside `debian`/`arch`, sourced from `uname -s` when
`/etc/os-release` is absent, and **an unidentifiable host is an error, not an empty string**,
because an empty family is what makes every `when` block silently false. The support question
itself is the owner's: P7 is already unpaid on `setting:` (GNOME-only) and the snapshot promise
(Linux-only), and a third platform admitted before the second is honest turns the principle into
a slogan. A legitimate answer is *"listed, dated, not scheduled"* — what is not legitimate is
leaving `family` returning a wrong answer on a platform whose package manager LiNix already
drives (`pkgin` is registered today).


**RULED (owner, 2026-07-24): a family that cannot be shown to be X makes `family == X` false, not an error.** The owner rejected the hard-error option: an unidentifiable or non-matching family simply fails the positive comparison, because it cannot be demonstrated to be that family. This is already the behaviour — `HostFacts::current` falls back to `std::env::consts::OS`, which is `freebsd`/`openbsd`/`netbsd` on the BSDs, so `family` answers the OS name there and `== debian` is correctly false. The build is a test locking that in and a why note; BSD backend registration (`pkg`/`pkg_add`) is ordinary work for whenever.
---

# The non-blocking round — forty-six questions, all ruled

*Something could still be built around each of these. Ruled 2026-07-23 through 2026-07-26; the U27–U38 extension-surface set closed last, on 2026-07-26.*

## U27

**Status: ANSWERED — ruled 2026-07-26.**

**U27 — Is the snapshot/rollback layer opened to a registry and config-driven providers, the way
backends are, or do new providers stay hand-written Rust? (XIII.23.)** Today `SnapshotProvider`
is a seven-method trait and the four providers (btrfs, zfs, timeshift, `windows_restore`) are a
hardcoded vec in `SnapshotManager::new` (`snapshot.rs:528`), first-available active. Adding
APFS/LVM/bcachefs/WinBtrfs is a full Rust impl each. Two reasonable answers: **(a)** a
`SnapshotProviderRegistry` plus a config-driven provider and a `custom_snapshots.toml`, matching
the backend model — a create/list/delete/restore filesystem becomes ~thirty lines of data;
**(b)** a registry only — providers stay Rust, but pluggable and no longer a hardcoded vec — on
the grounds that a provider which gets `restore_capability` wrong bricks a machine (V.60), a
higher bar than a package listing and maybe too high for a TOML file. *Recommendation:* (a),
with the one constraint that makes it safe — **`restore_capability` is never inferred**: a
provider whose config does not prove it can restore a running system is create-only
(`NotFromRunningSystem`), never `Live`, so the worst a wrong config does is decline an undo it
could have offered, never perform one it cannot keep. Ownership marking (S3) must be expressible
or retention is disabled for that provider; a custom provider registers last and never shadows a
built-in (XIII.2's rule).

**RULED (owner, 2026-07-26): option (a), the full plugin — and the restore capability is a value
the plugin author enters, not one LiNix guesses.** A snapshot provider is a declared block (the
same shape as a custom backend and a `[[setting_store]]`): the commands to take, list, delete and
restore a snapshot, as data in a file in the config repo. No new release is needed to teach LiNix
a new snapshot technology.

- **The plugin must state whether it can restore a *running* machine** — the one thing that
  cannot be inferred (V.60): taking a snapshot and restoring one over the live system are
  different abilities, and a wrong guess is a machine reported safe that is not. The author writes
  it because the author knows it. Restore-capability is a **required field with no default**;
  omitting it is a loud error naming the provider, never a silent "assume it can" or "assume it
  can't". A provider declared unable to restore a running system is create-only — it saves state
  and refuses the rollback rather than attempting one it cannot finish.
- **It must work for everything, so the built-ins use the same door.** btrfs, zfs, timeshift and
  Windows System Restore stop being a hardcoded list and become rows read through the one loader a
  user's row goes through — the K17/U1 rule, so the mechanism is proven by the shipped providers
  and cannot drift into a privileged path nobody tested. APFS, LVM, bcachefs and the rest are then
  data, not code.
- **It must be easy to add.** One file, roughly thirty lines of data, beside the other adapters in
  `adapters/` (U10), under the II.12 hook ledger because it is argv a shared repo can run —
  approved by `linix lock`, the same trust answer already given for custom backends and settings
  stores, not a new one.

**Implementing calls (owner ruled the shape; these follow from it):** a custom provider registers
last and never shadows a built-in (XIII.2); ownership marking (S3) must be expressible in the row
or retention is disabled for that provider; a row missing `restore` or a required capability field
is refused at load, not half-used. This makes providers plural, which is what **U28** (choose the
active provider by capability, not list order) now has to answer.

**RULED (owner decision session 2026-07-26): the "built-ins become rows too" half is Option A —
build it now, no permanent exemption.** The additive `ConfigSnapshotProvider` shipped 2026-07-27;
this closes the other half the original ruling asked for — the built-ins stop being a hardcoded
`Vec` (`core/snapshot.rs:528`) and go through the one loader, so the mechanism is proven by the
shipped providers and cannot drift into a privileged path nobody tested (the K17/U1 invariant).

- **btrfs, zfs, timeshift and lvm become argv rows** in `adapters/snapshot.toml`. Their live
  restore is validated on a Linux box with those filesystems afterwards; the argv and wiring are
  unit-tested here.
- **Windows System Restore becomes a row too — but via typed-placeholder substitution, not a
  free-text template.** This was the one point that needed the owner: Windows System Restore is
  typed PowerShell cmdlets (`Checkpoint-Computer`, `Restore-Computer -RestorePoint {id}`) run
  elevated, and SEC5 closed an injection hole there by making the id a `u32` and the label a fixed
  enum. The owner ruled it stays a row rather than a hand-written exemption, on the condition that
  the loader substitutes the id only as a validated `u32` and the label only as the enum value —
  never raw interpolation — so SEC5's property is preserved by construction. This is buildable and
  testable on the Windows host itself, so nothing about the Windows row waits on foreign hardware.
  Reasoned in **V.82**.

## U28

**Status: ANSWERED — ruled 2026-07-26.**

**U28 — Does a machine use one snapshot provider or several, and is the active one chosen by
capability rather than list order? (XIII.23.)** `SnapshotManager::new` takes the first available
provider and stops. But a machine can have a btrfs `/` and a ZFS data pool at once, and they are
not equal: ZFS restores a running system live, btrfs cannot (V.60). Choosing by vec order means
a btrfs-first machine silently gets the weaker safety net when a live-capable one is present.
*Recommendation:* prefer a `Live` provider over a create-only one when both are available,
independent of registration order; leave "several active at once" (snapshot every provider,
restore from the best) as a later question, since one strong provider is the safety net and N is
an optimization. Blocked by nothing — but it is the wrong default to leave in place once U27
makes providers plural.

**RULED (owner, 2026-07-26): a declared priority list, exactly like package managers.** The active
provider is not chosen by LiNix guessing from capability, and not by whatever order the providers
were registered — it is chosen by an **ordered list the user declares**, the same mechanism
package-manager `priority` already is: the first provider in the list that is available on this
machine becomes the active safety net. A default order ships and the user overrides it, exactly as
`priority` does for backends.

- **The list decides *which* provider; V.60 decides what LiNix *promises* about it.** These do not
  conflict. If the declared order puts a create-only provider first, LiNix uses it and **says so
  before the change** — the pre-change notice states which kind of snapshot this machine takes, so
  a weaker net is a visible choice, never a silent one. A provider that cannot restore a running
  machine still refuses the rollback; the list cannot make it promise what it declared it cannot
  do.
- **One active provider, first-available-wins.** "Snapshot with every provider and restore from
  the best" (belt-and-suspenders) stays a later question — one strong provider is the safety net;
  N is an optimisation, not the floor.

**Implementing call:** the ordered list is its own preference, sitting with the snapshot settings
rather than jammed into package `priority` — one-question-per-file (U10) — but it *is* the
`priority` shape (an ordered list of names, default shipped, user overrides), not a new one.

---

## U29

**Status: ANSWERED — ruled 2026-07-26.**

**U29 — Is APFS local-snapshot the macOS safety net, and does an APFS restore count as `Live` or
`NotFromRunningSystem`? (XIII.24.)** U6 is ruled — the Linux-only snapshot promise is documented
as such — but macOS ships APFS with local snapshots (`tmutil localsnapshot`, `diskutil apfs`)
and LiNix uses none of it, so the pre-sync snapshot / `rebuild` revert / `rollback` are simply
absent on the second supported platform. An `ApfsProvider` is the natural first customer of
U27's registry. *Recommendation:* build it, and answer the capability honestly — an APFS
snapshot that can only be restored by rebooting into the recovery environment is
`NotFromRunningSystem`, not `Live` (V.60). Whether macOS parity is *scheduled* or merely *listed*
(XIII.4) is the owner's call; the *capability* question must be answered before the provider
ships, whenever that is.

**RULED (owner, 2026-07-26): yes — macOS gets APFS as its snapshot provider, and it is built.**
APFS local snapshots (`tmutil localsnapshot`, `diskutil apfs`) become the macOS safety net, as a
provider row on U27's mechanism — the natural first customer of the plugin door. Its restore
capability is declared honestly: an APFS snapshot restored only by rebooting into the recovery
environment is **create-only, not live** (V.60), so it saves state and offers recovery-mode
restore rather than pretending to be a running-machine undo. This closes the platform gap U6
documented: macOS is no longer without a net, it has a create-only one, marked as such.

**Governance (owner, 2026-07-26): there is no "listed but not scheduled" — everything ruled to
build gets built.** The recommendation offered scheduling as the owner's call; the owner removed
the option. **The XIII.4 listed-vs-scheduled distinction is retired.** A decision that says "build
it" is scheduled work, not an acknowledgement filed for later, and this applies to every "open it
/ build it" ruling in this register.

---

## U30

**Status: ANSWERED — ruled 2026-07-26.**

**U30 — Is "declare a storage object" a family (zfs datasets, lvm volumes, btrfs subvolumes) or
separate backends, and what does the guard owe a removal that destroys a filesystem? (XIII.25.)**
`backends/btrfs.rs` declares subvolumes as objects; there is no zfs-dataset or lvm-volume
equivalent, though each is the same declared-sized-mounted noun. They do not fit `ManagerConfig`,
so it is Rust regardless — the question is one shared trait versus three backends. *The half that
is not cosmetic:* `btrfs:` remove runs `subvolume delete`, which destroys a filesystem, and a
zfs-dataset `remove` (`zfs destroy`) is the same at larger blast radius. **Every removal path
calls the guard (`app/sync/guard.rs`), and this one must too — verified from the code, not
assumed** (the II.10 lesson: a removal path nobody names is a removal path nobody guards).
*Recommendation:* settle the guard's contract for filesystem objects — at minimum, a declared
storage object with data on it is never destroyed without the gate a protected package gets —
before any second storage backend grows a `remove`.

**RULED (owner, 2026-07-26): one family, and the ordinary guard — no special escalation.**

- **Shape: one family.** zfs datasets, lvm volumes and btrfs subvolumes are the same
  declared-sized-mounted noun, so they share one trait, not three backends. (The owner agreed the
  cosmetic half.)
- **Guard: the normal gate, and it must fire.** A removal that destroys a filesystem goes through
  `app/sync/guard.rs` exactly as a protected-package removal does — the same mass-removal-style
  confirmation, no stronger dedicated gate and no refuse-to-auto-destroy special case. What is
  **not** optional is that the guard fires at all: this is a removal path, it destroys a
  filesystem, and II.10 says every removal path calls the guard — so a `zfs destroy` / `subvolume
  delete` reached by removing a line is never silent, it is guarded like any destructive change.
  The owner chose the normal gate over the two stricter options (empty-only auto-destroy;
  never-auto-destroy-a-populated-volume) — a storage removal is guarded, not special-cased.

The line that was genuinely open — *does a filesystem-destroying removal earn MORE than the normal
gate* — is answered **no**. It earns exactly the guard every removal earns, and the danger is met
by the guard firing, not by a second heavier gate.

**And a storage object is protectable exactly like a package (owner, 2026-07-26).** The guard's
existing protection — the `keep.txt` / "never remove" mark a user puts on a package the guard then
refuses to remove — applies to storage objects too: a user may protect a volume, and a protected
volume the guard **refuses to destroy** at all, the same way it refuses a protected package. This
is not a new mechanism; it is the one protection vocabulary reaching a new noun, which is the
right shape (no two of everything). So the danger has two answers together: the guard fires on
every storage removal (the normal gate), and a volume a user cares about can be marked so the
guard will not destroy it even then.

**Build note (owner, 2026-07-26): the providers do not exist yet and must actually be built.**
Only `backends/btrfs.rs` declares storage objects today; **zfs datasets and lvm volumes have no
implementation at all.** This ruling is the contract for code that is still owed — the shared
family trait, the zfs (`zfs create`/`destroy`/list) and lvm backends, the guard wiring and the
protection mark — all of it is work to do, not work done. Do not read this entry as "storage
objects are finished."

---

## U31

**Status: ANSWERED — ruled 2026-07-26.**

**U31 — Should health checks be an open vocabulary — a user-declared check command — rather than
a fixed set? (XIII.26.)** A health-checked upgrade (XIII.5) rolls back when the machine is
"unhealthy", but health is only what LiNix already knows how to test; a user whose service must
answer on a port, or whose config file must parse, cannot express it. A check is argv with exit
0 = healthy — the most check-shaped extension there is. *Recommendation:* open it, on the II.12
hook trust model (a check command is argv from a file, and the file may travel), and fail loud —
a check that cannot run is a failed check, not a passed one, or "healthy" quietly comes to mean
"the check was broken" (V's silent-wrongness). Not blocking: the built-in checks work; this is
the difference between a safety net LiNix designed and one the user can shape.

**RULED (owner, 2026-07-26): open it, as recommended.** Health checks become an open vocabulary —
a user declares a check command (argv, exit 0 = healthy) beside the built-in checks. Two rules,
both the standard ones:

- **Fail loud: a check that cannot run is a *failed* check.** If LiNix cannot execute the check,
  the result is "unhealthy" and the change rolls back — never "assume healthy". Otherwise "healthy"
  silently degrades to "the check was broken", the exact silent-wrongness this design refuses.
- **Same trust as every runnable thing:** a check command is argv a shared config repo can carry,
  so it rides the II.12 hook ledger and is approved by `linix lock`, not a new trust model.

The exact schema is implementation, not a further ruling.

---

## U32

**Status: ANSWERED — ruled 2026-07-26.**

**U32 — Do modules take parameters (the macro LiNix doesn't have), and is a parameter's type
checked? (XIII.29.)** A module is a named set of declarations that cannot take an argument, so two
machines wanting *almost* the same set copy it and drift. *Proposed:* `param user` / `param gpu =
none` in the module, `use workstation(user=shaul, gpu=nvidia)` at the call site; substitution is
`vars`' existing interpolation reaching into the module's parameters, and the expansion is
ordinary declarations, visible in `linix eval` and the removal preview before it runs. A missing
`param` with no default is a **loud error naming module and parameter**, never an empty string
that makes a `when` silently false (P3, the failure `vars` was hardened against). *The actual
decision:* whether a parameter is typed — a `gpu` that must be one of a named set versus free text.
A typed parameter is a second closed vocabulary the user defines: it names its legal values in the
error (VIII.2's virtue) but is also a second place a name can be misspelled. *Recommendation:*
build parameters; make types opt-in (free text with a loud "missing" is the floor, a named set is
sugar on top), so the feature is useful before the type system is finished.

**RULED (owner, 2026-07-26): build it, and the user decides per parameter whether it is typed.**
Modules take parameters — `param user`, `param gpu = none` in the module, `use workstation(user=
shaul, gpu=nvidia)` at the call site, substituted through `vars`' existing interpolation, expanding
to ordinary declarations visible in `linix eval` and the removal preview before anything runs. Two
things are fixed:

- **A missing required parameter is a loud error naming the module and the parameter** — never an
  empty string that makes a `when` silently false (P3, the failure `vars` was hardened against).
- **Types are the user's choice, per parameter.** Free text is the floor; a user may declare a
  parameter typed (a named set of legal values, whose error lists them — VIII.2's virtue) where
  they want that check, and leave it free text where they do not. The same principle as the rest of
  this round — the user can shape it — applied to the parameter's own vocabulary. Opt-in, so the
  feature is useful before any type system is finished, and never forced where free text is fine.

---

## U33

**Status: ANSWERED — ruled 2026-07-26 (owner overrode the recommendation; amends U3 and U4).**

**U33 — Are generated declarations wanted at all — a config that runs a program to *produce*
state, not describe it? (XIII.30.)** `vars` already lets a *value* come from a command through the
hook ledger; this is a whole *declaration* from a command ("install whatever `./pick-python.sh`
prints"). It is `read`/`eval` with `read`/`eval`'s liability: the config's behaviour stops being
knowable by reading it. LiNix already treats the neighbouring feature as radioactive — `exec:` is
"run a thing", and U3/U4 confined it to actions with no inverse, explicitly *not* installing
software; a generator that emits installs walks back to that line, now able to *generate* the
`exec:` (XIII.14's fear). If ever built, only under exec's constraints: output passes the guard
and the removal preview as if typed; it runs through the II.12 ledger (V.55); a failed generator
is a failed sync, never a silently empty set (VI.0). *Recommendation:* **not yet, and possibly
never.** `vars` covers values, U32 covers reuse, and what remains is precisely the unknowable-by-
reading property this design exists to refuse. Filed so the answer is a recorded *no* rather than
a gap someone fills quietly.

**RULED (owner, 2026-07-26): build it — and `exec:` may do anything — each gated by a config key.**
The recommendation was a recorded no; the owner overrode it deliberately. Both powers ship, and
both are **off by default behind an explicit config key** — the deliberate opt-in shape LiNix
already uses for every dangerous capability (`--allow-mass-removal`, `@allow_http`, `linix lock`).
A user who wants the power turns the key; a user who does not is never exposed to it.

- **Generated declarations exist.** A config may run a program that *produces* declarations, gated
  by its own key. What it emits is **not** exempt from the machine's safety: the output passes the
  guard and the removal preview exactly as if it had been typed, it runs through the II.12 approval
  ledger (V.55), and a failed generator is a **failed sync, never a silently empty set** (VI.0,
  fail-loud — a Part I principle, so it holds regardless of the key). The key buys the
  unknowable-by-reading tradeoff; it does not buy silence.
- **`exec:` may install software and do anything, gated by its own key. This amends U3 and U4.**
  U4 said `exec:` is not a licence to install software; the owner has lifted that categorical
  refusal and replaced it with a key. U3 still governs *removal*: `exec:` has no automatic inverse,
  so an `exec:` that installs still leaves the software when the line goes unless `@undo=` is given,
  and `plan` still says so. The onboarder remains the **better** path for anything installable (it
  gives a noun that removes/lists/locks); it is no longer the *only* permitted one.
- **Two keys, not one** ("a key which controls each") — generated declarations and `exec:`-anything
  are separately gated, so enabling one does not silently enable the other.

**Downstream, owed:** U3 and U4 entries carry an amendment note pointing here; the README's `exec:`
boundary (U4's deliverable) and XIII.14's fear are revisited when this is built. The Part I
principles (fail-loud, the guard, the plan showing every change) are **not** waived by either key —
they are what keeps an opened door from being a silent one.

---

## U34

**Status: ANSWERED — ruled 2026-07-26.**

**U34 — Is `linix repl` worth a second entry point, or is `linix eval | jq` enough? (XIII.31.)** A
read-only prompt that resolves a name against *this* machine, evaluates a `when`, and expands a
`use workstation(gpu=nvidia)` — answering "what does this resolve to here" by trying it. It is
`eval` (XIII.15) with a cursor and must share the same parser and resolver, never a second
implementation (the U20 rule). *Recommendation:* low priority — real value for anyone authoring a
config, but `eval` already exposes the model, so this is ergonomics, not capability. Worth it only
if it stays a thin front end over the existing engine.

**RULED (owner, 2026-07-26): build it — if it is easy.** `linix repl` ships as an interactive
read-only prompt for authoring a config. The owner did not require it be a literal thin wrapper,
only that it be easy — so the implementation shape is free, **with one non-negotiable that is a
correctness rule, not a style choice: it shares the one parser and resolver, never a second
implementation (U20).** Two engines drift and then disagree, which is the failure this rewrite
exists to end; that constraint holds however the repl is structured. It is ergonomics over the
model `eval` already exposes, so if it turns out not to be easy while staying single-engine, it is
not worth a half-build.

---

## U35

**Status: ANSWERED — ruled 2026-07-26 (owner widened past composition-only).**

**U35 — May a user name a new verb, strictly as a composition of built-ins? (XIII.31.)** LiNix has
~sixty commands (XIII.8) and no way to add the sixty-first. A verb that *sequences* existing verbs
— `linix refresh` = `sync`, then `upgrade`, then the fleet report — is `defun` over the command
surface, and safe because it composes audited operations rather than producing new ones. **The
line:** a user verb sequences built-in verbs and nothing else; the moment it runs arbitrary argv it
is `exec:` wearing a command's clothes, which U4 already settled as no. *Recommendation:* build it
with that boundary hard-coded — composition only, no shell — so the safe 90% ships without
reopening the `exec:` trust question the dangerous 10% would.

**RULED (owner, 2026-07-26): build it, and a user verb may run arbitrary commands too.** The
recommendation held the line at composition-only; the owner widened it, consistent with U33 letting
`exec:` do anything. So a user verb has two registers, and **one door, not two**:

- **Composing built-in verbs is safe and ungated.** `linix refresh` = `sync`, then `upgrade`, then
  the fleet report — it sequences audited operations and produces nothing new, so it needs no key.
- **Running arbitrary commands rides the `exec:` trust model from U33.** The moment a user verb
  runs argv of its own, that portion is the same power `exec:` is, so it inherits the same
  controls: gated behind U33's `exec:`-anything config key (off by default), approved through the
  II.12 ledger, and never exempt from the guard, the plan and fail-loud (Part I). It does **not**
  get a second, looser trust question of its own — that is the mistake the composition-only
  recommendation was guarding against, and routing arbitrary-command verbs through U33's existing
  gate answers it without a new mechanism.

---

## U36

**Status: ANSWERED — ruled 2026-07-26.**

**U36 — Are init systems a declared-provider kind, or does the built-in enum stay closed? (XIII.34.)**
`backends/service.rs` is a fixed `enum InitSystem` (Systemd, OpenRC, SysVinit, launchd, Windows
`sc`) behind a hardcoded command table; s6, dinit, runit, GNU Shepherd and appliance inits are
unreachable, and a `service:` line on such a host has no branch to take. It is the snapshot vec's
problem in another file, and the **lowest-risk** surface to open — start/stop/enable are ordinary
reversible operations with no data to destroy. *Recommendation:* open it as a `[[init]]` block on
XIII.33's mechanism; it is the cleanest fit the mechanism has, and P7 is better served by "write
six lines" than by "unsupported". Not blocking — the five built-ins cover most machines.

**RULED (owner, 2026-07-26): open it, as recommended.** Init systems become a declared-provider
kind — a `[[init]]` block on the same plugin mechanism as custom backends, setting stores and
snapshot providers, so s6, dinit, runit, GNU Shepherd and any appliance init are reachable by
writing a small file rather than shipping a release. The five built-ins become rows read through
the same loader (no two of everything), and a declared init system is argv a shared repo can carry,
so it rides the II.12 ledger and `linix lock` approves it. This is the lowest-risk surface —
start/stop/enable are reversible and destroy no data — so nothing beyond the standard trust rule
is owed. The schema is implementation.

---

## U37

**Status: ANSWERED — ruled 2026-07-26.**

**U37 — Are notification channels their own declared-provider kind, or is an event hook the
answer? (XIII.35.)** `app/scheduler/notify.rs` handles only `desktop`, `email`, `both` and warns
"unknown channel" for the rest, so Slack, ntfy, webhooks, Telegram, paging — every channel a real
fleet uses — is absent. **The overlap with XIII.13's event hooks is the decision:** a hook can
already shell out to `curl` on a sync or a guard refusal, so "notify me on Slack" is *possible*
today; the question is whether a first-class `[[channel]]` block earns its keep on top of that.
*Recommendation:* do not add a second mechanism — route non-built-in channels through the event
hook that already exists, and document it — unless a channel needs something a hook cannot express
(per-level routing), the only thing that would justify a block of its own. Filed so the answer is
a recorded decision, not a fifth channel bolted on next time someone asks.

**RULED (owner, 2026-07-26): no new mechanism — route through the existing event hook, and
document it.** Slack, ntfy, webhooks, Telegram and paging are reached by the event hook U15 already
shipped (a hook that runs `curl` on `after_sync` / `on_drift` / `on_guard_refusal`), not by a new
`[[channel]]` provider kind. A dedicated channel block would duplicate the hook — two ways to do
one thing, the disease this rewrite cures. The one thing that could have justified a block of its
own, per-severity routing, the owner did not ask for, so it is not built. The work owed here is
**documentation** — a copyable Slack/webhook hook example — not a mechanism. If per-level routing
is ever wanted, that, and only that, reopens the block question.

---

## U38

**Status: ANSWERED — ruled 2026-07-26. The T-series gate is now clear.**

**U38 — Is secret decryption a declared-provider kind, and behind which T-series rulings?
(XIII.36.)** `model/secret.rs` is built around `age` (age plugins, hardware tokens); sops, Vault,
1Password, cloud KMS and GPG have no way in, though each is "run a command that turns a reference
into plaintext" — XIII.33's shape exactly. **This is the surface where openness is not cheap.** A
decrypt provider's output *is* a secret: a bad one writes plaintext to disk, leaves it in the
process table, or logs it — the failure `secret:` exists to prevent. So a declared secret provider
is bound by the T-series handling rules LiNix argued for age (no-disk / in-memory / no-log, T7
reopened), and one that cannot promise them is refused, not trusted. *Recommendation:* yes in
principle — the mechanism is identical and users genuinely have other secret managers — but **not
before the T-series settles how plaintext is handled**, because opening this surface first hands an
unaudited command the one thing LiNix promises to guard. Safe order: rule the T-series, then open
the door the mechanism already makes trivial.

**RULED (owner, 2026-07-26): open it — the T-series is settled, so the gate is clear.** The
plaintext-handling rules this surface waited on are all ruled: decrypt never silently backs up
(T1), plaintext cannot be written back into the repo (T2), it is created with locked-down
permissions (T5), and LiNix does not hold secrets in process memory (T7, ruled out). With those
fixed, secret decryption becomes a declared-provider kind — sops, Vault, 1Password, cloud KMS and
GPG reachable on the same plugin mechanism and `linix lock` approval as every other surface.

- **The safety cost is real and is paid by a refusal, not a warning.** A decrypt provider's output
  *is* a secret, so a declared provider is **bound by the T-series handling rules** (no plaintext
  to disk, none left in the process table, none logged), and **one that cannot promise them is
  refused, not trusted.** Openness here is not free the way it is for init systems (U36): the
  provider proves it handles plaintext safely, or it does not run.
- Everything ruled gets built (2026-07-26 governance), so this is scheduled work, not a filed
  intention — now that its gate is open.

---

## U39

**Status: ANSWERED — ruled 2026-07-26.**

**In the tree today:** built in the same commit as this ruling. `capability::INSTALLS_FROM_SOURCE`
is the one table — `("helm", "url")` — read by both ends: the grammar admits `@url` as a package
option and refuses it by name on every backend that installs by name, and `register_helm` builds
its `ManagerConfig::install_source_option` from the same row. Both harnesses run helm's full
lifecycle where they previously named it as an open bug and skipped it.

**The first version of this shipped broken, and only a real `helm` said so.** The unit tests built
a `PackageSpec` by hand, so nothing ever asked the grammar whether `@url` was a legal key — and it
was not: II.2's option table is closed, so every `helm:diff@url=…` line was refused as a
misspelling while every test passed. `capability.rs` now has a test that every install-source key
is in `PACKAGE_OPTION_KEYS`, which is the drift this could otherwise repeat.

**U39 — When a manager installs by one string and removes by another, which one is the
declaration?** `helm plugin install` takes a URL; `helm plugin list` and `helm plugin uninstall`
speak the name in the plugin's own `plugin.yaml`. A LiNix declaration carries one name, so
whichever half it named, the other half broke — and naming the URL is the half that breaks
**silently and permanently**: the install succeeds, then every later sync sees a package that
`list` never mentions, tries to remove it, and fails identically. One helm plugin wedged every
operation after it. Proven by a real run on the `tools` image, and it is not helm-specific in
principle — it is the general question of which string is the identity.

**RULED (owner, 2026-07-26): the name is the identity, always. Install-time data rides in an
option.** A declaration is `helm:diff@url=https://github.com/databus23/helm-diff`.

- **The identity is the string the manager will still answer to later.** Install runs once;
  list and remove run on every sync forever. A declaration that names the install argument is
  a declaration LiNix can never check or undo, which is the opposite of what a declarative
  model is for.
- **A declaration missing its source is refused, not guessed.** Deriving `diff` from
  `.../helm-diff` is right often enough to be trusted and wrong often enough to install a
  plugin under a name nothing can remove; the real name lives in a `plugin.yaml` that cannot
  be read before the plugin is fetched. The refusal names the fix.
- **`go` already had the right shape and is why this is a rule rather than a helm patch.**
  `go install` takes a module path and Go ships no uninstaller, but the module path is what
  `go version -m` reports back, so the identity survives — and removal derives the binary from
  it. `web` is consistent the other way (the URL is the identity at install, list *and*
  remove). helm was the only backend where the two vocabularies differed and the code assumed
  they did not.

---

## U40

**Status: ANSWERED — ruled 2026-07-27.**

**In the tree today:** built in the same commit as this ruling. `RawExecutor` carries a
`ChildStdin` policy — `Closed` for the reader layer, `Interactive` for the mutating one — and
captures stdout and stderr on both. `run_on` sets `SYSTEMD_PAGER`/`PAGER`/`GIT_PAGER` on every
spawn, and the systemd rows in `init_providers.toml` carry `--no-pager`.

**U40 — Does a command LiNix runs write to your terminal, or to LiNix?** It had been answered
by accident, and both ways at once: `RawExecutor::execute` asked whether *LiNix's own stdin* was
a terminal and, if it was, handed the child all three handles. That meant the answer differed
between a human and CI, and the human got the worse half of it — with stdout inherited,
`output.stdout` came back empty and all 79 `run_output` call sites parsed an empty string. `linix
list -b apt` reported **609 packages piped and 1 under a terminal**. Nothing looked broken,
because what reached the screen was `dpkg-query`'s own output, which reads like a package list.
The same inheritance let `systemctl` decide a human was watching and start a pager, so read-only
`linix status` hung on a keypress and had to be killed, and printed 80, 640 and 83 lines across
three identical runs.

**RULED (owner, 2026-07-27): LiNix reads every command's output, and shows you what it read.**
Capture is a property of the call, never of the terminal LiNix happens to have.

- **stdout and stderr are always captured, on every path, on every platform.** A parser that
  works in CI and not on your machine is worse than one that never works, because only one of
  the two gets reported.
- **stdin is the one stream a child may share, and only a mutation may share it.** `sudo` asks
  for a password on the terminal it was started from; a read has nothing to ask and nobody to
  answer it. `sudo` writes its prompt to `/dev/tty`, so the prompt still reaches the user with
  stderr captured.
- **A long mutation still shows its progress.** The bytes go both places: captured for the
  caller, mirrored to stderr as they arrive when a terminal is attached. Mirrored to stderr and
  never stdout — stdout carries LiNix's own answer, and a manager's chatter interleaved with it
  is not parseable by whoever piped us.
- **Pagers are suppressed at the spawn, not left to the absence of a terminal.** Capturing
  removes the usual trigger, but `$PAGER`/`$SYSTEMD_PAGER` forces one anyway. The suppression
  goes in the one env map every spawn inherits, and `--no-pager` on every systemctl row besides.
- **No config key, no environment variable, no `--capture` flag.** One path. A switch here is a
  switch that turns the bug back on.

**The blind spot is the finding, not the defect.** 1,324 tests, four container lifecycles and
three OS builds all ran with pipes on every handle, so not one of them could observe any of
this. `tests/pty_tests.rs` runs the built binary under `script -qec` against a stub manager on
`PATH` and asserts that what LiNix printed is what LiNix parsed; it is a named step in CI's fast
half. Confirmed to fail against the previous behaviour before it was made to pass.

---

## U41

**Status: ANSWERED — ruled 2026-07-27.**

**In the tree today:** built in the same commit as this ruling. `Transaction` records a `Prior`
per node before the node runs, holds the user's `Config`, and its rollback calls
`guard::protection_of` before any compensating removal.

**U41 — What does a rollback do when the guard refuses one of its compensating removals?**
`transaction.rs` contained **zero** references to the guard. `guard::enforce` runs at plan time
over the planner's `Remove` nodes; a rollback's removals are issued at execution time and never
passed through it, so `protected_packages` and OS-essential protection did not apply to them —
in direct contradiction of the project's own rule that every path that removes calls the guard.
Wiring it in raises the question the wiring cannot answer: a refused compensating removal leaves
the transaction **partly applied**, and what LiNix should then do is a product decision.

**RULED (owner, 2026-07-27): the guard wins, and the rollback says what it could not undo.**

- **A protected package is not removed, by any path, for any reason.** A refusal is a refusal
  (V.26); it is not softened because the caller is a recovery path. V.64 already says these
  paths need the guard *more* than ordinary ones, because they run outside the plan the user
  read and usually when nobody is watching.
- **The partly-applied state is reported, never hidden.** `rollback` already returns an error
  naming every compensating action that failed; a guard refusal joins that list, by name, with
  the reason. The user is told exactly which package is still installed and why.
- **What LiNix does not know, it does not delete.** A prior state the manager could not report
  is `Unknown`, and an `Unknown` is never read as "it was not there". The package stays and is
  named in the report. Guessing the other way deletes software this run never installed.
- **An upgrade is compensated by the old version, not by an uninstall.** `spec_is_missing`
  schedules an `Install` node for a package that is already present when a `@version=` or
  `@channel=` changes, so compensating every `Install` with `remove()` turned a failed upgrade
  into an uninstall. Rollback now reinstates the version the package was on; where the manager
  reported no version, it says so and leaves the package alone rather than removing it.
- **A rolled-back removal comes back pinned.** The reinstall carried `options: HashMap::new()`,
  so a package restored after a failed removal came back at whatever is newest — the declared
  pin silently gone.

---

## U42

**Status: ANSWERED — ruled 2026-07-27.**

**In the tree today:** built in the same commit as this ruling. `undo` is gone as a top-level
command; the gallery it opened is `linix snapshot restore`, beside `snapshot list` and
`snapshot prune`.

**U42 — Do the overlapping command clusters get consolidated?** A review counted 45 top-level
commands and named four clusters that overlap "without a clear rule for choosing", the headline
being that `status` and `prune` are two views of one drift computation and that
`undo`/`rollback`/`generation` are three vocabularies for restoring prior state.

**Measured against `linix --help` before the ruling, because the description was wrong.** The
surface is **62 entries, not 45**, and **ten of the thirteen commands named do not exist**:
`remove`, `prune`, `orphans`, `clean`, `unmanaged`, `status`, `doctor`, `migrate`, `clone`,
`generation`. Both headline examples are about commands that are not in the program. An audit
reads what is written; only running it reads what is there — and this one was never run.

**RULED (owner, 2026-07-27): no consolidation. One rename, because the fault was a name.**

- **The removal cluster stays whole. It is not a cluster of synonyms** — each verb does
  something no other one does: `uninstall` a package; `remove-orphans` what the manager itself
  calls an orphan; `purge-unmanaged` everything LiNix does not manage; `unmanage` forget a
  package but keep it installed; `reset` forget everything but keep everything installed;
  `clean-cache` archives and no packages. Collapsing any two removes a capability, and II.17's
  deletion list is the only approved removal.
- **The one real overlap was a name, not a redundancy.** Going back has two mechanisms —
  filesystem snapshots, and the git manifest history — and `undo` was the *snapshot* one while
  `history` (a TUI) and `rollback <ref>` (a CLI) are two interfaces onto the *manifest* one.
  A user wanting to undo their last sync reached for `undo` and got the wrong mechanism.
- **The gallery is now `linix snapshot restore`.** Its name says which of the two it is, and it
  sits with the other snapshot verbs. `undo` is not reassigned to anything: a word that meant
  the wrong thing does not improve by meaning a second thing.
- **`history` and `rollback` stay as they are.** A TUI and a flag-driven command over one
  mechanism are an interface pair, not two vocabularies.

**Also fixed under this ruling:** two user-facing messages pointed at `linix doctor`, which is
not a command. They say `linix check`.

---

## U43

**Status: ANSWERED — ruled 2026-07-27.**

**In the tree today:** built in the same commit as this ruling.

**U43 — How much does an ordinary run say about itself?** The default filter was `info` and
there are 256 `info!`/`warn!` call sites, so every run printed LiNix narrating its own startup
above the answer the user asked for — `No state file found at …`, on **every** run, because a
read-only command never writes the registry it just reported missing.

Three things were true of the flags and only one of them was known: `--verbose` promised
"debug-level logging" and produced **none** (measured: 0 debug lines, against 5 for
`RUST_LOG=debug`), because the subscriber was built at `main.rs:41` and clap did not parse until
`:81`, so `cli.verbose` reached the executor and never reached the filter. `--quiet` did not
touch the log stream either.

**RULED (owner, 2026-07-27): an ordinary run prints its answer and nothing else.**

- **Default `warn`.** What was on the `info` channel was commentary, not answer.
- **`-v` is info, `-vv` is debug, `-q` is errors only, and `-q` beats `-v`** when both are
  given — a run that says both meant the quiet half or it would not have typed it. `RUST_LOG`
  outranks all of it, unchanged.
- **The level is read from argv, not from the parsed `Cli`.** It has to be live before the shim
  hijack, and reading it after clap is exactly why `--verbose` did nothing.
- **A command's answer goes to stdout; only narration goes to the log.** This is the half that
  had to land first: `sync` reported `already up to date` at `info!` with nothing on stdout, so
  dropping the default without moving it would have made a no-op sync silent. Twenty-three
  lines moved — the whole of what `lock` and `unlock` report, `rebuild`'s skips and completion,
  `try`'s verdict, `config init`, `heal`'s repairs, the no-op sync and the no-op upgrade.

---

## K18

**Status: ANSWERED — ruled 2026-07-24.**

**In the tree today:** file writes are already atomic one file at a time — `write_atomic` stages
a temp file and renames it into place, so a `link:` target is never half-written. **Package
swaps are not atomic and mostly cannot be**, and the existing answer is K3's snapshot: one taken
before the first removal, reverted if the reinstall fails.

**K18 — Should LiNix use a backend's own atomic mechanism where one exists (owner question,
2026-07-23)?** Asked as *"is there any way to make each swap atomic?"* The honest answer is that
it splits three ways and only the third is a decision:

- **Files: already atomic per file**, and not atomic across a set. A `link:` that writes forty
  files can be staged and renamed at the end to narrow the window, but no operating system
  offers a multi-file rename, so *narrower* is the whole of what is available.
- **Packages, on ordinary managers: no, and not by any effort of LiNix's.** `apt`, `dnf`,
  `winget` and the rest expose no transaction to join. **What LiNix already has is
  all-or-nothing in the outcome rather than in the instant** — K3's one snapshot before the
  first removal, reverted on a failed reinstall, with stop-and-name-what-is-missing where no
  snapshot provider exists. The window is real and visible; the end state is not half-done.
- **Packages, on managers that are genuinely transactional: yes, and this is the question.**
  `nix` is already a registered backend and its profile switch is a symlink flip — atomic, and
  rollback is another flip. `rpm-ostree` and `transactional-update` are the same shape. **LiNix
  drives all of them today as if they were `apt`**, taking the snapshot-and-revert path over a
  mechanism that needs neither.

*Recommendation:* a backend may declare that it swaps atomically, and where it does, LiNix uses
that instead of the snapshot path and says so in the plan — *"nix: atomic, no snapshot needed"*.
The value is not speed; it is that **the one honest sentence about a rebuild's risk changes per
backend**, and today LiNix prints the cautious one everywhere. **This is not urgent** and nothing
is blocked on it — it is filed so that the answer stops being "no" when it is only "not yet".


**RULED (owner, 2026-07-24): make it an option.** Where a backend has its own atomic swap, a config option lets LiNix use it; the default stays K3's pre-removal snapshot, because most package swaps cannot be atomic and a guarantee that only sometimes holds must be asked for, not assumed.

**RULED (2026-07-24): an option, added when a backend needs it.** Where a backend has its own atomic swap, a config option uses it; the default stays K3's pre-removal snapshot. NOT added as a dead key now: no backend currently exposes atomic swap, and this project holds that a preference that silently does nothing is worse than none (K4's own reasoning). The option lands with the first backend that can honour it — the ruling is what that backend's option will implement.

**REAFFIRMED (owner decision session 2026-07-26): stays parked, not part of this build.** The pre-real-machine build does NOT add a dead atomic-swap key. K18 lands with the first backend that actually exposes atomic swap, exactly as ruled above.
---

## T7

**Status: ANSWERED — ruled 2026-07-24.**

**In the tree today:** nothing. `app/run.rs:138` is the only place LiNix is in a process's launch
path at all.

**T7 — Runtime injection of secrets into process memory: REOPENED for discussion (owner,
2026-07-23).** XII.2 ruled this out on 2026-07-23 and told the reader not to re-open it; **the
owner has since said the conversation stays open**, so the refusal is downgraded to a question
and XII.2 is amended to say so. The reasoning that produced the refusal is not withdrawn and is
the thing to argue with:

- **It asks LiNix to be a supervisor.** For a credential never to touch disk, LiNix must be in
  the launch path of every program that reads one. That is `systemd`'s `LoadCredential`, a
  `direnv`, or a secrets agent — three things that already exist and that LiNix is not.
- **The half-measure is worse than either end.** Injecting only into children of `linix run`
  protects exactly the processes LiNix starts and none of the ones that actually read
  `~/.npmrc`, while reading as though it protected both.
- **The bar the original ruling set:** a use case that lives entirely inside `linix run`. That is
  still the sharpest question to answer first — **what program, run how, needs the secret?**

*No recommendation.* The refusal was argued; what has not been heard is the case for it.


**RULED (owner, 2026-07-24): keep it out — if it is hard, do not do it, and it is hard.** Runtime injection of secrets into process memory asks LiNix to become a process supervisor (it must stay in the launch path of every process that reads the secret), which is a different and far larger thing than a package manager. The reopening was deliberate; the ruling is to leave XII.2's refusal standing. A secret still reaches a process the ordinary way — decrypted to a file the process reads, or an env var — never via LiNix holding it in memory.
---

## D8

**Status: ANSWERED — ruled 2026-07-24.**

**D8 — `when` inside an options body.** II.2 says a declaration's body is options, so
`github { when family == debian { … } }` is not legal today, and VIII.2's example wraps the whole
`github` block in a `when` instead. That works but gets repetitive across four families.
*Recommendation:* keep it illegal. The wrap form is uglier and does not need a new grammar rule,
and a new block kind here is how the grammar starts growing exceptions.


**RULED (owner, 2026-07-24): keep it illegal.** `when` inside an options body stays disallowed. Wrapping the whole `github { … }` block in a `when` works and needs no new grammar rule; a new block kind here is how the grammar grows exceptions.
---

## D11

**Status: ANSWERED — ruled 2026-07-24.**

**D11 — The default order is detected, so a LiNix upgrade can change it.** A machine with no
`formats` line that installs a `tarball` today could install a `deb` after an upgrade. The lock
protects an existing install; a fresh `linix lock` or a new machine does not. *Recommendation:*
treat the default order as versioned and say so in the changelog when it moves — or accept the
churn explicitly. Not decided.


**RULED (owner, 2026-07-24): yes, version the default order.** The detected default artifact order carries a version constant; when it moves, the changelog says so. A machine with no `@formats=` line is then told, rather than silently installing a `deb` after an upgrade where it installed a `tarball` before.
---

## D12

**Status: ANSWERED — ruled 2026-07-24.**

**D12 — Network, rate limits, and offline.** Listing assets is a GitHub API call per repo.
Unauthenticated is 60/hour, which a repo with thirty `github:` lines exhausts on the second
`sync`. `LINIX_GITHUB_TOKEN` exists (II.1). *Recommendation:* resolve from `locks/github` without
any API call when the lock is present and the version is pinned; only `linix lock` and an
unpinned line hit the network. Needs deciding because it determines whether `sync` works on a
plane.


**RULED (owner, 2026-07-24): resolve from the lock offline.** A pinned `github:` line resolves from `locks/github` with no API call; only `linix lock` and an unpinned line hit the network. `lock` is what freezes the resolved asset/version, so a later `sync` reproduces it without the 60/hour unauthenticated GitHub limit. This is what makes `sync` work offline and on a repo with many `github:` lines.

**Already built (`answered_locally` in `github.rs`).** A pinned line with a lock and matching on-disk assets resolves with no API call; only unpinned lines and `linix lock` hit GitHub. The ruling described existing behaviour — no new code needed.
---

## D13

**Status: ANSWERED — ruled 2026-07-24.**

**D13 — Changing a `channel` — refresh or reinstall?** `snap refresh --channel=edge` is not
`snap remove && snap install`, and moving `edge → stable` is usually a downgrade. **A downgrade
is a removal-shaped event and the guard should see it.** *Recommendation:* refresh where the
backend supports it, and route the downgrade case through the plan and the guard like any other
destructive change.


**RULED (owner, 2026-07-24): refresh, and route a downgrade through the guard.** Changing a `channel` refreshes in place where the backend supports it (`snap refresh --channel=`), and the downgrade case (`edge → stable`) goes through the plan and the guard like any destructive change, because a downgrade is removal-shaped.

**RULED, NOT YET BUILT (2026-07-24).** Refresh where the backend supports it; route a channel downgrade through the plan and guard. Queued rather than built: it needs the planner to detect a *channel change* (query the installed channel, compare to the declared one — the planner currently checks version, not channel) AND a notion of channel ordering to tell a downgrade from an upgrade, both of which touch the change-detection core. Deferred to avoid a risky half-change there; the ruling is settled for when it is built.
---

## D14

**Status: ANSWERED — ruled 2026-07-24.**

**D14 — Does `why` explain the selection?** When `github:x/y` installs a `.tar.gz` on a machine
the user expected a `.deb` on, the answer lives in three places (line, `priority`, built-in
default) and `linix why` is the command that should say which one won. *Recommendation:* yes,
and it is a small amount of work only if the resolver keeps the reason rather than just the
result. Decide before the resolver is written, not after.


**RULED (owner, 2026-07-24): yes.** `linix why` explains WHICH rule selected the artifact — the line's `@formats=`, `priority`, or the built-in default. The resolver must keep the reason, not just the result; decided before the artifact resolver is finalised so the reason is retained rather than reconstructed.

**BUILT, 2026-07-24.** The artifact lock records `selected_by` — which rule chose the file (`@asset=` pattern, `@formats=` line, or the built-in default). `linix why <pkg>` shows `selected: <asset> — chosen by <reason>`, read from the lock with no network re-selection.
---

## D17

**Status: ANSWERED — ruled 2026-07-24.**

**D17 — Regex lines.** What `github:re:…@formats=` means when one pattern spans repos with
different asset sets is unspecified. *Probably:* the list applies to each match independently and
a match with no legal asset is the VIII.2 error, named per repo. Not decided, and low urgency —
`github:re:` is rare in practice.


**RULED (owner, 2026-07-24): per-repo.** `github:re:…@formats=` applies the format list to each matched repo independently, and a repo with no matching asset is the ordinary VIII.2 error, named for that repo.
---

## W9

**Status: ANSWERED — ruled 2026-07-24.**

**W9 — Interpolation outside `when`.** IX.5 says no. Record the boundary explicitly so the
answer is a decision rather than an omission, because the first `link:` request will arrive
quickly. *Recommendation:* stay narrow; reopen only with a use case that cannot be expressed as
two `when` arms.


**RULED (owner, 2026-07-24): no.** No variable interpolation outside `when`. `$role` is tested in a condition, not substituted into a value; the same intent is two `when` arms. Reopen only with a case that cannot be.
---

## W10

**Status: ANSWERED — ruled 2026-07-24.**

**W10 — Variables referencing variables.** `tier = $role-heavy`. Introduces ordering, cycles
(the same walk as `use` loops and `@requires` loops, II.7), and interpolation-inside-a-value,
which collides with W9. *Recommendation:* no, and the cycle machinery already existing is not a
reason to invite the problem.


**RULED (owner, 2026-07-24): no.** Variables do not reference variables (`tier = $role-heavy`). It introduces ordering, cycles and interpolation-inside-a-value (which collides with W9), for a convenience two `when` arms already cover.
---

## K6

**Status: ANSWERED — ruled 2026-07-24.**

**In the tree today:** No group syntax anywhere in `src/`.

**K6 — Does LiNix learn per-backend group syntax** (`@kde-desktop`, `pacman -S plasma`)? It
would make one line install a desktop. It also means `backend:name` has a third meaning on some
backends and not others, which is the kind of unification VIII.1 refused. *Recommendation:* no
for now; a `when family` block listing each distro's name is explicit, works today, and reads.


**RULED (owner, 2026-07-24): no.** LiNix does not learn per-backend group syntax (`@kde-desktop`, `pacman -S plasma`). It would give `backend:name` a third meaning on some backends and not others — the unification VIII.1 refused. A `when family` block naming each distro's package is explicit, works today, and reads. Not building.
---

## K12

**Status: ANSWERED — ruled 2026-07-24.**

**In the tree today:** No symlink handling in `app/locate.rs` or `config/settings.rs`.

**K12 — Is a symlink still supported for "my LiNix files live in my dotfiles repo"?** With X.6's
settings file the symlink is no longer the only answer, but it costs nothing and some users will
reach for it first. *Recommendation:* yes, documented, with the settings file as the primary
mechanism.


**RULED (owner, 2026-07-24): yes, keep the symlink, documented.** A user whose LiNix files live in a dotfiles repo may symlink the config directory. The settings file (`linix path --set`) is the primary, first-class mechanism; the symlink costs nothing and some users reach for it first, so it stays supported and documented.
---

## N4

**Status: ANSWERED — ruled 2026-07-24.**

**N4 — Is `default/incoming` a `firewall:` statement or a preference key?** As a statement it
inherits `when` and the plan; as a key in `preferences.toml` it is machine-local and invisible
to git. *Recommendation:* a statement — the default policy is the most important line in a
firewall and belongs in the repo with the rest.

**RULED (owner, 2026-07-24): both, and the statement wins.** The default policy may be
written as a `firewall:` statement (in the repo, gated by `when`, visible in `plan`) or as a key
in `preferences.toml` (machine-local). Where both say something, **the line wins** — the same
precedence the owner set for N6, and for the same reason: the declaration is the thing you can
read, review and share, and a machine-local key silently overriding it would be the invisible
answer beating the visible one.

---

## N5

**Status: ANSWERED — ruled 2026-07-24.**

**N5 — What does removal restore?** X.4 ruled that a removed `setting:` resets to the schema
default rather than to the value the user had before LiNix. *Recommendation:* the same answer,
for the same reason — restoring a per-rule prior state means keeping a per-rule store of it,
and "undeclared means the firewall's own default" is the shape every other statement's removal
already has. The cost is the same one X.4 recorded and it must be documented, not hidden.

**RULED (owner, 2026-07-24): the firewall's own default, as recommended.** The same answer
X.4 gave for `setting:`, for the same reason — restoring a per-rule prior state means keeping a
per-rule store of it, and "undeclared means the firewall's own default" is the shape every other
removal already has. The cost is documented rather than hidden.

---

## N6

**Status: ANSWERED — ruled 2026-07-24.**

**N6 — What happens when a config declares both `firewall:` lines and a `link:` to the
ruleset file?** *Recommendation:* an error at resolve time naming both files and lines, in the
class of II.7 rule 5. Two owners of one perimeter is the two-of-everything failure, and it
should be caught before any command runs, not discovered at 2am.

**RULED (owner, 2026-07-24): warn, apply both, and the `firewall:` line wins.** The
recommendation was an error at resolve time; the owner's answer is softer and more useful — a
config that declares rules *and* links a ruleset file is doing something legible (a base file
plus specific overrides), so LiNix warns that two things own the perimeter and lets the explicit
declaration take precedence where they disagree. **The warning is not optional**, because two
owners of one perimeter is still the two-of-everything failure; what changed is that it is
reported rather than refused.

---

## N7

**Status: ANSWERED — ruled 2026-07-24.**

**N7 — Does `watch` revert firewall drift unattended, or only report it?** Everything else
`watch` reconciles is software; this reconciles reachability. *Recommendation:* report by
default, revert only under an explicit key, and never revert a rule that would trip N2.

**RULED (owner, 2026-07-24): revert by default, and report instead only when the revert
would close the port carrying the session.** The recommendation had it the other way round. The
owner's answer is the more consistent one: drift is corrected everywhere else in this model, and
a firewall rule nobody declared is drift. The single exception is the one that cannot be undone
from the far end of an SSH connection — there LiNix reports and leaves it, because an
un-reverted rule is a thing you fix tomorrow and a reverted one can be a machine you cannot
reach.

---

## T3

**Status: ANSWERED — ruled 2026-07-24.**

**T3 — What does a missing hardware token look like?** The plugin may prompt on a terminal
nobody is watching. *Recommendation:* a timeout, and a message naming the token and the
identity file rather than passing the plugin's own text through.


**RULED (owner, 2026-07-24): timeout, and a message LiNix owns.** A `@decrypt` whose hardware token is absent times out rather than hanging on the plugin's own prompt, and LiNix names the token and the identity file rather than passing the plugin's text through.
---

## T4

**Status: ANSWERED — ruled 2026-07-24.**

**T4 — May an unattended `watch` tick decrypt?** A touch-required key turns a background
reconcile into a silent block. *Recommendation:* `watch` skips `@decrypt` lines whose identity
is a plugin stub and says so once, rather than hanging.


**RULED (owner, 2026-07-24): skip and say so once.** An unattended `watch` tick skips a `@decrypt` line whose identity is a touch-required plugin stub, and says so a single time, rather than blocking the whole reconcile waiting for a human who is not there.
---

## U2

**Status: ANSWERED — ruled 2026-07-24.**

**U2 — Is a custom backend a full peer of a built-in?** Repos, orphans, dependency queries and
`is_essential` are `ManagerConfig` fields `CustomBackendDef` does not expose.
*Recommendation:* expose them as optional keys, absent meaning *this backend cannot answer
that* — the `ManualListing` distinction already made for exactly this reason: "not configured"
must not be read as "the answer is none".


**RULED (owner, 2026-07-24): first-class.** A custom backend is a full peer of a built-in. The fields a built-in has and `CustomBackendDef` did not — repositories, orphan listing, dependency queries, OS-essential — are exposed as optional keys, absent meaning *this backend cannot answer that*, never *the answer is none* (the `ManualListing` distinction, generalised). This is the onboarder becoming a true equal, which is the whole 'it can drive anything' thesis.
---

## U4

**Status: ANSWERED — ruled 2026-07-24.**

**U4 — Is `exec:` a licence to put a shell script where a backend belongs?** The onboarder is
the better answer for anything that installs software, and `exec:` should not become the way
people avoid writing eight lines of TOML. *Recommendation:* document the boundary in the
readme, and treat repeated `exec:` lines that install things as a sign the onboarder needs a
missing field (U2), not as usage to encourage.


**RULED (owner, 2026-07-24): document the boundary.** `exec:` is for actions with no inverse, not for installing software — an `exec:` that installs is a one-way door (deleting the line does not undo it). The onboarder is the answer for anything installable: it gives a noun, which removes/lists/locks. The README's `exec:` section now says so and links the onboarder.

**AMENDED by U33 (owner, 2026-07-26): the categorical refusal is lifted.** `exec:` may install
software and do anything, gated behind an explicit config key (off by default). The boundary this
entry drew is now a *recommendation*, not a prohibition: the onboarder is still the **better** path
for anything installable (a noun that removes/lists/locks), and the README says so, but `exec:` is
no longer forbidden from installing when the key is on. Removal is still governed by U3 — an
installing `exec:` has no automatic inverse without `@undo=`.
---

## U6

**Status: ANSWERED — ruled 2026-07-24.**

**U6 — Does this document mark its Linux-only guarantees?** The pre-sync snapshot, `rebuild`'s
revert and `rollback`'s safety net all assume a provider that exists only on Linux
filesystems. *Recommendation:* yes, immediately and independently of whether VSS or APFS is
ever adapted — an unqualified promise that silently does not hold on two of three platforms is
P3's failure in prose form.


**RULED (owner, 2026-07-24): yes.** The Linux-only guarantees — the pre-sync snapshot, `rebuild`'s revert, `rollback`'s safety net — are marked as such in the docs, independently of whether VSS or APFS is ever adapted. An unqualified promise that silently does not hold on two of three platforms is P3's silent-wrongness in prose.
---

## U7

**Status: ANSWERED — ruled 2026-07-24.**

**U7 — Is a health check per-package or per-sync?** Per-package answers "did *this* upgrade
break it" and is precise; per-sync catches the breakage a package cannot see (the boot, the
network). *Recommendation:* both, and they are not alternatives — `@health=` on a line, plus a
`health` list in `preferences.toml` for the machine-wide checks, with the same revert path.

**RULED (owner, 2026-07-24): both, as recommended.** `@health=` on a line answers *did this
upgrade break this*, and a machine-wide `health` list in `preferences.toml` catches what a
package cannot see — the boot, the network, the thing two packages away. They are not
alternatives and share one revert path.

**BUILT, 2026-07-24 (7f).** `@health=` is a package option key and a `health = [...]` list is a
`preferences.toml` key. Both are collected in one place and share one revert path. A declared
check with no snapshot provider refuses **before** the change (V.65). `@check=`, an unreachable
branch reading an option key the grammar never accepted, was deleted in the same commit.

---

## U8

**Status: ANSWERED — ruled 2026-07-24.**

**U8 — Is the removal preview a flag or a verb?** *Recommendation:* a flag on the commands that
already compute it. A new verb for an existing computation is how this repo got two of
everything.


**RULED (owner, 2026-07-24): a flag, not a verb.** The removal preview already exists as `check drift` and `--dry-run`; the decision is not to add back an `orphans`/`prune` verb. The stale "prune would remove" message was corrected to name `sync`.
---

## U10

**Status: ANSWERED — ruled 2026-07-24, and neither option was taken.**

**U10 — Where does a backend's bootstrap live?** In `priority`, beside the backend it obtains,
or in `custom_backends.toml`, beside the definition. *Recommendation:* `priority` — it is the
file that already decides which backends this machine uses, and a custom backend's definition is
about *how to drive* a manager, not *how to get* one. The two files stay one-question-each.

**RULED (owner, 2026-07-24): a third file — and the other two move to join it.** *"It should be
a separate file, all 3 should be in the shareable config part, and all should be in the same
folder."* The recommendation's own reasoning (one question per file) was right and was applied
one step further than it had been: **how to get a manager** is a third question, so it is a third
file, and the three sit together because they are one subject — what you have taught this LiNix.

```
adapters/backends.toml    how to drive a package manager LiNix does not ship   (XIII.2)
adapters/settings.toml    how to read and write a settings store               (K17)
adapters/bootstrap.toml   how to obtain a manager this machine does not have   (7c)
```

- **In the config repo**, so a definition travels with the configuration that needs it — the
  point 7a/U1 established, now applying to all three.
- **Each file is approved separately** through II.12's ledger (`adapters:<filename>`), because
  they carry different argv: approving the backends you added is not a review of the settings
  adapters. One identity per *file*, not per definition, so an edit that **adds** a definition
  still invalidates the approval.
- **The K17 arrangement is superseded**: settings adapters shared `custom_backends.toml` because
  at the time that was where repo-supplied definitions lived. They have their own file now.
- **The folder name is `adapters/`** — an implementing call, not a ruling: it is the word the
  spec already uses for settings stores (K17), and a backend definition adapts a CLI the same
  way. Bootstrap sits with them because it answers a question about the same subject.
- **NO LEGACY:** the old `custom_backends.toml` path is deleted, not read as a fallback.

---

## U11

**Status: ANSWERED — ruled 2026-07-24, and generalised past the question that was asked.**

**U11 — Does `watch` imply `--locked`?** An unattended reconcile that silently accepts a new
upstream version is the least supervised place for a version to change. *Recommendation:* yes by
default, overridable by a key — a machine reconciling itself at 3am should be converging to what
was decided, not to what was published.

**RULED (owner, 2026-07-24): it is not a `watch` question. `sync` itself defaults to the
recorded version, with an explicit `--upgrade` to move forward — and `watch`, being `sync` with
nobody watching, inherits that rather than being special-cased.** The owner's words: *"it should
be the same as sync, which if sync does not do this, it needs fixing."*

**It did need fixing, and this was a live defect.** `sync` defaulted to `locked: false` and
`watch` hard-coded it, so `locks/versions.json` was read *only* under `sync --locked`. A machine
rebuilt from a config therefore installed whatever upstream had published that morning, not the
version the lock recorded — which is the reproducibility claim the lock exists to make.

**Three modes now, and the middle one is new:**

| | a recorded version | nothing recorded | a pin that disagrees |
|---|---|---|---|
| **default** | wins | resolves freely | **the line wins** |
| `--upgrade` | ignored | resolves freely | the line wins |
| `--locked` | wins | **error** | **error** |

- **Nothing recorded is not an error by default.** That is the ordinary state of a machine that
  has never run `linix lock`, and making it fatal would mean no config works until it is locked.
  Strict `--locked` keeps that refusal, because there a missing entry is a gap in the
  reproduction rather than a detail.
- **A hand-written `@version=` beats the lock outside strict mode.** A version you typed is a
  decision; the lock is a record of one. Under `--locked` the same disagreement is an error,
  because a reproduction that silently picks one of two answers has reproduced neither.
- **`linix lock` stays the deliberate act** that records versions, exactly as it is the
  deliberate act that approves a hook or an `exec:` script.

---

## U12

**Status: ANSWERED — ruled 2026-07-24.**

**U12 — Does `try` reuse the Phase 6 images, or build from a base the config names?** Reusing
them is nearly free and covers debian/alpine/arch today; a config-named base is what a user with
an unusual host actually needs. *Recommendation:* start with the Phase 6 images, and treat a
config-named base as the second step rather than the blocker — the value is in the rehearsal
existing at all.

**RULED (owner, 2026-07-24): reuse the Phase 6 images to start.** debian/alpine/arch are
already built and cover most hosts; a config-named base is the second step, not the blocker. The
value is the rehearsal existing at all.

---

## U13

**Status: ANSWERED — ruled 2026-07-24.**

**U13 — Does `@runs=always` exist?** It is the escape hatch inside the escape hatch, and every
such key eventually becomes the default somebody copies. *Recommendation:* yes, but it prints
what it is doing on every sync — a line that runs unconditionally must be visible in the run it
made non-idempotent, or the next person debugging a slow sync has no thread to pull.


**RULED (owner, 2026-07-24): yes.** `@runs=always` exists and prints a line naming itself on
every sync (`runs=always — every sync`), so a non-idempotent line is visible in the run it made
non-idempotent. Once is the default; `@runs=N` runs a set number of times (already built as the
ceiling). A count may also be expressed by gating `@runs=always` with a `when` — the owner's
preferred spelling — which the existing `when` machinery already supports.
---

## U15

**Status: ANSWERED — ruled 2026-07-24.**

**U15 — Where do LiNix-level event hooks live, and are they per-machine?** `preferences.toml` is
machine-local, so `after_sync` on the laptop is invisible to the desktop. That is right for a
notification hook and wrong for a policy one. *Recommendation:* `preferences.toml` first —
machine-local behaviour is the honest default for something that talks to *this* machine's
Slack — and revisit only when a real case wants a fleet-wide event.

**RULED (owner, 2026-07-24): both locations, not one.** A hook may live in
`preferences.toml` (machine-local — the notification that talks to *this* machine's Slack) or in
the config repo (the policy every machine should run). The recommendation offered only the first;
the owner's answer is that the choice belongs to the user, because the two kinds of hook are
genuinely different and forcing them into one file makes one of them wrong.

**They are additive, not overriding.** A repo hook and a machine hook for the same event both
run — the repo's because every machine should, this machine's because it is this machine. A
precedence rule would mean adding a local notification silently disables the shared policy, which
is the quiet failure this model exists to avoid.

**BUILT, 2026-07-24 (7j).** Both locations: `hooks/<event>` in the config repo and
`[hooks.<event>]` in `preferences.toml`. Both fire, repo first, with separate ledger identities
so approving the shared policy never rubber-stamps the local file. Events are `after_sync`,
`on_drift`, `on_guard_refusal`; a failing hook warns and does not fail the sync.

---

## U16

**Status: ANSWERED — ruled 2026-07-24.**

**Still open, and now reachable — 2026-07-23.** `binary` exists (7a), and a path in it is
**refused** with a message saying why: this is the status quo preserved, not an answer. Allowing
it later is additive; allowing it now would decide the question in code.

**U16 — Does the field split (XIII.12) allow an absolute path as `binary`?** A prefix that runs
`/opt/vendor/thing` is more useful and is also a definition that only works on one machine.
*Recommendation:* allow it, resolve `~`, and have `doctor` report a custom backend whose binary
is missing — the failure should be a named diagnosis, not an unknown-backend error three layers
away.


**RULED (owner, 2026-07-24): yes.** A custom backend's `binary` may be an absolute path; a
leading `~` is expanded. A definition naming a path that is not on this machine is not refused
at load — it is a named diagnosis in `check health` ("`/opt/vendor/thing` does not exist or is
not executable"), where the fix is obvious. Whitespace and emptiness are still refused, being a
malformed value rather than a path.
---

## U17

**Status: ANSWERED — ruled 2026-07-24.**

**U17 — Is `linix eval`'s output versioned from the first release?** *Recommendation:* yes, a
top-level schema version, decided before anything consumes it. P2 says there is no legacy to
carry, and this is the one output that will acquire consumers LiNix cannot see.

**RULED (owner, 2026-07-24): yes.** `linix eval` carries a top-level schema version from its
first release. It is the one output that will acquire consumers LiNix cannot see, and P2 leaves
no legacy to carry — so the version is free now and impossible later.

**BUILT, 2026-07-24 (7k).** `linix eval` prints the resolved state as JSON with a top-level
`schema`. It takes no lock and touches no backend. Sources are repo-relative with forward
slashes so two machines' evaluations diff cleanly.

---

## U18

**Status: ANSWERED — ruled 2026-07-24.**

**U18 — Are grouped backends with per-group priority worth building at all?** The workaround —
write the prefix — already works, and what it costs is the portability a bare name exists for.
*Recommendation:* build it only with the invariant attached: **a bare name still resolves once
per machine**, and two modules that would resolve the same name through different groups is an
error naming both, which is II.7 rule 5 reached by a new road rather than a new rule. Without
that, this feature ships two `ripgrep` binaries fighting over `$PATH` — the failure
`app/conflicts.rs` already exists to catch.


**RULED (owner, 2026-07-24): build it — it is only a shortcut.** A group is a NAME for a backend chain, so instead of `apt,dnf,cargo:ripgrep` on every line you define `tools = apt, dnf, cargo` in a `groups` file and write `tools:ripgrep`. It expands to exactly that chain in the one parser (V), inheriting the chain's meaning and safety with nothing added — `priority` still exists, a bare name still resolves through it. **Groups nest** (owner): a member may be another group, flattened to terminal backends at load, and a cycle is refused like a `use` loop. This is NOT the per-module-priority design the recommendation feared — that footgun does not apply to a chain alias. BUILT the same day: `src/model/groups.rs`, `Vocab::with_groups`, grammar expansion, verified on the binary (`all = cargo, winmgrs` / `winmgrs = scoop, winget` → `all:rg` resolves).
---

## U20

**Status: ANSWERED — ruled 2026-07-24 (build only if thin AND easy; deferred).**

**U20 — Is a language server wanted, and is it allowed to be a second implementation?** *This is
the whole question, not the feature.* *Recommendation:* wanted, but only as a thin front end
over the same parser and resolver the binary uses — the moment it re-implements the grammar it
becomes the second implementation this rewrite exists to end, and it will disagree with the
first within a release. If it cannot be thin, do not build it.


**RULED (owner, 2026-07-24): yes, but only if very easy — and it is not, yet.** A language server is a stdio JSON-RPC protocol server (document sync, diagnostic ranges, the LSP handshake); even diagnostics-only is a few hundred lines and a protocol, which is not "very easy" and not worth a half-implementation. **Deferred.** The editor-diagnostic hook it would provide already exists in a thinner form: `linix check config` prints `file:line: message` from the same parser the binary uses, which efm-langserver / null-ls / ALE consume directly. Its one limit is that it stops at the first error rather than collecting all — the natural first step if this is ever picked up, and cheaper than an LSP.
---

## U21

**Status: ANSWERED — ruled 2026-07-24.**

**In the tree today:** No exit-code table; `main.rs:33` is the only `process::exit` and it is `0`.

**U21 — Is the exit-code table settled once, up front?** *Recommendation:* yes — 0 converged, 1
LiNix failed, 2 differences found, 3 refused by the guard — decided in one place before
`--locked`, `try` and `check` are written. An exit code decided per command is a convention no
script can rely on, and the separation that matters is 3: a guard refusal is neither a failure
nor a difference.

**RULED (owner, 2026-07-24): yes — 0 converged, 1 LiNix failed, 2 differences found, 3 refused
by the guard.** Decided in one place before the commands that use it. **BUILT the same day**
(`core::exit`), and it exposed a real gap: the guard refused through `Error::Other`, so a refusal
was indistinguishable from a crash and no script could avoid retrying one. It has its own
`Error::Refused` now, `check` returns `Error::Differences`, and one mapping point in `main`
turns both into codes. Verified on the binary: findings → 2, clean → 0, bad argument → 1.

---

## U25

**Status: ANSWERED — ruled 2026-07-24.**

**U25 — One tree or several?** Several (`dotfiles:./dotfiles-work` under a `when`) composes
with the model already and costs nothing; one is simpler to explain. *Recommendation:* several,
because the statement takes a path anyway and forbidding a second one would be a rule with no
mechanism behind it — but two trees that would link the same destination is an error naming
both, which is II.7 rule 5 reached by a new road rather than a new rule.

**RULED (owner, 2026-07-24): several, as recommended.** The statement takes a path, so
forbidding a second one would be a rule with no mechanism behind it. Two trees that would link
the same destination is an error naming both — II.7 rule 5 reached by a new road, not a new
rule.

---

# Answered earlier — the first three rounds

## T6

**Status: ANSWERED — fully ruled (blocking half 2026-07-23; both sub-questions 2026-07-26).**

**In the tree today:** `backup_once` (`link.rs:172`) has **no opt-out and no bound of any kind
beyond one-per-target.** It never clobbers an existing backup, so a target accumulates exactly
one — and `remove` (`link.rs:369`) does not delete or restore it, so that one is permanent.

**T6 — There must be a way to opt out of the backup, or to limit how many accumulate (owner
request, 2026-07-23).** Raised while ruling T1, and **it is not a secrets question** — every
`link:` managed-content write calls `backup_once`, so this governs ordinary config files too.
Four things need answering and they are not the same question:

1. **The opt-out's shape.** A per-line `@backup=no` says it where the exception is, at the cost
   of an option key on every `link:` line. A `preferences.toml` key says it once for the machine
   and cannot express *"this one file, not the others"*. Both is two mechanisms for one question.
2. **What "limit amounts" means, given it is already one per target.** The accumulation is across
   *targets*, not within one — forty linked files means up to forty orphaned backups. So the
   candidates are an age (delete a backup older than N days), a command that lists and clears
   them, or a rule tying the backup's life to the declaration's.
3. **Does removing the `link:` line remove the backup, restore it, or leave it?** Today: leave
   it, and that is almost certainly wrong. **Restoring it is the shape every other extra
   already has** — `extras_lock` undoes what a declaration did — and it is the answer that makes
   the backup a rollback rather than a leak.
4. **Is there a command to see them at all?** They are invisible to `check` because they are not
   managed, which means the one thing standing between a user and forty stale plaintexts is
   remembering the file-naming convention.

*Recommendation:* per-line `@backup=no` **and** removal restoring the backup (3), which together
answer 1 and 2 without a retention policy: a backup that is put back when the declaration goes
does not accumulate, and the line that wants no backup says so. A `linix` command to list orphaned
backups then covers the case where the user deleted the line before this existed.

**RULED (owner, 2026-07-23): removing the declaration restores the backup.** Sub-question 3 is
answered, and it answers 2 with it: a backup that is **put back** when the line goes cannot
accumulate, so no retention policy, no age, and no cleanup command are needed for the ordinary
case. `remove` (`link.rs:369`) currently drops the target and orphans `<target>.linix-backup`
forever; it will instead restore the original and delete the backup, which is the shape
`extras_lock` already has for every other extra — **a declaration undoes what it did.**

**This shrinks T1.** Decrypt mode still never backs up, but the reason is now narrower and the
fix smaller: without restore-on-removal a suppressed backup would have been a special case, and
with it the general path is already safe.

**Still owed, and deliberately not ruled here:** sub-question 1, the opt-out's spelling
(`@backup=no` on the line, a machine-wide key, or both), and sub-question 4, whether any command
lists backups orphaned by the versions of LiNix that shipped before this ruling. Both are
smaller once restore-on-removal exists, and neither blocks it.

**RULED (owner, 2026-07-26): both sub-questions closed. T6 is fully answered.**

- **Sub-question 1 — the opt-out is per-line `@backup=no`.** It states the exception exactly where
  the exception is (this file, not the machine), and restore-on-removal already killed the pile-up
  a machine-wide key would have been for — so a single mechanism, not two, and no "which one wins"
  question. A machine-wide key is not added.
- **Sub-question 4 — nothing is owed.** NO LEGACY: LiNix has no real users, so there are no
  pre-ruling orphaned backups in the wild to sweep, and no cleanup command is built. If stray-
  backup *visibility* is ever wanted it belongs as a line in `check`, never a new verb (U8) — but
  it is not built now, because there is nothing to see.

---

## N1

**Status: ANSWERED — ruled 2026-07-23.**

**In the tree today:** Nothing. No firewall code exists in `src/`.

**N1 — Is the declared perimeter exclusive?** *This is the whole feature.* Additive means the
lines say "these rules exist" and anything else a human added survives. Exclusive means they say
"these rules and no others", and an undeclared rule is drift to be removed — which is what
"instantly detecting and purging any unauthorised out-of-band changes" asks for, and is the only
version that makes the perimeter a fact rather than a floor. It is also the version that deletes
the rule someone added for a reason nobody wrote down. *Recommendation:* exclusive, because an
additive firewall answers no question worth asking, **but only with N2 answered and only behind
`purge-unmanaged`'s existing opt-in shape** (II.11) rather than on by default in `sync`.

**RULED (owner, 2026-07-23): the firewall does not get its own answer. `sync` is additive and
`purge-unmanaged` is exclusive, as always.**

The question was framed as a choice about firewalls and it is not one — **it is the model's
existing split, applied to a new backend**, and the right answer to *"is my declaration
exclusive?"* is the same for every backend that ever asks it. The three cases, spelled out
because the framing hid that they were already decided:

| the rule | who made it | `sync` | `purge-unmanaged` |
|---|---|---|---|
| declared, and present | you, in a file | left alone | left alone |
| declared once, now undeclared | LiNix, and the declaration is gone | **removed** — it is in the extras ledger | removed |
| never declared, added out of band | a human at 2am | **left alone** | **removed** |

**This deletes the special shape the recommendation proposed.** There is no "exclusive mode
behind an opt-in": `purge-unmanaged` *is* the opt-in, it already exists, and inventing a
firewall-shaped version of it would have been a second implementation of the one question this
model answers once.

**Recorded in II.11, with its reason in V.63**, because it is a general rule about the two
commands and not a fact about firewalls — and because the question could only be asked in the
first place by someone who could not find it written down.

**It also narrows N7.** "Does `watch` revert firewall drift" no longer means "does it purge
rules nobody declared" — it cannot, that is `purge-unmanaged`'s job now. It means only: when a
rule **LiNix owns** is changed out of band, does an unattended tick put it back? That is a
smaller question and a sharper one, because putting a rule back can close a port somebody opened
at 2am to fix something, with nobody there to read about it (N2).

---

## N2

**Status: ANSWERED — ruled 2026-07-23.**

**In the tree today:** Nothing. No firewall code exists in `src/`.

**N2 — What does LiNix do when the change would close the session it is running over?** A
confirmation prompt cannot work: the prompt travels over the connection the change severs.
*Recommendation:* refuse. Detect the port of the controlling connection, and refuse any plan
that would deny it, naming the port and the rule — overridable only by a flag that says the
user has console access. Building this feature without this check is building the lockout.

**RULED (owner, 2026-07-23): refuse, and detect the port rather than asking.** A confirmation
cannot work — the prompt travels over the connection the change severs. LiNix detects the port
carrying the controlling connection and refuses any plan that would deny it, naming the port and
the rule that would close it. The only override is a flag asserting console access.

**This check binds every path that can close a port, not just `sync`.** N1's ruling means
`purge-unmanaged` can close one, and a `watch` tick reconciling a rule LiNix owns can close one
while nobody is watching — **which is the more dangerous of the two, because nobody is there to
read the refusal.** A check on one command is a check on nothing; this is II.10's rule about the
guard, reached by a new road.

---

## N3

**Status: ANSWERED — ruled 2026-07-23.**

**In the tree today:** Nothing. No firewall code exists in `src/`.

**N3 — Which adapters ship, and does one adapter justify the backend?** XI.2 says the backend
earns its place across firewalls and not within one. *Recommendation:* it is not worth starting
below two adapters plus Windows; if only `ufw` is in reach, document the `link:` pair and close
this part.

**RULED (owner, 2026-07-23): build it — and the reason the answer changed is K17.**

The entry's own position was that below two adapters plus Windows the honest recommendation is
to build nothing and document the `link:`+`service:` pair instead. **That argument was entirely
about cost per adapter, and K17 changed the cost.** Adapters are a declarable table with the
built-ins as rows in it, so five firewalls are five rows rather than five Rust backends, and
XIII.12's field split already showed `firewall:22/tcp` working from six lines of TOML.

**Windows Defender Firewall is in the first set**, not a later platform phase — P7, and the
owner's daily machine. A Linux adapter (`ufw` or `firewalld`) is the other.

**What does not change is XI.2's honesty about the alternative.** The `link:`+`service:` pair
still works and is still the right answer for someone with one machine and one firewall; what
the backend buys is one spelling across several, per-rule drift instead of per-file, and
read-before-write.

---

## T1

**Status: ANSWERED — ruled 2026-07-23.**

**In the tree today:** **Still live.** `link.rs:319` calls `backup_once` on the managed-content write path that mode D uses; `:172` is the copy. The 0600 at `:285` is applied to the target only.

**T1 — `backup_once` copies the previous secret to a world-readable file.** `link.rs:319` and
`:154` run for every managed-content write, including mode D: if the target already holds a
secret, LiNix copies it to `<target>.linix-backup` before overwriting — with default umask
permissions, and with no `.linix-backup` in any ignore file. The 0600 at `:285` is applied to the
target only. *Recommendation:* mode D never backs up. The point of `backup_once` is that a user
is not silently robbed of a config file they hand-wrote; a secret LiNix itself wrote a moment ago
is not that, and the backup is a plaintext credential in a predictable path nobody will think to
delete. **This is a defect in shipped code, not a design question — but it is recorded here
rather than fixed silently, per rule 4.**

**CORRECTED 2026-07-23, before the ruling — two of the three facts above are false, and the real
defect is worse than the one recorded.** Read from the code rather than from the sentence:

- **The backup is not written under the default umask.** `link.rs:203` uses `tokio::fs::copy`,
  which copies the source file's permission bits. A `0600` original produces a `0600` backup.
- **`.linix-backup` is not absent from every ignore file.** `core/git.rs:169` writes
  `*.linix-backup` into the config repo's `.gitignore` at `linix git init`. It only covers
  backups that land *inside* the repo, which is T2's case, but the claim as written is wrong.
- **What is actually true, and was not recorded: nothing ever removes the backup.** `remove`
  (`link.rs:369`) deletes the target and leaves `<target>.linix-backup` untouched, and
  `backup_once` refuses to clobber an existing one. So a decrypted credential's predecessor
  **survives the declaration being deleted, and survives forever.** No command lists them, no
  command cleans them, and the file is invisible to `check` because it is not managed.

**RULED (owner, 2026-07-23): decrypt mode never backs up.** The point of `backup_once` is that a
user is not silently robbed of a config file they hand-wrote. A secret is not that, and a
plaintext credential in a predictable path that nothing will ever delete is a worse outcome than
the one the backup exists to prevent.

---

## T2

**Status: ANSWERED — ruled 2026-07-23.**

**In the tree today:** **Still live.** Nothing in `link.rs` compares the resolved `@target=` against the config root.

**T2 — Nothing stops `@target=` from pointing back into the config repo.** A
`link:./secrets/token.age@target=./secrets/token@decrypt=age` writes the plaintext next to the
ciphertext, inside git, and the next `sync` commits it. *Recommendation:* refuse a `@target=`
that resolves inside the config root when `@decrypt` is set — the check is cheap, the failure is
unrecoverable (a secret in git history is a rotated secret), and X.5's promise that a backup is
safe to hand to someone depends on it holding.

**RULED (owner, 2026-07-23): refuse a `@target=` that resolves inside the config root when
`@decrypt` is set.** The check is cheap and the failure it prevents is unrecoverable — a secret
in git history is a rotated secret. X.5's promise that a `bundle` is safe to hand to someone
depends on this holding, and `core/git.rs:169`'s `*.linix-backup` ignore line does not cover it:
the plaintext target is not named `.linix-backup`.

---

## T5

**Status: ANSWERED — ruled 2026-07-23.**

**T5 — Is the plaintext 0600 at creation, or after?** Today `write_atomic` creates under the
umask and `set_permissions` follows (`link.rs:285-292`). The window is small and local, and on
Windows there is no restriction at all. *Recommendation:* create restricted rather than
chmod after, and on Windows either set an ACL or say plainly in the docs that mode D gives the
file no special protection there — the second is acceptable, silence is not.

**RULED (owner, 2026-07-23): create restricted, and Windows gets a real answer rather than
silence.** The plaintext is created with its final permissions rather than created under the
umask and chmod'd afterwards.

**On Windows the file gets an ACL or the documentation says plainly that it does not.** Silence
is not acceptable — this is the owner's daily platform, and an unqualified *"the plaintext is
0600"* that holds on one of three platforms is P3's failure written as prose.

---

## K17

**Status: ANSWERED — ruled 2026-07-23.**

**In the tree today:** `backends/setting.rs` has a closed `enum SettingStore` with two variants,
`GSettings` and `None`. Adding a store means adding a variant, which means shipping a release.

**K17 — How does `setting:` reach a store nobody has written an adapter for?** Raised by K7's
2026-07-23 ruling, which says *everywhere* rather than naming a closed set. Every adapter is the
same three operations — read a key, write a key, reset a key to its default — and for most stores
each is one command with the key interpolated into it. That is exactly the shape
`custom_backends.toml` already describes for package managers (XIII.2, XIII.12): argv from a
table, output read by a declared parser.

- **A closed enum, grown per release.** Simplest, and it is today's code. It means the machine
  running the store LiNix has not heard of gets a refusal until a LiNix release reaches it, which
  is the machine most likely to be running something unusual in the first place.
- **A declarable adapter, the way custom backends already are.** Three commands and a value
  encoding in a table, so a COSMIC or a Hyprland or a thing not invented yet is six lines rather
  than a pull request. It costs what U1 costs — a definition that a shared repo can execute is
  II.12's supply-chain surface and must inherit the hook trust model, not a new one.
- **Both, with the built-ins as data too**, so there is one code path rather than a fast one and
  a slow one. **Two of everything is how this repo got into trouble**, and an enum plus a table
  is exactly two.

*Recommendation:* the third. The built-in adapters become rows in the same table the user can
add a row to, `setting:` reads that table and nothing else, and the refusal for an unadapted
store stays exactly as it is. **Decide before the registry adapter (7e) is written** — it is the
second adapter, and the second one is where the shape is set.

**RULED (owner, 2026-07-23): a lot of stores, and adding one is a plugin, not a release.** The
third option — the built-in adapters become rows in the same table a user can add a row to,
`setting:` reads that table and nothing else. **One code path, not a compiled fast one and a
declared slow one**, because an enum plus a table is two of everything with a new name.

- **`gsettings` stops being special.** It becomes a row like the rest, which is the only way the
  built-ins stay honest: an adapter mechanism that the built-ins bypass is a mechanism nobody has
  actually tested.
- **The refusal survives.** A store with no row makes every `setting:` line an error naming it.
  That is what lets adapters land one at a time and what keeps a key from being silently
  unapplied.
- **It inherits the hook trust model, not a new one.** An adapter definition is argv that a
  shared config repo can execute, which is II.12's supply-chain surface — the same consequence
  U1 carries for custom backends, and it must be answered the same way rather than twice.

**BUILT 2026-07-23, as ruled.** `enum SettingStore` is deleted. An adapter is a
`[[setting_store]]` row — `name`, `detect` (the command whose presence means the machine runs
this store), optional `os`, and the `read`/`write`/`reset` argv with `{schema}`, `{key}` and
`{value}` substituted. `gsettings` is a row in `src/backends/setting_stores.toml`, **parsed by
the same loader a user's row goes through**, so the shipped adapter cannot drift into a
privileged path nobody has tested.

**The trust answer is literally the same one, not the same shape.** User rows live in the config
repo's `custom_backends.toml` — the file 7a moved and put under the hook ledger — and both
readers go through one `read_approved_definitions`. One file, one approval, one refusal message,
and no way to add a third kind of definition that quietly skips the check. The alternative
(`setting_stores.toml` as its own file) would have been a second loader and a second ledger
entry for the identical question.

**A row that cannot be read is refused rather than half-used.** X.4's read-before-write is what
makes `setting:` a declaration instead of a command that runs every sync, so an adapter with no
`read` is not a slow adapter — it is not an adapter. Same for a missing `reset`: removing the
declaration would silently do nothing.

**The refusal now names what LiNix looked for**, so the machine running the unlisted store learns
what to write a row about rather than only that it failed.

---

---

## D2

**Status: ANSWERED — ruled 2026-07-23.**

**In the tree today:** `backends/artifact/select.rs:260` (`classify_format`) and `platform.rs:96` (`classify`). The entry's own caveat — *"needs testing against real releases before it is a rule"* — is still unmet.

**D2 — How is a format recognised from an asset filename?** There is no metadata on a GitHub
asset saying "this is a tarball" — only a filename, and release naming is a free-for-all
(`fd-v10.2.0-x86_64-unknown-linux-gnu.tar.gz`, `fd_10.2.0_amd64.deb`, `fd-linux`, `fd`). Pure
extension matching fails on `binary`, which has no extension by definition. *Recommendation:*
extension match for everything that has one, and `binary` means "matched this machine's os/arch
and has no recognised extension" — but this needs testing against real releases before it is a
rule, because it is the one part of this feature that fails quietly rather than loudly.

**RULED (owner, 2026-07-23): confirmed as the rule, and the testing the entry asked for is now
work rather than an assumption.** An extension decides the format; a name with no recognised
extension that matches this machine's os/arch is `binary`.

**The caveat is the reason the work is filed, not a reservation about the rule.** A wrong
*extension* guess produces an error. A wrong *`binary`* guess installs the wrong file and says
nothing — the one place in this feature that fails quietly rather than loudly. The entry said it
needed checking against real releases before becoming a rule; that never happened, and it is now
in the plan.

**CHECKED 2026-07-23, and the check found two live defects.** The asset lists of six real
releases (fd, jq, gh, neovim, rclone, helm) were fetched and every answer verified by hand, on
three platforms. The fixture is `src/backends/artifact/real_releases.txt` and the answers are
asserted, so this is a check that can fail rather than an inspection that happened once.

- **`accepts` is not "matched".** The code read the rule as *does not contradict this machine*,
  and the ruling says *matched*. Under the weak reading, `MD5SUMS` — a real asset of every
  rclone release, no extension, naming nothing — was an executable candidate on every platform,
  and so was anything else extension-less that a release happens to attach. **A `binary` now
  requires the filename to name this machine's os or arch**, which is what the ruling says.
  `@asset=` naming the file exactly overrides it, because naming it *is* the claim — otherwise
  a project shipping one bare `mytool` would become uninstallable.
- **`linux64` named no operating system.** The token matcher required a non-alphanumeric after
  an alias, so `linux` inside `jq-linux64` — a real asset of jq's release — did not match, and
  the file read as running anywhere. On Windows it was an executable candidate. A closing run of
  digits is part of the boundary now (`linux64`, `win64`, `mac64`), while the leading boundary
  is unchanged so `386` still does not match inside `i386`.

**The one thing left as a question was ruled 2026-07-24 and is now fixed.** On jq and rclone the
selector chose the project's **source tarball** over a binary naming the exact machine, because
the tie-break ranked format order above specificity even when that order was *detected* rather
than asked for. **Owner ruled: a detected order yields to the machine; a `@formats=` the user
wrote still wins outright.** The tie-break now leads with specificity when
`FormatOrder::is_user_specified()` is false and with format rank when it is true; jq resolves to
`jq-linux-amd64`, and a user who writes `@formats=tarball` still gets the tarball. The macOS
default order also gained `zip` in the same change — gh, rclone and starship ship their macOS
build as one and resolved to nothing without it. Both are covered by the real-release fixture,
whose expectations are now the file a human would pick on every row.

---

## K5

**Status: ANSWERED — ruled 2026-07-23.**

**In the tree today:** Built as recommended.

**K5 — May a level-3 reset (X.3) run while a config repo exists?** Forgetting the registry
while the declarations remain leaves LiNix believing it manages nothing and the files saying
otherwise. *Recommendation:* refuse unless the repo is empty or `--force`, and say which.
**BUILT the recommendation, 2026-07-20:** `linix reset` refuses when `modules/`, `profiles/`
or `active` exists unless `--force`, and the refusal names the repo path and both ways forward.

**RULED (owner, 2026-07-23): confirmed as built.** `linix reset` refuses while a config repo
exists unless `--force`, and the refusal names the repo and both ways forward. Forgetting the
registry while the declarations remain leaves LiNix believing it manages nothing and the files
saying otherwise, and there is no reading of that state that is not a trap.

---

## K11

**Status: ANSWERED — ruled 2026-07-23.**

**In the tree today:** `config/settings.rs:17` — `const ONLY_KEY: &str = "config_root"`, enforced by the parser.

**K11 — May LiNix's settings file (X.6) hold anything besides the repo path?** *Recommendation:*
no, and the refusal should be enforced by the parser, not by discipline. **A file holding exactly
one key is the file that grows a second one** — and the moment it does, there are two preference
systems (it and `preferences.toml`) and a new question about which wins on every key either
could hold. The one key it holds is the one key `preferences.toml` structurally cannot.

**RULED (owner, 2026-07-23): confirmed as built.** One key, enforced by the parser rather than
by discipline. A second key would make two preference systems, and every key either file could
hold would raise a new question about which one wins.

---

## K14

**Status: ANSWERED — ruled 2026-07-23.**

**In the tree today:** `handle_rebuild` never reaches `git_autocommit`. **No test asserts it**, as the entry says.

**K14 — Does `rebuild` produce a git commit?** Nothing about the declared state changed, so
there is nothing to commit — but a history that does not record a rebuild means `git log` is no
longer a complete account of what happened to the machine (II.4's claim). *Recommendation:* no
commit; `rebuild` is recorded wherever snapshots are, not wherever intent is.

**The recommendation holds and is what the code does** — `handle_rebuild` never calls
`perform_maintenance`, which is the only path to `git_autocommit`. **It is still not asserted by
a test** (2026-07-21): the honest one needs a backend that can really remove and reinstall, and
a test that only greps the source would pass on a rebuild that committed through some other
route. Recorded rather than faked.

**RULED (owner, 2026-07-23): confirmed as built.** `rebuild` writes no git commit. Nothing about
the declared state changed, so there is nothing to record as intent; a rebuild is recorded
wherever snapshots are.

**The test stays owed and is filed.** A test that greps the source would pass on a rebuild that
committed through some other route, so the honest one needs a backend that really removes and
reinstalls.

**Checked 2026-07-23: the test exists and has never been run.** `docker/integration/run-in-container.sh`
section 12 does exactly what this entry asks — a real package removed and reinstalled, and git
asked directly for its commit count rather than `linix git log`. It cannot run here: there is no
container runtime on the development machine, and the harness installs and removes real system
packages, so pointing it at the WSL install would not be a test, it would be an incident. **Filed
as Phase 6, not as owed work** — the code is written, the run is what is missing.

---

## K16

**Status: ANSWERED — ruled 2026-07-23.**

**In the tree today:** Built as recommended.

**K16 — Does `clean-cache --all` need the guard?** It removes no packages, so today's answer is
no (R19 established exactly this reasoning for `clean-cache`). Level 3 of X.3 is a different
command and does need confirmation. *Recommendation:* keep the split — the guard protects
packages, not disk space, and widening it to cover caches dilutes what a guard refusal means.
**BUILT the split, 2026-07-20:** `clean-cache --all` takes no confirmation and no guard (it
touches caches and `tmp_dir`, no installed software); `linix reset` takes the typed-count
confirmation because it destroys the registry. The reason is written into `handle_clean_cache`.

**RULED (owner, 2026-07-23): confirmed as built.** `clean-cache --all` takes no guard and no
confirmation. **The guard protects packages, not disk space** — widening it to cover caches would
dilute what a guard refusal means, and the worst outcome of a wrong `clean-cache --all` is
re-downloading.

**`linix reset` is not part of this entry and was only ever the contrast** (owner asked,
2026-07-23). It is a different command answering a different question: it makes every managed
package unmanaged, which is why it takes the typed-count confirmation and `clean-cache` does not.
K5 is where that lives. Recorded because the contrast read as though the two were one decision.

---

---

## U5

**Status: ANSWERED — ruled 2026-07-23.**

**U5 — Does `setting:` get a Windows registry adapter and a macOS `defaults` adapter?** This is
P7's first real test. *Recommendation:* yes, registry first — it is the cleanest
read-before-write store on any platform, and it is the difference between LiNix declaring a
Windows machine's software and declaring the machine.

**ANSWERED by K7's ruling (owner, 2026-07-23): yes.** `setting:` must work everywhere, so the
registry and `defaults` adapters are owed rather than optional. *(This once read "does not unblock the work — U19 is still open".)* **U19 is answered and
built** (2026-07-24): `@scope=user|system`, defaulting to whatever the store does anyway, and it
is the convention macOS `defaults` inherits. **Nothing blocks the adapters now**; they are simply
unwritten — `setting_stores.toml` still ships `gsettings` and nothing else.

---

## D1

**Status: ANSWERED — ruled 2026-07-23.**

**In the tree today:** `github.rs:159` takes GitHub's own `releases/latest` (non-draft, non-prerelease) and `:241` strips a leading `v` from a `@version=` pin. The recommendation's "errors if both exist" half is not there.

**D1 — What is "the release"?** `github:sharkdp/fd` names a repo, not a version. GitHub has
draft releases, prereleases, and tags that never became releases at all. And `@version=10.2.0`
has to mean *something* here — a tag, presumably, but tags are `v10.2.0` about half the time.
*Recommendation:* latest non-draft, non-prerelease release; `@version=` matches the tag with and
without a leading `v` and errors if both exist; no "track prereleases" option until someone asks.

**RULED (owner, 2026-07-23): confirmed as built, and the missing half is owed.** The release is
GitHub's newest non-draft, non-prerelease; `@version=` matches the tag with and without a leading
`v`. **Owed:** a repo carrying both `10.2.0` and `v10.2.0` as tags must be an error naming both.
Today one wins silently, which is the quiet failure this whole entry existed to prevent.

**ALREADY BUILT when the ruling was written — checked 2026-07-23, nothing to do.** `one_release`
(`github.rs`) returns `Error::Validation` naming both tags when both spellings resolve, and
`resolve_release` is its only caller, so there is no second path where one wins silently. Tests:
`a_pin_that_answers_to_both_spellings_is_an_error_naming_both` and
`either_spelling_alone_resolves_to_that_release`. It landed on **2026-07-20** in `8a63c80`, three
days before the entry said it was missing. **This is the tree being better than the sentence
again** — the same direction Part VII warns about, and the reason "In the tree today" lines are
worth re-running rather than reading.

---

## D10

**Status: ANSWERED — ruled 2026-07-23.**

**In the tree today:** `backends/artifact/format.rs` — one `Format` enum, one `ALL` table, the error names the legal set.

**D10 — The closed vocabulary, and where it lives.** VIII.2 fixes ten names and makes an
eleventh an error. That list has to live somewhere both the parser and the error message read
from, or it drifts — and a typed list of names that drifts is precisely the failure this document
has recorded seven times. *Recommendation:* one table in the grammar crate, and the error message
prints it rather than restating it.

**RULED (owner, 2026-07-23): confirmed as built.** One table, and the error prints the legal set
rather than restating it.

---

## W1

**Status: ANSWERED — ruled 2026-07-23.**

**In the tree today:** `$` shipped throughout `model/vars.rs`; a bare name is not accepted.

**W1 — The sigil: `$role`, or bare `role`?** IX.4 argues for `$`. The counter-argument is real:
bare names read better, the reserved set is five words and could simply be reserved forever, and
`$` in a file that is not a shell invites people to expect shell semantics (`${}`, `$(…)`,
env fallthrough) that will not exist. *Recommendation:* keep the sigil — the future-fact
collision is the kind of quiet, delayed breakage this document has recorded seven times — but
this is the single most reversible-now, expensive-later choice in the part.

**RULED (owner, 2026-07-23): confirmed as built.** The sigil stays. `$role`, never bare.

---

## W6

**Status: ANSWERED — ruled 2026-07-23.**

**In the tree today:** `model/vars_provider.rs:43` ignores directories by name: *"a `vars.d/` …"*.

**W6 — Is `vars` one file or a directory?** One file matches `active`/`priority`. A repo with
forty machines may want `vars.d/`. *Recommendation:* one file; revisit only with a real fleet
complaining.

**RULED (owner, 2026-07-23): confirmed as built.** One file. A `vars.d/` directory stays ignored
by name until a real fleet asks otherwise.

---

## K7

**Status: ANSWERED — ruled 2026-07-23.**

**In the tree today:** `backends/setting.rs` — the `SettingStore` enum is `GSettings` and `None`; `None` makes every `setting:` line an error.

**K7 — Which desktops does `setting:` adapt to, and in what order?** In scope as of the owner's
ruling (X.4), so the question is no longer whether. GNOME via `gsettings` is the largest
population and the cleanest adapter (typed schemas, readable current values); KDE via
`kwriteconfig` is ini files with no schema, so *reading the current value* — which X.4 requires —
is harder there. *Recommendation:* GNOME first, KDE second, and **`setting:` refuses on a desktop
with no adapter rather than falling back to writing something.** A key silently unapplied is
worse than an error, because the whole point is that the file is the truth.

**BUILT the recommendation, 2026-07-20: GNOME via `gsettings`, KDE refused for now.** The
`SettingStore` enum has exactly `GSettings` and `None`; a desktop that resolves to `None` makes
every `setting:` line an error naming the missing adapter. KDE joins by adding a variant and its
three command mappings — the pure-function shape is set up so that is the whole change.

**RULED (owner, 2026-07-23): `setting:` must work everywhere. GNOME-only is a stage, not the
answer.** The recommendation is confirmed as far as it goes and its scope is rejected: KDE, the
Windows registry and macOS `defaults` are all owed, not optional, because **P7 says a feature is
unfinished until Windows and macOS have an equivalent or a written reason there can be none** —
and there is no such reason here. Every one of these stores can be read before it is written,
which is the only property X.4 requires.

**The refusal survives the ruling, and is the reason the ruling is safe.** A store with no
adapter makes every `setting:` line an error naming it. That is what lets the adapters land one
at a time without any of them being able to silently not apply a key.

**Everywhere means everywhere, and the named stores below are a priority order, not the set
(owner, 2026-07-23).** A blessed list of five is a list that is always missing the sixth, and the
machine holding the sixth gets an error for a key LiNix could perfectly well have written. The
rule is the general one: **`setting:` adapts to whatever settings store the machine is actually
running.** The table is where to start, not where to stop.

**This forces a mechanism question the old ruling did not have, recorded as [K17](#k17).** A
closed Rust `enum SettingStore` cannot mean *everywhere*: every new desktop would be a LiNix
release, and the machine that needs it is the one that cannot wait for one.

**The stores, in the owner's own order of need (2026-07-23):**

| store | how a value is read and written | state |
|---|---|---|
| **Windows registry** | the registry itself, typed | **owed, and first** — the owner's daily machine |
| **KDE** | `kreadconfig5`/`kreadconfig6`, `kwriteconfig` | owed |
| **COSMIC** | the file tree under `~/.config/cosmic/`, one file per key | owed |
| **Hyprland** | a plain text config file, plus `hyprctl` at runtime | owed, **and it may not be a `setting:` at all** — see below |
| GNOME | `gsettings` | built, and **the one store the owner does not use** |

**Hyprland is a different shape and must not be forced into this one.** The other four are
key-value stores with a read API; Hyprland's truth is a text config file, with `hyprctl
getoption` reporting a runtime value that can disagree with it. A `setting:` line there means
LiNix owning individual lines inside a file it did not write — which is not what any other
adapter does, and `link:` already places whole files. **Whether Hyprland is a `setting:` adapter,
a `link:` case, or a third thing is open and is not decided by this ruling.**

**This answers U5: yes.** It no longer waits on anything: the registry adapter's first line is
`HKCU` or `HKLM`, which is **U19 — answered and built** (`@scope=user|system`, 2026-07-24). The
adapter rows are unwritten work, not blocked work.

---

## K7b

**Status: ANSWERED — ruled 2026-07-23.**

**In the tree today:** `backends/setting.rs` implements the statement form.

**K7b — What is the key syntax?** `setting:SCHEMA/KEY @value=…` is one spelling; a backend-shaped
`gsettings:org.gnome…` is another and would reuse the `backend:name` parser instead of adding a
statement. *Recommendation:* the statement form, because the desktop is not a backend (X.4) and
the adapter is chosen by what is running, not by what the user typed.

**RULED (owner, 2026-07-23): confirmed as built.** The statement form. The desktop is not a
backend, and the adapter is chosen by what is running rather than by what the user typed.

---

## K8

**Status: ANSWERED — ruled 2026-07-23.**

**In the tree today:** Built as recommended: the affected commands bail, `doctor` reports git as degraded.

**K8 — How does a git-less LiNix announce what it cannot do?** Once at `init`, on every
affected command, or only in `doctor`. *Recommendation:* on the affected commands (they are
few, and that is where the user is when it matters) plus a `doctor` line. Never on `sync` —
warning on the command that runs unattended, every time, teaches people to ignore it.

**BUILT the recommendation, 2026-07-20.** The affected commands already said it — `rollback`,
`diff` and `history` each bail with "this needs git, run `linix git init`" rather than
crashing, and `git_autocommit` is a silent no-op without a repo. The one gap was the standing
`doctor` line, now added: `doctor` reports git as *degraded* (not a fault) when it is absent or
the config is not a repo, naming exactly what is unavailable. Nothing warns on `sync`.

**RULED (owner, 2026-07-23): confirmed as built.** The affected commands say it, `doctor` carries
the standing line, and `sync` never warns.

---

## K10

**Status: ANSWERED — ruled 2026-07-23.**

**In the tree today:** `cli/args.rs:507` and `:520`; `main.rs:192-193`. Two commands, exactly the recommendation.

**K10 — `linix edit` and `linix path`, or flags on an existing command?** *Recommendation:* two
small commands, because both are things a shell wants to call directly.

**RULED (owner, 2026-07-23): confirmed as built.** Two commands, because both are things a shell
calls directly.

---

## K11b

**Status: ANSWERED — ruled 2026-07-23.**

**In the tree today:** `config/settings.rs` — the platform config dir, and a flat file rather than a nested one so it cannot land inside the repo it locates.

**K11b — What is that file called and where exactly does it live?** It is not in the repo, not in
git, and not scanned; beyond that the platform config dir and `$LINIX_DATA_DIR` are both
defensible. *Recommendation:* the platform config dir — it is configuration, not data, and
putting it next to the data dir invites the assumption that deleting the data dir is safe.

**RULED (owner, 2026-07-23): confirmed as built.** The platform config directory, and a flat
file rather than a nested one so it cannot land inside the repo it exists to locate.

---

## K13

**Status: ANSWERED — ruled 2026-07-23.**

**In the tree today:** `schedule.rs` carries a `NEVER_UNATTENDED` list.

**K13 — Does `rebuild` appear in `schedules`?** *Recommendation:* no, and the parser should
refuse it by name, for the reason in X.1. A destructive repair operation that can be scheduled
is one that will run at 3am on a machine nobody is watching.

**BUILT (2026-07-20): the parser refuses it.** `schedule.rs` carries a `NEVER_UNATTENDED` list
(`rebuild`, `purge-unmanaged`) checked against the first word of `run`, so `run = sync --locked`
still parses. The refusal names the command and says why.

**RULED (owner, 2026-07-23): REVERSED, and generalised. The forbidden set is a list in config,
defaulted, and each command in it is independently removable.**

The hardcoded `NEVER_UNATTENDED` constant goes. In its place, **one `[guard]` list naming the
commands this machine refuses to run unattended, shipped with `rebuild` and `purge-unmanaged` in
it.** Taking a name out is how you permit it; that is the "one key for each" the owner asked for,
without a key per command and without a new mechanism the next dangerous verb would need adding
to by hand.

- **The default preserves today's behaviour exactly.** A config that says nothing refuses both
  commands, as it does now, so no existing setup changes meaning.
- **It answers the sibling in the same change.** `purge-unmanaged` is not a separate ruling and
  does not need one — it is a row in the same list, removable on the same terms, which is what
  makes this a fix to the class rather than to `rebuild`.
- **It is a `[guard]` list, not a `[schedules]` one.** `[schedules]` in `preferences.toml` was
  deleted by the 2026-07-20 audit so the `schedules` file is the only schedule store;
  resurrecting it would be the zombie key that audit killed.
- **The refusal names the list.** A `schedules` entry naming a forbidden command is refused with
  the list's own name in the message, so the way out is in the error rather than in the docs.

**BUILT 2026-07-23, exactly as ruled.** `[guard] never_unattended`, defaulted to `["rebuild",
"purge-unmanaged"]`; the `NEVER_UNATTENDED` constant is deleted. The list reaches
`schedule_config` as an argument rather than being read inside the model, so the rule has one
home (`preferences.toml`) and the check is testable without a config on disk. The refusal quotes
the key **and its current contents**. Five tests, including the two the ruling's own wording
implies and nothing else would have covered: **taking a name out permits that command and leaves
the other refused**, and **an empty list refuses nothing** — the alternative, an empty list
silently restoring the built-in pair, is the shape that makes a guard setting unable to mean what
it says.

---

## D3

**Status: ANSWERED.**

**D3 — Two assets, same format, both legal for this machine.** `fd_10.2.0_amd64.deb` and
`fd-musl_10.2.0_amd64.deb`. `formats = deb` selects both and must pick one.

**RULED (owner, 2026-07-20): shortest filename wins, said out loud, plus `@asset=` taking a
pattern.** Four parts, and the fourth is a separate axis the question surfaced:

1. **Default: shortest matching filename**, and the selection is *reported* — the plan names the
   asset it chose and the ones it passed over, and the chosen name goes in `locks/github` so it
   cannot drift under a pinned line. A guess that is printed and locked is not the silent guess
   D3 was afraid of.
2. **`@asset=` takes a glob, not just an exact name.** `@asset=*musl*` survives a version bump;
   an exact name does not, and a pin that needs re-editing every release is a pin nobody keeps.
3. **`@asset=` narrows, it does not select.** When the pattern still matches several, rule 1
   applies among the matches. One tie-break in the system, not two.
4. **`@asset=all` installs every match** rather than picking. This is the one shape the original
   question did not contain: a release that ships several artifacts you genuinely want.

**This answers D16** (`gnu` vs `musl` is exactly rule 1 plus `@asset=*musl*`) and closes it.

---

## D4

**Status: ANSWERED.**

**D4 — What does installing a `tarball`/`zip`/`binary` actually do?** A `.deb` is
self-installing; an archive is not. Something has to decide where it extracts, which file inside
it is the executable, and what lands on `PATH`.

**RULED (owner, 2026-07-20): extract, find the executable, shim it — with `@bin=` to name it
when the guess is wrong.**

- LiNix extracts to its own artifact directory and **must not invent a second way onto `PATH`.**
  A second PATH mechanism is the two-of-everything failure with a new name.

  > **Corrected 2026-07-20: this said "reuses `shim:`", and `shim:` cannot do this job.** A
  > shim is the linix binary deployed under the target's name; on startup it reads its own
  > filename and re-dispatches, running the bare name through `PATH`. Pointed at an extracted
  > binary that is *not* on `PATH`, it would find itself. `shim:` is a re-dispatch mechanism,
  > not a deployment one, and the two are different features that happen to write to the same
  > directory. **The rule that survives is the one that mattered: one deployment mechanism, not
  > one per backend.** See the 2026-07-20 entry in Part VII.
- **The default guesses**, by looking for an executable whose name matches the package. The guess
  is *reported* in the plan, like D3's — the same discipline, for the same reason.
- **`@bin=PATH` names the executable inside the archive** (`github:foo/bar@bin=build/bar`) and
  turns the guess off. It is the escape hatch for odd layouts and for archives holding several
  executables.
- **An archive where the guess finds nothing, or finds several, is an error listing what it
  found** — never a silent pick. D3's tie-break is for *assets*, not for executables inside one:
  two binaries in a tarball are two different programs, and shortest-name is meaningless there.

---

## D6

**Status: ANSWERED.**

**D6 — `@sha256` cannot cover a per-machine asset.** A shared module says
`github:x/y@sha256=…`, but the Ubuntu box downloads the `.deb` and the Fedora box downloads the
`.rpm`. **One hash cannot verify two files.** So either the checksum option is per-asset (a list,
keyed by filename — verbose, and generated by hand), or it is only legal alongside a single
pinned format, or checksums move into `locks/github` as generated content and stop being a
hand-written option. This collides directly with the unimplemented SEC checksum work in Phase 5
and **must be settled with it, not separately.**

**RULED (owner, 2026-07-20): checksums live in `locks/`, generated.** LiNix records the hash of
the artifact it actually downloaded, per machine, beside the asset name and URL VIII.2 already
puts there. `@sha256=` remains legal **only on a line that pins exactly one format**, where one
hash can cover one file and the user is asserting something checkable; anywhere else it is an
error saying why. *(Options offered: the lock, a per-asset hand-written list keyed by filename,
or legal-only-alongside-a-pinned-format with no lock changes.)*

Two consequences that follow and are not separately decidable:

- **This is the same work as the Phase 5 SEC checksum items**, not an adjacent feature. The
  `web`/`appimage`/`github` verification path and this lock field are one implementation.
- **A hash in the lock is a record, not a policy.** It says what was downloaded, so a change is
  visible in `linix diff` and a re-download that differs is an error. It does not by itself
  demand that the user pre-declare anything, which is what makes it work on a fleet where the
  asset differs per machine.

---

## D7

**Status: ANSWERED.**

**D7 — Does a `github { formats = … }` block in `priority` mean the backend is enabled?**
V.15 says listed = available. A block with an options body is still a listing, so presumably yes
— but then a user who writes only a formats block has silently enabled a backend. *Recommendation:*
yes, it enables — one list, one question, exactly as V.15 argues. Say so explicitly.

**ADOPTED and BUILT (2026-07-20): yes, a body is a listing.** `Priority::parse` pushes the
backend onto the order and stores its body, so a lone `github { formats = deb }` both enables
`github` and sets its default. One list answering one question, as V.15 argues. The alternative
— a body that configures a backend without enabling it — would mean `priority` had two kinds of
mention with different force, which is the `backend_priority`/`enabled_backends` split V.15
already deleted once.

---

## D9

**Status: ANSWERED.**

**D9 — A line's `formats` replaces the backend's list. Confirm.** VIII.2 asserts replace-not-
extend. The alternative (prepend the line's entries, keep the backend's as fallback) is more
forgiving and produces an order nobody wrote. *Recommendation:* replace, as written — but it is
an assertion I made, not a ruling, so it is listed here.

**ADOPTED and BUILT (2026-07-20): replace, at both seams.** `to_spec` writes the backend's
`priority` body into the spec first and lets the line's own options overwrite the key whole, so
all three levels compose as *line beats `priority` beats built-in default* with no partial
merge at either step. The merge happens once, in the one function that turns a declaration into
a spec, rather than in each backend — a backend that resolved its own precedence would be the
second implementation of it.

---

## W2

**Status: ANSWERED.**

**W2 — Are values typed?** IX.2 shows strings only. But `when $role in [travel, work]` already
exists in the grammar, so a list value is a natural request, and `when $gpu == true` reads worse
than a flag. Options: strings only and every comparison is a string compare (simplest, one type,
no coercion surprises); or add lists; or add booleans. ~~*Recommendation:* strings only for v1.~~

**RULED (owner, 2026-07-20): full JSON types — strings, numbers, booleans, lists.** The
position-1 recommendation above is void, as IX.6 said every W recommendation is. A provider that
returns JSON has these types already, and flattening them to strings at the boundary throws away
information the user deliberately produced. *(Options offered: strings only, strings plus lists,
or full JSON types.)*

**This buys a coercion problem, and the coercion rules are the work.** They are not a detail
that falls out of the implementation — each one is a place a comparison can quietly answer the
wrong question, so each is decided here or it is decided by accident:

- **No cross-type coercion in comparisons.** `"1" == 1` is **false**, not true, and not an
  error. A provider that returns a JSON string is making a claim about the type, and silently
  equating it to a number would make the type annotation meaningless.
- **`==` and `!=` are legal between any two values; ordering (`<`, `>`) is legal only between
  numbers.** Ordering strings invites version-compare expectations LiNix cannot honour —
  `"10" > "9"` is false under every string ordering and true under every intuition.
- **`in` tests list membership** with the same no-coercion equality.
- **There is no truthiness.** W3's "no bare `$flag`" holds and gets stronger: `when $gpu` stays a
  parse error suggesting `$gpu == true`, so an empty string, `0`, `false` and `[]` never quietly
  differ from each other.
- **A detected fact is still a string**, and comparing `$var` to one follows the rule above.

**BUILT (2026-07-20, fourth session).** `model/vars::Value` is the four types; one `parse_literal`
reads a `vars` line and a `when` right-hand side alike; `Value::equals`/`Value::order` and
`config/parser::eval_when` enforce every rule above. `<`, `>`, `<=`, `>=` are new to `when` and
refuse a non-number pair by name. One deviation, recorded: **string equality is
case-insensitive**, preserving the detected-fact behaviour `os == LINUX` has always had.

**Owed:** the value type lands in Part II with a Part V entry naming the bug (a comparison that
answers a question the reader did not ask), and `linix vars` (W12) prints the type alongside the
value or the whole feature is undebuggable. **Deferred until stages 2–6 land**, so Part II does not
describe a half-built feature.

---

## W3

**Status: ANSWERED.**

**W3 — Is a bare `$flag` a condition?** `when $gpu { … }` meaning "non-empty" is the obvious
shorthand and it needs deciding before people write `gpu = false` and find that it fires.
*Recommendation:* no bare form — require an explicit comparison, and make `when $gpu` a parse
error suggesting `$gpu == …`. **`false` as a truthy string is a footgun with no upside.**
**ADOPTED and BUILT (2026-07-20, fourth session):** a bare `$flag` in `when` is a parse error
naming the fix.

---

## W4

**Status: ANSWERED.**

**W4 — Where in resolution does `vars` load?** It has to be parsed and resolved before any file
containing `when` is evaluated, including `active` — which means before profiles are known. And
`vars` itself contains `when` over detected facts. So: detect facts → resolve `vars` → everything
else. *Recommendation:* state this as a fixed phase in II.7, because getting it wrong produces an
ordering bug that will look like an intermittent one. **BUILT (2026-07-20):** vars resolve once
per invocation, before any `when` is evaluated (`resolve_model`), and the resolved set is carried
on the facts and frozen into a saved plan so `apply` reuses it rather than re-running a provider
that could disagree (Stage 5). Owed: writing the phase into II.7 as text.

---

## W5

**Status: ANSWERED.**

**W5 — What does `linix check` do with `vars`?** `check` parses everything on demand (II.3). A
variable defined but never used is harmless; a variable *used* but not defined is an error W3/IX.3
catches at parse time. But an unused variable on a fleet may mean "the block that used it was
deleted on this branch". *Recommendation:* `check` reports unused variables as a note, not an
error. **BUILT (2026-07-20, fifth session).** It is not done through resolution but by a static
scan, which is the *more* correct reading of the intent: `model/vars::referenced_names` reads
every `$name` out of the model files (`modules/`, `profiles/`, `active`, `priority`, `schedules`,
and a line-file `vars`), and `check` lists any resolved variable absent from that set as a note,
never an error. Static because the motivating case is a fleet — a variable used only in another
host's `when host == …` arm must count as used, and this host's resolution never reaches that
arm. So the answer is the whole repo's references, not just the ones this box hit.

---

## W7

**Status: ANSWERED.**

**W7 — The undetectable variable — is there an escape hatch?** "Is this a work machine" is not
derivable from hostname, os, or arch on every fleet, and **IX.1's central claim quietly depends
on it usually being derivable.** When it is not, the options are: an env var
(`LINIX_VAR_ROLE=work`) — which makes the resolved state depend on how the command was invoked,
and II.6 already establishes wariness there (*"an unset `$PROFILE` must not empty the machine"*);
a gitignored local file — which is per-machine hand-maintained state, the exact thing II.1
forbids; or a refusal, forcing the user to add a `when hostname ==` arm. *No recommendation.*
**This is the decision that determines whether IX.1's argument is honest or a technicality,
and it should be ruled on before anything else in this part.**
**ANSWERED by the provider model (2026-07-20):** an external `vars.py` or the embedded `env(name)`
reads `LINIX_VAR_ROLE` (or any variable) itself, so the escape hatch is the environment via a
provider — no per-machine committed state, no LiNix-level env-var mechanism. The `env()` host
function is built (Stage 4).

---

## W8

**Status: ANSWERED.**

**W8 — Do variables work in `active`?** `when $role == travel { Travel }` is the single most
useful place for this feature and also the place with the sharpest edge: `activate` and
`deactivate` edit `active` as a file (II.6), including its `when` blocks, and they currently
reason about host blocks specifically — *"Travel is not active on this host, `active` line 4
activates it when host == laptop"*. That message and that logic have to learn variables.
*Recommendation:* yes, allow it, and treat the `activate`/`deactivate` message work as part of
the feature rather than a follow-up — a half-taught `deactivate` would report a state it did not
reach, which II.6 already calls out as the defect to avoid.
**CORE BUILT (2026-07-20):** `when $role == travel { Travel }` in `active` resolves — it read its
own varless facts before and failed with "unknown when key `$role`"; `parse_active` now threads the
run's facts (which carry the variables).

**COMPLETE (2026-07-20, sixth session), and it was a bug, not only a message.** The resolution path
had been taught variables; **every path that EDITS your files had not.** `activate -a`,
`deactivate`, `uninstall` and `declares` all read `active` through `HostFacts::current()`, whose
variable set is empty — and an empty set does not make `when $role == travel` a block that fails to
match, it makes `$role` an unknown key. Each of those verbs refused a correct file outright. Fixed
by deleting the varless readers rather than defaulting them: `parse_active`/`read_active` take
facts, `Editor::new` takes facts, and **`StateResolver::facts_for_host` is the one place that
produces them** (`resolve_model` now calls it instead of resolving variables inline). The messaging
half is `model::profiles::describe_gate`: a block is named with its variables' current values —
*"`when $role == travel` ($role is desktop)"* — because `active` holds the condition and `vars`
holds the value, and pointing a reader at the first without the second explains nothing. Verified
against the binary: `deactivate Trip` on a `when $role == travel` block reports the removal, the
emptied block, and the value, where it used to fail to parse the file.

---

## W11

**Status: ANSWERED.**

**W11 — Does `why` explain a variable?** When a package is present because
`when $role == travel` matched, `linix why` should say *"`$role` is `travel`, set at `vars`
line 6 by `when host in [thinkpad, x220]`"* — one hop further than it explains today.
*Recommendation:* yes, and W4's fixed resolution phase is what makes it cheap. Decide before
the resolver is written. **BUILT (2026-07-20, sixth session).** The definition half was W12's
`VarOrigins`. The gating half is now built: the resolver's per-statement `conditional` flag became
a **chain** of `Gate`s (a predicate and the line it is written on, `Gates` beside `Origin` in the
grammar — two questions, two answers). The chain composes across all three levels that can gate a
package — the `active` block that turned the profile on, the profile's block around its `use`, the
module's own block — and lands on the spec as `__gated_by`, filtered to the conditions that test a
variable, which is the hop `why` cannot make from the file alone. `why` prints it under
`because:`.

**A package or module reached twice keeps the shortest chain.** Reached once inside a condition and
once outside it, it is here unconditionally, and an explanation that names the condition anyway is
a wrong answer, not a partial one.

`to_spec`'s three provenance arguments became one `Provenance` in the same pass: origin, scopes and
gates answer three different questions and had begun to read as interchangeable, which is the
mistake that made `upgrade --module dev` match a filename.

---

## W12

**Status: ANSWERED.**

**W12 — Is there a command to print resolved variables?** `linix vars`, showing each name, its
value on this machine, and which line set it. Debugging a fleet without it means reading the
file and simulating the `when` blocks by hand. *Recommendation:* yes — small, and it is the
first thing anyone will want when a block does not fire. **BUILT (2026-07-20), completed fifth
session:** `linix vars` prints each name, its typed value, its type, the active provider (line
file / external / embedded), and now *"set at vars:6"* — the winning definition's line, or the
provider file for a script. Resolution carries a `VarOrigins` map beside the value set
(`resolve_with_origins`/`load_vars_with_origins`), computed by the one resolution core so the
value path never pays for it. This is the origin foundation W11 needs.

---

## W13

**Status: ANSWERED.**

**W13 — Does changing a variable go through the guard?** It must: editing one line in `vars`
can deactivate a profile and remove a hundred packages. That is the ordinary plan-and-guard path
(II.8) and needs no new mechanism — but it does mean **a one-line edit to `vars` is potentially
the most destructive edit in the repo**, and the plan output should make the cause visible
rather than presenting a hundred unexplained removals. **CORE SATISFIED by construction
(2026-07-20):** variables feed the desired state, which feeds the plan, which feeds the guard — a
`vars` edit that removes a hundred packages hits `max_removals`/`protected` like any other change.

**RULED (owner, 2026-07-20, fifth session): the plan shows a run-level note, not per-package
attribution.** The three options were a run-level note (compare the plan's frozen vars to this
run's and print *"Variables changed: role (travel → desktop)"* above the removals), per-package
attribution (resolve twice and diff the gating, so each removed package names the variable that
dropped it), or nothing. The owner chose the note. It is decoupled from the W11/W8 gating-side
tracking entirely — the plan already freezes its resolved vars (Stage 5), so the note is a diff
of two `Vars` maps with no second resolution and no per-package guesswork. It gives the cause
next to the count, which is the property W13 asks for. **BUILT (fifth session):** the plan/sync
preview prints changed variables above the removals when the run's vars differ from the frozen
plan's.

---

## W14

**Status: ANSWERED.**

**W14 — Does `vars` belong in `linix diff`?** Phase 4 limits `diff` to
`modules/profiles/active/priority/schedules`. **`vars` has to join that list or the file that
explains a change is the one file the change view cannot show.** *Recommendation:* yes; this is
a one-line fix that will be forgotten if it is not written down here. **BUILT (2026-07-20):**
`diff` and the git manifest views match `vars*` (the line file and every provider file).

---

## K1

**Status: ANSWERED.**

**K1 — Does `rebuild` remove everything before installing anything, or one package at a time?**
*This is the whole feature.* All-at-once genuinely forces orphan collection and can leave the
machine unusable partway through; one-at-a-time is safe and collects nearly nothing, because a
shared dependency is never orphaned at any instant. Batch-per-backend is a third answer.

**RULED (owner, 2026-07-20): batch per backend.** All of one backend's declared packages come
down, then all of them go back up, then the next backend. The reasoning the ruling settles on:

- **It collects.** Within a backend, a dependency shared only by packages that are all removed
  in the same batch really does become an orphan, so the repair actually repairs — which
  one-at-a-time does not.
- **It bounds the blast radius.** A failure strands one backend's software, not the machine.
  A box mid-`rebuild` of `cargo` still has a shell, a package manager and a network stack,
  because those are `apt`'s batch and `apt`'s batch already finished or has not started.
- **The backend is the unit the orphan question is asked in anyway.** `apt` cannot orphan a
  `cargo` crate. Batching by backend is not a compromise between the two extremes; it is the
  granularity at which the underlying operation is defined.

**Backend order is therefore load-bearing and is not the registry's iteration order.** The
backend that owns the shell and the system libraries goes first.

**RULED and built (2026-07-20): foundation first, where foundation is `needs_root()`, then the
rest, each tier in `priority` order.** The blast-radius reasoning first offered for this — put
the risky batch first so a strand lands furthest from boot — **is wrong and is not the reason**;
`apt` stranding first is the worst outcome available, not the best. The reason is dependency
direction: a crate can need a system compiler, and no system package has ever needed a crate.
See V.49.

---

## K3

**Status: ANSWERED.**

**K3 — What does `rebuild` do when the reinstall fails after the removal succeeded?** The
machine is now missing declared software and the command is halfway. Snapshot-and-revert
(II.10's pre-sync snapshot path) is the existing mechanism and probably the answer, but it has
to be decided, because "rebuild left me with nothing" is the review this feature gets if it is
not.

**RULED and BUILT (owner, 2026-07-20): snapshot, and revert on a failed reinstall.**
*(Options offered: snapshot-and-revert, stop-and-report, or refuse to start without a snapshot
provider.)* Three things the ruling settles that the question did not contain:

1. **One snapshot, taken before the first removal — not one per batch.** A per-batch snapshot
   could only restore the batch that failed, and by then an earlier backend has already been
   rebuilt on top of it. The unit of the rollback is the rebuild, not the batch.
2. **No snapshot provider is not a refusal.** `rebuild` still runs and says up front that a
   failure cannot be rolled back automatically, falling back to stop-and-name-what-is-missing.
   Refusing outright would make the command unavailable on every plain ext4 box, which is most
   of them.
3. **A failed *restore* is reported as its own outcome.** The machine is then both half-rebuilt
   and un-restored, and saying "rolled back" would be a lie about the state the user is in.
   That error names the snapshot and says to restore it by hand before anything else.

---

## K9

**Status: ANSWERED.**

**K9 — Is the backup command `bundle`, an alias, or nothing?** **RULED 2026-07-22 (owner): it is
`bundle`, finished.** Open since 2026-07-19 with the implementation deliberately unproposed; the
constraint that was recorded then — **not a second archive writer** — is what decided it. `bundle`
already writes everything a backup needs and stops at a `RESTORE.md`, so the answer is the
missing half: **`restore DIR`**, a command rather than an instruction file, refusing a non-empty
config directory unless told otherwise, with an end-to-end test that runs **without git** because
that is the case X.5 leaves it carrying alone. Reasoned in **V.59**; the rule is in **II.8**.

---

## K15

**Status: ANSWERED.**

**K15 — Does `plan` distinguish a rebuild's removals from real ones?** A plan showing "remove
214 packages" when all 214 come straight back is technically true and will terrify the reader.
*Recommendation:* yes — the plan says *reinstall* where remove-then-install is the same package,
and reserves *remove* for removals that stay removed.

**BUILT (2026-07-21).** `rebuild` prints its own plan, which never says "remove". The gap was
that the two transactions it runs go through the ordinary `sync` path, whose summary narrated
214 removals — the sentence K15 exists to prevent. The engine is now told which run it is
narrating (`metrics::Narration`, from the guard scope): under a rebuild the counters read
`Reinstalled` and `Removed to reinstall`, and plain `Removals` is reserved for removals that
stay removed. The backends' own progress logs are unchanged, deliberately: `apt` really is
removing those packages at that moment.

---

---

# Parked — each on a condition, and the conditions are checked now

**A parking condition is a claim that has to be re-checked, and D15's was not.** It said "parked
until D5 is answered", D5 was ruled and built five days later, and the entry went on saying
PARKED — filed among the questions that need nothing, which is where a question that needs a
ruling goes to be missed. **Every `PARKED` entry names what it waits on; nothing re-read those
names when the thing arrived.** `scripts/decision-count.sh --check` now does: a parked `Status:`
line must carry `waits on <what>`, and the run fails if the clause is missing or names a decision
that has since been ANSWERED (V.109).

**D15 is parked twice over, and the second parking is the interesting one.** Freed by D5, it was
re-parked the same day on a *measurement* rather than ruled on an argument — because what it
turns on is a fact about snapd that nobody here has observed. A condition a script cannot check
(D16 waits on someone hitting the case; D15 waits on an experiment) is allowed and says so, since
a clause that reads as checkable and quietly is not would be worse than none.

## D15

**Status: PARKED — waits on a measurement of what snapd does to a sideloaded snap.** Parked once
before on D5; **D5 was ruled 2026-07-24 and built 2026-07-26**, so that brake came off and nobody
noticed, and the entry sat under a met condition for a week (V.109). Reopened 2026-07-31 and
**re-parked the same day, deliberately and on a different condition** — the owner declined to
rule it on an argument when twenty minutes of experiment settles it.

**The condition is an experiment, and it is small.** On a machine with snapd: install a snap from
a local file, then make it refresh, and record what snapd does to it. The answer decides this
entry outright and nothing else needs deciding first. **Re-parking on a measurement rather than
ruling on a guess is the point** — the alternative was a ruling whose reasoning was "probably".

**D15 — `.flatpak`/`.snap` assets in a GitHub release.** They exist. Adding them to the
vocabulary means `github` installing something `flatpak` then does not own — **D5's ownership
question, one layer worse.**

**What D5 settled, and what it does not.** The installing backend owns the artifact: the lock
records it, removal routes back through it, and `check`'s dedup and `purge-unmanaged` defer to
it. That machinery is generic — `backends/artifact/system_pkg.rs` and
`Queryable::owned_system_packages` — so a `.flatpak` handed to `flatpak install` would inherit
the same ownership rule a `.deb` handed to `dpkg` already has, and the question D15 was parked on
has an answer.

**What is still a question is whether the rule holds where the manager is a running service.**
`dpkg` writes a database and then stops; `snapd` and `flatpak` keep running, hold remotes, and
refresh on a schedule of their own. So the sharp form of D15 is no longer "who owns it" — it is
**what a sideloaded snap does over time**, and that has two possible answers and they fail in
opposite directions: if snapd refreshes it from the store, LiNix's lock describes a file that has
been replaced; if it never refreshes it (a sideloaded snap has no store association to refresh
*from*), the declaration silently pins a build that ages forever while `check` reports it
healthy. **Which of those actually happens has not been measured** — it needs a machine with
snapd and a release asset, not an argument. A `.flatpak` from a release has the same shape one
step down.

**Two answers, failing in opposite directions, and nobody knows which one is real.** If snapd
refreshes a sideloaded snap from the store, the lock describes a build that has been replaced and
LiNix believes the machine matches a config it no longer matches. If snapd *never* refreshes it —
plausible, because a sideloaded snap has no store association to refresh from — the declaration
pins a build that ages for ever, takes no security updates, and `check` calls it healthy.
Neither is acceptable and they need opposite fixes, which is exactly why guessing is worse than
waiting.

**A cheaper question may kill it first: does anyone actually ship these as release assets?**
Flatpak's distribution model is Flathub and snap's is the Snap Store; publishing the raw file to
a GitHub release is unusual. If it is rare, "no, and here is why" is a complete answer that costs
nothing to write.

*No recommendation, and deliberately not one.* The ownership half is answered; the half about a
manager that acts on its own schedule is a fact about snapd, and facts are measured.

---

## D16

**Status: PARKED — waits on someone actually hitting it.** Not on another decision: `D3`'s
ruling already covers the case in principle, and what is missing is a real machine where the
ambiguity bites. An event, not a question, so no check can tell when it arrives — which is
itself worth writing down, because the alternative is a condition that reads as checkable and is
not.

**D16 — libc variants** (`gnu` vs `musl`, both valid for this machine). A real ambiguity
`formats` cannot express, and a fourth axis is not worth opening until someone hits it. **D3's
answer probably resolves this one for free**, which is the argument for answering D3 properly
rather than expediently.

---

# The Q-series — what building it kept asking

Nineteen questions the code raised after the six proposal rounds closed, each one found by a
thing that did not work. They were filed under *Parked or closed* because they were appended to
the end of the file and that was the last heading in it — nineteen entries, none of them parked,
under a heading that said they were. **A section heading is a claim about what is under it**,
and this file's whole argument is that an unchecked claim rots.

## Q1

**Status: ANSWERED.**

**Q1 — Does a failed install leave its line in the file?** **RULED 2026-07-27 (owner): withdraw
it when it can never succeed.**

The two integration harnesses had contradicted each other *in writing* about this for months —
`run-in-container.sh:252` said "The failure must not be left in the manifest",
`integration-windows.sh:259` said "a pinned name that a manager could not install is a failed
sync, not a wrong name, and only a name nothing can resolve is withdrawn" — and neither claim
was in any spec file. Both harnesses then deleted the line themselves and asserted it was gone,
so neither reading was ever tested.

The rule: **a line is withdrawn when the sync failed in a way that cannot succeed on a second
attempt** — `Unresolvable` (no backend claims the name) *or* a `CommandFailed` the backend's own
`ExitPolicy` classified `Permanent`. Everything else keeps the line, because a dropped network,
a held lock or a failed hook all mean you did mean it and retrying is right.

Three constraints came with the ruling and are part of it:

1. **Permanence is read off `CommandFailed`, never off `Error::retryability()`.** That method
   also calls a refusal, a cancelled prompt, a bad config file and an unsupported platform
   `Permanent` — true about retrying, and none of them says the name was wrong. Deleting a
   declaration because someone answered "no" to a prompt would be worse than the wedge.
2. **Only lines the manager actually named are withdrawn.** A batch install whose manager
   stopped at the first bad name leaves the rest alone; withdrawing on a guess is the one
   outcome worse than keeping.
3. **A line kept on purpose says so, names its file, and says how to remove it** —
   `linix unmanage <line>`. The wedge was never only that the line stayed; it was that nothing
   told the user which file to open. A wedge with an exit is not a wedge.

The rule is in **II.8**; the reason is in **V.90**.

---

## Q2

**Status: ANSWERED.**

**Q2 — Is a package manager you never installed "critical"?** **RULED 2026-07-27 (owner): no —
absent is its own state.**

`linix check health` opened with `Backends: 25 OK, 0 degraded, 23 critical (of 48 total)` on an
ordinary Windows box where nothing was wrong: the 23 were managers the user does not have, like
apt and brew. Meanwhile the `linix check` rollup said `ok health 25 backend(s) ready`, so the
summary and the detail view disagreed about the same machine.

The rule: **`HealthStatus::Absent` means the manager is not installed here and nothing asked for
it. `Critical` means it is installed, or `priority` lists it, and it cannot work.** Absent is
never counted as a failure and never colours the verdict. Fail-loud is about failures; a
manager you never asked for is not one.

The rule is in **II.10**; the reason is in **V.91**.

---

## Q3

**Status: ANSWERED.**

**Q3 — What does a mistyped command exit with?** **RULED 2026-07-27 (owner): 1, and the
published table stays at four codes.**

`readme.md:708` publishes four exit codes and says "a script can branch on them". Measured,
`linix nosuchcommand`, `linix --nosuchflag` and `linix sync --badflag` all exited **2**, which
that table defines as "a read-only command looked and found work to do" — so a CI job following
the documentation read a typo as a drifted machine, defeating the entire purpose of code 2. The
cause is that clap uses 2 for usage errors and exits before LiNix's own mapping runs.

The rule: **a usage error exits 1.** "Failed — something went wrong" is already in the table and
is exactly true; LiNix did not do what was asked. No fifth code, because "the same four
everywhere" is the property the table is for.

Ruled with it, as a straight violation of the same published contract rather than a new
question: **every refusal exits 3.** `purge-unmanaged`'s ratio refusal exited 1 because it was
raised with `anyhow::bail!` instead of `Error::Refused`, so it never reached the mapping. The
harnesses could not see it: they assert refusals with `nok`, which accepts any non-zero code.

The rule is in **II.8**; the reason is in **V.92**.

---

## Q4

**Status: ANSWERED.**

**Q4 — Are unverified backends labelled "experimental"?** **RULED 2026-07-27 (owner): NO. They
are tested instead, and nothing ships until they are.**

The proposal — from the readiness review, and recommended by its author — was to split the
backend list into *supported* (has passed a real install → list → binary → remove round-trip in
an automated gate) and *experimental* (everything else), and to say so in `check health`, in the
`priority` file `init` writes, and in the readme. 52 backends are registered and 22 have ever
been run against the real tool; every defect the review found lives in that remainder.

**The owner rejected it, and the reason is the rule:** *this codebase does things; it does not
cover for not doing them.* A label converts an unfinished job into a permanent disclaimer, and a
disclaimer nobody has to retire is one nobody does. LiNix does not go to production until every
registered backend has been thoroughly tested and reviewed — so the work is the coverage, and
the missing coverage is a **release blocker**, not a caption.

What follows from it, and all of it is binding:

1. **No `experimental` or `supported` label exists** — not in `check health`, not in `priority`,
   not in the readme. There is nothing to add and nothing to grep for.
2. **`linix init` keeps scaffolding every manager it finds.** Scaffolding fewer would have been
   the same disclaimer written as a default.
3. **A backend with no real lifecycle in an automated gate blocks the release.** That is the
   thing to fix, and the only thing.
4. **No new backend is added until the current set passes.** Unchanged by this ruling and
   reinforced by it.

This one is a rule about the project, not about the program, so it has no Part II entry. Its
reason is in **V.93**, and the coverage it demands is tracked in `plan.md`.

---

## Q5

**Status: ANSWERED.**

**Q5 — Does `@unverified` reach past the backends that download?** **RULED 2026-07-28 (owner):
YES — add it.**

The flag was scoped to `web:`, `appimage:` and `github:`, the three places LiNix itself fetches a
URL and runs the result, and II.2 said so: *"Downloading backends only."* helm broke the framing.
helm v4 verifies a plugin's signature before installing it, and a source that cannot carry one —
a git URL, which has no `.prov` beside it — is **refused outright**, not warned about:

```
Error: plugin source does not support verification. Use --verify=false to skip verification
```

So `helm:diff@url=https://github.com/databus23/helm-diff` could not be installed by any
declaration, and the readiness review's reading of this as an argv defect (E11) was wrong: helm
was correct and LiNix had no way to say the one thing that would let it through.

The rejected repair was to put `--verify=false` in helm's install command. That is a global
switch — verification off for every helm plugin, every user, invisibly — which is the failure
`@unverified` is a per-line flag to prevent. A helm-specific flag (`@no_signature`) was also
rejected: two spellings of one decision.

What is binding:

1. **`@unverified` is legal on `helm:`**, where it becomes `--verify=false`. Without it, helm's
   verification stands and LiNix passes no flag.
2. **The rule is "something would otherwise have checked"**, not "LiNix downloads". II.2's option
   table says that now, and `backends/artifact/capability.rs` holds the one table both the
   grammar and the install path read.
3. **`@allow_http` did not follow.** The two never imply each other (SEC2); helm's plain-HTTP
   switch is for OCI registries LiNix does not reach, so `@allow_http` on `helm:` stays refused.
4. **Per line, still.** An install batch whose specs disagree about the flag becomes two
   commands — a shared command would hand one line's opt-out to a line that never asked.
5. **Visible after the fact.** `status` lists it for as long as the package is installed, and the
   heading says *installed with* rather than *downloaded with*, since for helm LiNix downloads
   nothing.

The rule is in **II.2**; the reason is in **V.94**.

---

## Q6

**Status: ANSWERED.**

**Q6 — May a definition in `adapters/backends.toml` take a built-in's name?** **RULED
2026-07-28 (owner): YES, and only by saying so — it is a key.**

The onboarder registered custom backends last and skipped any name already in use, so a
definition named `apt` was silently ignored. That kept a pulled config from hijacking `apt` by
guessing a popular name, and that half stays. But it also meant that when a manager changes its
CLI under LiNix — helm v4's plugin signatures, pixi's renamed `global upgrade-all`, nimble's
`--` — the person watching it fail could see the fix and had no way to apply it before a
release.

What is binding:

1. **`overrides = true` on a definition replaces whatever holds that name**, built-in included.
   Absent, the collision is skipped with a warning that names the key.
2. **Two deliberate acts, never one.** The sentence in the definition, and II.12's approval of
   the file. A name alone changes nothing, which is the security property.
3. **Announced on every run that loads it**, naming the backend and the program it now runs —
   not once at approval time.
4. **`check health` answers for the replacement**, so an override whose binary is absent reports
   that backend critical. That needed no special case: health probes the definition that won.
5. **Backends only.** Snapshot providers, init systems and secret stores still never shadow a
   built-in. Same argument, different blast radius — a wrong `apt` installs the wrong thing, a
   wrong snapshot provider removes the rollback that was meant to save you. Widening is a
   separate ruling and has not been made.

The rule is in **II.1** (`adapters/`); the reason is in **V.95**.

---

## Q7

**Status: ANSWERED.**

**Q7 — Does the removal guard cover the resources a declaration puts in place, or only
packages?** **RULED 2026-07-28 (owner): the resources are guarded the same way.**

Deleting a `link:`, `service:`, `setting:`, `shim:`, `schedule:` or `repo:` line made the next
`sync` tear that resource down without counting it, without naming it in the plan or the
`--dry-run` preview, and without consulting `protected_packages`. Measured: five link targets
deleted — including one the user had protected — with `max_removals = 1` set, reported as
`already up to date`. `readme.md` had said for months that every path removing anything went
through one guard. There were eleven such paths and nine guards.

The alternative on the table was to leave the guard packages-only and merely *report* the
teardown before it happened. It was rejected on blast radius: a `link:` target can be a
decrypted secret, a `service:` is something running now, a `setting:` is system-wide.

What is binding:

1. **`protected_packages` applies to resources**, matched on the key and also on the final
   component of a path key — `protected_packages = ["vimrc"]` protects `link:/home/u/.vimrc`.
2. **`max_removals` counts the whole command**, packages and resources together, not each
   phase separately.
3. **OS-essential and undeclarable do not apply.** No resource manager publishes an essential
   list, and no resource key parses as a package line — applying that second test would refuse
   every teardown on every machine forever.
4. **`--allow-mass-removal` answers the count and nothing else**, exactly as it does for
   packages. Protection stays a refusal (V.26).
5. **The teardown is announced before it happens**, at a level the default filter shows, and
   `sync` no longer reports `already up to date` over work it did.
6. **`linix repo remove` is guarded too** — the imperative twin of the `repo:` teardown. Guarding
   one and not the other is the twin-branch shape `history.md` records as S6.
7. **The claim is re-counted, not re-asserted.** `tests/removal_guard_enumeration_tests.rs`
   enumerates every removal call in `src/` and fails on one no ledger entry accounts for.

The rule is in **II.10**; the reason is in **V.96**.

---

## Q8

**Status: ANSWERED.**

**Q8 — Should the security refusals return the documented refusal code, or the failure code
they return today?** **RULED 2026-07-28 (owner): all of them return 3.**

`readme.md` publishes four exit codes "so a script can branch on them", and `3` means LiNix
refused on purpose. That was true when it refused to remove too many packages and false for
every refusal about security. Nine sites, enumerated from the code rather than from the two
that were reported:

| site | refuses | rule | was |
|---|---|---|---|
| `core/download.rs` | plain HTTP | SEC2 | `Validation` |
| `core/download.rs` | unverified, no `@sha256` | SEC2 | `Validation` |
| `core/executor.rs` | a secret nothing protects | T5 | `Other` |
| `backends/link.rs` | decrypt into the git repo | T2 | `Validation` |
| `app/hooks.rs` | unapproved hooks | II.12 | `Validation` |
| `app/shim_manager.rs` | deploy over a foreign file | SEC1 | `Validation` |
| `utils/file.rs` | deploy over a foreign file | SEC1 | `Validation` |
| `app/apply/dotfiles.rs` | files outside `$HOME` | SEC3 | `Other` |
| `app/snapshot_restore.rs` | a registry path that is a file-read primitive | — | `Snapshot` |

The last was one of five the grader could not classify. The other four —
`model/firewall.rs`, `model/health.rs` and `model/rehearsal.rs` twice — were **already
correct**: the message is built in one file and wrapped in `Error::Refused` in another, and a
scan whose window stopped at the file boundary read the split as an offence.

**The exit code is the lesser half.** `main.rs` stated that the `Error::Refused` arm was "the
one point every refusal in the program passes through, so no command can be added that refuses
without the hook hearing about it". Someone who wires `on_guard_refusal` to be told when LiNix
refuses was told about a mass removal and **not** about a refused unverified download, an
unprotected secret or an unapproved hook — silent exactly where it matters most.

What is binding:

1. **Every refusal is `Error::Refused`**, whatever it refused, so exit 3 and the hook are
   properties of the variant rather than of each site remembering.
2. **The claim is tested, not commented.** `tests/grader_refusal_exit_code_tests.rs` enumerates
   every "refusing to" site, follows a message builder through as many hops as the code has,
   and separately fires a real approved hook through a real refusal.
3. **A refused declaration is kept, and said to be kept.** LiNix refused the line *as written*
   and the refusal names what to change, so the line is what the user edits. It is no longer
   described as something `sync` "will try again", which promised a retry that fails
   identically forever.
4. **Retryability follows.** Moving off `Validation`/`Snapshot`/`Other` makes these `Permanent`
   rather than `Unknown`, which is the true answer: nothing about a second attempt differs.

The rule is in **II.10** and the exit-code table in `readme.md`; the reason is in **V.97**.

---

## Q9

**Status: ANSWERED.**

**Q9 — Should a verb that takes a backend name refuse one that does not exist, the way
`install` does?** **RULED 2026-07-28 (owner): yes, with `install`'s message.**

`install nosuchbackend:foo` refused loudly and named both the file to edit and the spelling to
check. `list -b nosuchbackend` printed nothing and exited 0 — and so did `aptt`, `APT`, and the
empty string. Zero rows and exit 0 is exactly what a real backend with nothing installed
prints, so a user who mistyped `--backend` was told, in the program's own voice, that the
manager was empty. Principle I is "fail loud, never silent", and one verb obeyed it while its
siblings did not.

**It also disarmed a check.** §8.1's A bar is "every `[READY]` backend can answer `list`". Run
over all 24 ready backends on a Windows host, all 24 exited 0 — and only 11 returned any rows.
The other 13 were indistinguishable from a name that does not exist, so the measurement could
not fail for half its subjects.

What is binding:

1. **Every verb taking a backend name refuses an unknown one** — `list`, `upgrade`, `rebuild`
   and `repo`, checked from the code rather than from the one that was reported.
   **Completed 2026-07-29 for the `backend:name` form, which that enumeration missed.** The four
   verbs named above take the backend as a `--backend` flag; the same ruling applies to the verbs
   that take it as a prefix on a package spec, and none of them checked it. Measured after Q9
   shipped: `hold nosuchbackend:foo` **recorded a hold** against a manager that does not exist
   and answered `Held 1 package(s).` at exit 0; `unhold`, `unmanage`, `why`, `upgrade`, `rebuild`,
   `unlock` and `info` each answered a true sentence about the wrong thing at exit 0, and
   `uninstall` blamed the user's modules for a line they never wrote. Every one of those answers
   is byte-identical to what a correctly-spelled name gets when there is nothing to do, which is
   the same indistinguishability that made `list -b <typo>` a defect. All nine refuse now,
   through `App::require_known_spec_backends`, with the one message.
   *Enumerating a rule over "verbs that take a backend name" and then checking the four that
   take it one way is the shape this register keeps recording: the clause says "from the code",
   and the code has two spellings of the argument.*
2. **One message**, `install`'s, naming the `priority` file and the spelling. Two spellings of
   one refusal is how E18's family started.
3. **A real backend that cannot run here is a different answer.** `flatpak` on a machine
   without flatpak is not a typo — it is a fact about the machine — so it says that and exits
   0. Both used to be the same silence, and only one of them is the user's mistake.

The rule is in **II.8**; the reason is in **V.99**.

---

## Q10

**Status: ANSWERED.**

**Q10 — Should the `mix` backend install Hex before installing an archive from it?** **RULED
2026-07-29 (owner): yes — and it asks first, with `--yes` as the flag that forces it.** The
general form of the ruling, given on the same day for all three of Q10, Q11 and Q13: *LiNix
should ask whether to do that setup for you, with a flag to force it.*

Measured in the `tools` container, 2026-07-28 and again 2026-07-29. `mix`'s install is
`mix archive.install hex <name> --force`, which fetches an archive from hex.pm. That needs Hex
itself, and mix says so in as many words:

```
$ mix archive.install hex --force phx_new 1.6.16
Could not find Hex, which is needed to build dependency :hex
Shall I install Hex? (if running non-interactively, use "mix local.hex --force")
** (Mix) Could not find an SCM for dependency :hex from Mix.Local.Installer.MixProject
$ mix local.hex --force        -> * creating /root/.mix/archives/hex-2.5.1
$ mix archive.install hex --force phx_new 1.6.16   -> * creating /root/.mix/archives/phx_new-1.6.16
```

**What the ruling is, exactly.** LiNix does not install Hex silently and does not merely print
the command. It says what is missing and what it would run, and runs it if you agree —
`--yes` agreeing in advance, which is the same flag that already answers the plan. A
non-interactive run without `--yes` says what it would have asked and changes nothing, because
an installer that runs because a config file said so and nobody was there to look is the risk
the II.12 ledger exists for.

**Where it lives:** `[[prereq]]` rows, shipped compiled-in
(`src/app/apply/prereq_builtins.toml`) and extensible per repo through
`adapters/prereq.toml`, which rides the II.12 ledger like every other `adapters/` file. The
offer runs in II.7's phase 0, right after the bootstrap offer that gets a missing manager.

Two defects were found underneath this one and fixed with it, neither of them Hex's fault:

- **The canary could never have passed.** `mix:hex` was the harness's mix canary, and
  `mix archive.install hex hex` answers `No package with name hex (from: mix.exs) in registry`
  even with Hex present. The Hex defect and an impossible canary were reported as one failure.
- **`mix archive.uninstall` without `--force` prompts**, takes the empty answer from a closed
  stdin, **exits 0 and leaves the archive installed.** LiNix reported removals that did not
  happen — the scoop-exit-0 shape (E7), one manager over.

The rule is in **II.7**; the reason is in **V.102**.

---

## Q11

**Status: ANSWERED.**

**Q11 — What should `opam:` do on a machine with no opam switch?** **RULED 2026-07-29 (owner):
offer to create one, ask first, `--yes` forces it — and `check health` stops calling opam ready
until there is one.** Same ruling as Q10 and Q13, and the same mechanism.

Measured in the `tools` container:

```
$ opam switch show     -> [ERROR] No switch is currently set   (exit 50)
$ opam install -y ocamlfind -> the same error, exit 50
with a switch:  opam switch show -> `default` (exit 0), and the install succeeds.
```

**The compiler is still not chosen for you.** The offered command is
`opam switch create default ocaml-system` — the compiler the machine already has. It is not a
version number, because pinning one is choosing for someone: it fixes the compiler for the
whole account and it is a long build. On a machine with no OCaml at all it fails in four
seconds with opam's own `unmet availability conditions`, which is the point at which the choice
really is the user's and they name one. That is what the earlier recommendation ("report it, do
not create one") was protecting, and asking protects it too.

**The health half is built.** A manager that is installed, answers `list`, and cannot install
anything is reported **degraded** with the reason and the command — not `[READY]`. Degraded
rather than critical: reads genuinely work and the fix is one line. This is the W11 shape
(a backend that reports health and cannot work) applied to a *state* rather than to a missing
binary, and it is checked for manager-level prerequisites only — a per-package one like asdf's
plugin is a question about a declaration, and `check health` has no declarations.

**The `tools` image was also wrong, and that is fixed here.** Its Dockerfile ran
`opam switch create default` with no compiler and swallowed the failure with `|| echo "SKIP
opam init"`, so the image has shipped with no switch since it was written, every opam install
in it failed, and the nightly job read that as an opam defect. It now installs `ocaml-nox`,
creates the switch with `ocaml-system`, and asserts `opam switch show` succeeds — a setup step
nobody checks is a setup step that quietly stops happening.

The rule is in **II.7**; the reason is in **V.102**.

---

## Q12

**Status: ANSWERED.**

**Q12 — Should the integration sweep fail when its real coverage collapses, and against what
number?** **RULED 2026-07-28 (owner, deferring the threshold to the builder): a ratchet per host
class — record the count, fail when it falls, never when it rises.**

The coverage audit asks whether every registered backend got a lifecycle **or** a plan-smoke,
and a plan-smoke satisfies it. So a run with 4 real lifecycles and a run with 15 both PASS. The
clean Windows sweep reported 4 — not because anything broke, but because 8 of 15 canaries were
already installed on that host and the harness correctly refuses to remove software the user
already had. **The gate's coverage is inversely proportional to how much the machine is used,
and nothing noticed.**

G2 gave this audit a floor for an empty *registry*. It had none for collapsed *lifecycles*.

What is binding:

1. **A ratchet, not a threshold.** The honest number varies by host, and a number guessed once
   is the kind of constant this repo keeps rediscovering was wrong. A ratchet needs no guess: it
   only asserts "this host class did better before, so it can again".
2. **The host class is `<harness>-<os>[-<distro>]-<ci|local>`.** `ci` and `local` are separate
   deliberately — that *is* the difference the finding is about, and holding a developer's box
   to a clean runner's number would make the gate red on every machine that has been used, which
   is how a gate learns to be ignored. The distro is in the key because ubuntu and the `tools`
   image are not comparable runs.
3. **The OS token is normalised.** `uname -s` under git-bash is `MINGW64_NT-10.0-26200`; keying
   on it would mint a fresh host class, and a free pass, at every Windows update.
4. **An unrecorded host class is recorded, not failed.** A gate that fails the first time it
   sees a new platform is a gate that stops people adding platforms.
5. **The numbers live in `scripts/lifecycle-floor.txt`**, with the reasoning beside them, so
   lowering one is a visible line in a diff — the single edit the mechanism exists to expose.
6. **Both harnesses carry it**, native and container. A ratchet on one is the "guard on one
   command" shape again.

The rule is in **IV.1**; the reason is in **V.101**.

---
## Q13

**Status: ANSWERED.**

**Q13 — Should `asdf:` add the plugin a declared tool needs?** **RULED 2026-07-29 (owner): yes,
asking first, `--yes` forces it.** Raised with Q10 and Q11 as one question, because they are one
question: *when a manager needs a setup step before it can install anything, does LiNix do it or
print it?*

Measured in the `tools` container (asdf v0.14.1):

```
$ asdf install nodejs latest   -> No such plugin: nodejs      (exit 1)
$ asdf plugin add jq           -> exit 0
$ asdf install jq latest       -> jq 1.8.2 installed!
$ asdf plugin add jq           -> `Plugin named jq already added`, exit 0
```

**This is the row that most needs asking rather than doing, and the owner's ruling gives it
that.** An asdf plugin is a third-party git repository whose shell scripts asdf then executes;
it is not a package download. The offer prints the repository-fetching command before it runs.

Two details the mechanism gets from this row specifically:

- **It is per declared tool, not per manager.** The plugin *is* the package name, so the row is
  offered once per line rather than once per `asdf:`. That is read off the argv (`{name}`), not
  declared beside it, so a row cannot claim one and be written as the other.
- **The probe reads output, not an exit code.** `asdf plugin list` exits 0 and prints
  `No plugins installed`, so an exit-code probe would report every missing plugin as present.
  One line of the output, trimmed, must equal the name — `jq` must not be answered by `jqx`.

The harness canary moved from `nodejs` to `jq` at the same time: both need the plugin, and jq's
downloads one binary in seconds where nodejs fetches a release tarball.

The rule is in **II.7**; the reason is in **V.102**.

---

---

## Q14

**Status: ANSWERED — ruled 2026-07-30.**

**Q14 — What should `@unverified` do on a tool version that has no flag to turn verification
off?** Q5 ruled that `@unverified` reaches a manager that verifies signatures itself, and named
helm's `--verify=false`. That flag is **helm 4's**. Measured 2026-07-30 on helm 3.16.2: `helm
plugin install --help` does not document it, `tool_help::accepts_flag` answers `Some(false)`,
LiNix withholds it and logs a `tracing::warn!`. GitHub's `ubuntu-latest` ships helm 3, so this is
the ordinary case rather than the exotic one.

Two consequences, both measured rather than argued:

- A `helm:` line carrying `@unverified` is accepted, resolves, and **does nothing**. If helm then
  refuses an unsignable source, the advice names the option the user already wrote.
- **The gate the round-3 grade asked for cannot exist on such a host.** Withholding a correct flag
  and withholding a drifted one are the same action, so no assertion can tell them apart there.
  `tests/grade2_flag_drift_blindspot_tests.rs` — written by the grader, verified green on their
  helm 4.2.3 host and red on a planted mutation — is **red on helm 3**, for a reason that is not
  drift.

The options:

- **(a) Leave it.** The table targets helm 4, the probe adapts, the warning is the notice. Drift in
  `VERIFIES_ITSELF` is then detectable only on a helm-4 machine, and the blindspot gate has to
  become a named skip everywhere else.
- **(b) Refuse the declaration.** `@unverified` on a backend whose installed tool cannot honour it
  is a config error, named in the file, so the option never silently does nothing. But it removes
  the only declaration that installs a helm plugin on helm 3 — which is what Q5's ruling existed
  to make possible.
- **(c) Version-split the behaviour**: emit on 4, refuse on 3.

Not decided in code. The builder's lean is **(b)** — "accepted and does nothing" is the class of
defect this register keeps closing — while noting that (b) is the option that takes a capability
away from a user on the older tool, which is why the choice is the owner's and not the builder's.

**RULED (owner, 2026-07-30): none of the three — the question rested on a wrong fact, and it was
measured before it was asked again.** helm 3.21.3's `helm plugin install --help` documents exactly
two flags, `--help` and `--version`. There is no `--verify`, no `--keyring` and no provenance:
**helm 3 does not verify plugins at all.** It verifies *charts* — `helm install --verify` and
`--keyring` are both there — and helm 4 added plugin verification, which is where `--verify=false`
comes from.

So on helm 3 the state `@unverified` asks for is the state the machine is already in. **The line
is accepted, no flag is built, and nothing is said** — not "accepted and does nothing", which is
the defect class this register keeps closing, but *accepted and already true*, which is a correct
no-op. Option (b) would have refused a correct declaration and removed the only way to install a
helm plugin on helm 3 — the capability Q5's ruling existed to create.

**The `warn!` goes.** Withholding a flag the tool never had is not an event, and a warning on a
run that did the right thing is how people learn to stop reading warnings.

**And the drift gate becomes writable rather than impossible.** It asserts the capability table
against what the installed tool does, in *both* directions: a flag in the argv where the tool
verifies, and no flag and no warning where it does not. That can go red on either version.
`tests/grade2_flag_drift_blindspot_tests.rs` asserts the one-directional version and is red on
helm 3 for a reason that was never drift; it is **replaced, not skipped**.

The rule is in **II.2** — the `@unverified` row and the subsection below the option table; the
reason is in **V.104**.

---

## Q15

**Status: ANSWERED — ruled 2026-07-30.**

**Q15 — Should a command whose whole output is a file it was told to write honour `--dry-run`?**
Measured 2026-07-30, on a fresh config, after the round-4 work made `--dry-run` a property of the
one writer:

- `linix --dry-run bundle --out X` writes **five files** into `X` — `active`, `priority`,
  `modules/starter.txt`, `packages.json`, `plan.json` — byte-identical to the run without the
  flag, and prints `See X/RESTORE.md for offline restore steps.`
- `linix --dry-run plan` writes `linix-plan.json`, the same as `linix plan`.
- `linix --dry-run export --out X` was **not** measured either way: the fixture had no package to
  export, so neither run wrote anything and there was no control.

II.8b says *"a preview writes no file the run would have written"*, and by the letter of it these
are violations. The counter-argument is in the exemption these three carry in
`tests/dry_run_every_verb_tests.rs` — *"writes to a path the user names"* — which is a real
distinction: nothing about the machine changes, the destination was named on the command line,
and `--dry-run plan` producing no plan is a command with no output at all.

The options:

- **(a) Exempt them by rule**, not by an exemption list nobody ruled on: II.8b gains a sentence
  saying a command whose *product* is a file at a path the user named is outside the rule, and
  says which commands those are.
- **(b) Honour the flag everywhere**: `--dry-run bundle` prints what it would write and writes
  nothing; same for `plan` and `export`.
- **(c) Split them**: `plan`'s output *is* its preview, so leave it; `bundle` builds a restore
  artifact and should not build one during a preview.

The builder's lean is **(c)**, and this was deliberately not decided in code: it changes what a
user sees, and the reason `bundle` feels different from `plan` is a judgement about what those
commands are for, which is the owner's to make. What was fixed instead is the half that is not a
judgement — the two verbs that *said* they had written when they had not (`lock`, `heal`).

**RULED (owner, 2026-07-30): (c), the split — and `export` goes with `bundle`.** The line is not
*"did the user name the path"* — they name it for `bundle` too — but **is the file the preview or
the result**.

- `bundle` and `export` produce an artifact that outlives the run and can be carried elsewhere. A
  restore bundle made by a preview is indistinguishable from one made deliberately, and the next
  person to find it cannot tell it was a rehearsal. Both honour the flag: they print what they
  would write, to where, and write nothing.
- `plan`'s file **is** its preview. `--dry-run plan` that wrote nothing would be a command with no
  output — the flag would turn the command off rather than make it safe. It is exempt, and the
  exemption is a rule with a reason rather than a line in a test's exemption list.

`export` was **not measured either way** by the grader — its fixture had no package to export, so
neither run wrote anything and there was no control. It is ruled with `bundle` on the reasoning
above rather than on a measurement, and that is recorded here so the builder measures it rather
than assuming this entry did.

Whatever a preview declines to write, it says so with the `[DRY-RUN]` marker every other verb
uses and never in the past tense — `bundle` printed *"Bundle written to X"* over nine files it
had really written under the flag, which is B-1's defect with the sign flipped.

The rule is in **II.8b**; the reason is in **V.105**.

---

## Q16

**Status: ANSWERED — ruled 2026-07-30.** Raised by the round-5 grader, measured rather than argued.

**Q16 — Is a bare grammar keyword a package name?** A package name is one bare word (II.2), so a
line containing only `link` is a grammatically valid package declaration — and that is what LiNix
makes of it. Measured on a release binary, a module containing the single word `link`:

```
$ linix eval
  "present": [ { "backend": "cargo", "name": "link", "source": "modules/kw.txt:1" } ]
$ linix --dry-run sync -y
  install 1   remove 0   (total 1 change(s))     backends: cargo
$ linix check
  ->  drift   1 to install, 0 to remove, 0 to place, 0 to undo
                 run `linix sync`
```

Thirteen of fourteen keywords behave this way, and each resolves to a real backend holding a real
package of that name — the resolver searched live indexes to produce these:

```
when -> cargo:when      absent -> pip:absent    link  -> cargo:link    service -> cargo:service
setting -> cargo:setting  shim -> scoop:shim    schedule -> cargo:schedule  repo -> cargo:repo
if -> gem:if            else -> npm:else        end -> cargo:end       import -> gem:import
include -> cargo:include                        use  -> refused (the only one)
```

**This is not a parser defect, which is why it is a question.** Written with their punctuation the
same words refuse correctly and legibly — `link:`, `service:`, `shim:`, `when linux`,
`when linux {` all exit 1 with a located `Configuration error`. The ambiguity is confined to the
bare word, and it is a property of the language rather than a bug in the code that implements it.

The cost is that the most likely typo in the format — typing a resource prefix and stopping before
the colon — silently declares a package, and every preview in the program then agrees, because the
model genuinely contains it. `check` recommends the sync. A side effect worth pricing in: resolving
one of these costs 10–27 seconds, since a bare name has no backend and the resolver asks every
manager in priority order; the same fixture with `cargo:ripgrep` is 0.2s.

The options:

- **(a) Reserve them.** A bare word that is a known keyword is a parse error naming the form the
  user probably meant (`link` → *"did you mean `link:PATH`?"*). Costs anyone whose package really is
  called `end` the need to write `cargo:end`.
- **(b) Warn and continue.** Declare it, but say so once per line. Keeps every name declarable and
  puts a sentence between the typo and the install — but a warning on a `sync` that is about to do
  the right thing for everybody else is noise, and V.42's objection to narration applies.
- **(c) Require a backend for a colliding name.** `end` alone is refused; `cargo:end` is accepted.
  This is (a) with an escape hatch, and it is the only option that keeps every package reachable
  while making the typo impossible.
- **(d) Leave it.** The grammar is consistent and II.2 is doing what it says.

The grader's lean is **(c)**. Deliberately not decided in code: it changes what a user sees, it can
remove the ability to declare a package by a bare name, and both are the owner's to rule on.

Red tests are committed and failing: `tests/grade4_keyword_is_not_a_package_tests.rs`, driven off
`known_prefixes()` so a prefix added later is covered without anyone remembering.

**RULED (owner, 2026-07-30): (c) — refuse the bare word, and keep every package reachable.** A
line containing only a keyword is a parse error that names both ways to mean it:

```
modules/dev.txt:4: `link` is a keyword, not a package name
  to link a file:                      link:/path/to/source @target=…
  to install a package by that name:   list:link   (or pin one: cargo:link)
```

**The owner asked for a way to still say "make it be a package", and the language already had
one.** A bare `NAME` is defined in II.2 as short for `list:NAME`, so `list:link` means precisely
what the bare form used to. **No quoting is introduced.** Quoting was the obvious shape for an
escape hatch and it is the wrong one here: **V.10** already rejected quotes because `"` needs
`\"` needs `\` needs a newline rule, and nothing about this question disturbs that reasoning.
The ruling adds a refusal and takes nothing away.

It binds the bare **word**, not the prefix — `link:` with its colon and nothing after it was
already a legible refusal and stays exactly as it is.

The rule is in **II.2**; the reason is in **V.103**.

---

## Q17

**Status: ANSWERED — ruled 2026-07-30.** A follow-on from `Q4`: that ruling made missing coverage
a release blocker, and this one says how the remaining coverage is obtained.

**Q17 — How does a backend that mutates the real machine get its first real lifecycle?**

Twenty of the sixty registered backends had never completed a real install → list → binary →
remove in any harness, and twelve of those were in **neither** the canary table nor the
exemption table of either sweep — no coverage and no stated reason. Three of them, `winget`,
`choco` and `psresource`, were excused in writing on the grounds that they *"install machine-wide
on a developer's real machine"*. The excuse did not survive being read next to the rest of the
table: `scoop`, `npm`, `cargo`, `go`, `dotnet` and eleven others install on that same real
machine and have had real lifecycles all along.

**RULED (owner, 2026-07-30): install and uninstall them, like everything else.** A disposable
Windows box is not a precondition. The harness already refuses to remove software the host
already had — that guard is what makes this safe, and it is the same guard the other sixteen
managers rely on.

**And privileged containers are authorised** for `lvm`, `zfs` and `btrfs`, which cannot be
exercised any other way: they need real block devices, and a loopback file in a `--privileged`
container is the only disposable way to give them one. These are the destructive effectors, the
code with the most to lose from being wrong, and they were argv-tested and never run.

What binds:

1. **A backend is excused from a lifecycle only for a reason that is *detected*, never assumed.**
   `choco` is skipped when the shell is not elevated and lifecycled when it is; `psresource` is
   skipped when the host has no PSResourceGet cmdlets and lifecycled when it has them. The
   pattern already existed for `pip` and PEP 668 and is now the rule: an assumed skip is a check
   nobody revisits, which is `Q4`'s disclaimer wearing harness clothes.
2. **"It touches the real machine" is not a reason.** Every package manager does. The reason has
   to be something the harness genuinely cannot do — no such userland, no such device, no
   account to sign in with.
3. **Coverage is measured across harnesses, not within one.** Each sweep audited only its own
   registry, so `winget` — excused on Windows and absent from Linux — was examined by nothing
   anywhere. A claim that some *other* image lifecycles a backend is verified on the run of that
   image, so no row can excuse a backend on the strength of a sweep nobody performs.
4. **A platform we do not have is still a release blocker, not an exemption.** `mas` needs a
   signed-in Mac; `pkg`, `pkg_add` and `pkgin` need BSD userlands a Linux container cannot host.
   Those stay counted.

This is a rule about how the project is tested rather than about the program, so it has no Part
II entry — the same shape as `Q4`, whose reason it extends. Its reason is in **V.93**.

---

---

## Q18

**Status: ANSWERED — ruled 2026-07-31.** Raised 2026-07-30 by the first run of the storage
backends in the project's history (`Q17`'s privileged image).

**Q18 — The storage backends read options the grammar refuses. Which half of Part II is wrong?**

`lvm:` cannot be written at all. Measured, in a privileged container with a real volume group on
a loopback device:

```
$ linix -y install 'lvm:linixvg/canary@size=64M'
Error: Configuration error: <argument>: `@size` is not an option
  options on a package are: version, hold, expires, until, requires, sha256, formats,
  asset, bin, channel, allow_http, unverified, health, download_only, url, and the
  `*_install` hooks.
```

And without it:

```
Error: `lvm:linixvg/canary` has no `size` — a logical volume needs one to be created,
       e.g. `lvm:linixvg/canary@size=10G`.
```

**The backend's own error message instructs the user to write a line the parser rejects.** There
is no third form. `lvm:` is unusable by construction, and has been since it was written.

**Part II says both things.** II.2's option table (`PACKAGE_OPTION_KEYS` in
`config/grammar/statement.rs`, whose comment cites II.2 as its source) permits fifteen keys and
none of these. The storage paragraph says the opposite in plain words: *"`zfs:tank/data` and
`lvm:vg0/data` join `btrfs:` as declared, sized, mounted objects — Rust, not a `ManagerConfig`,
**because a volume has a size and a mountpoint**, not a version."*

The full extent, from the code rather than from the paragraph:

| backend | options it reads | in II.2's table |
|---|---|---|
| `lvm` | `size` (**required** — install refuses without it) | no |
| `zfs` | `quota`, `mount` | no |
| `btrfs` | `quota`, `mount`, `options` | no |

So `lvm:` is entirely unusable; `btrfs:` and `zfs:` install fine by name and **every documented
option on them is refused**. A declaration that sizes or mounts a volume — the thing the
paragraph says these backends exist for — cannot be written.

**Why this is the owner's and not the builder's.** `CLAUDE.md` rule 4: *anything where Part II
looks wrong — do not fix Part II yourself.* Part II contradicts itself here, and both halves are
implemented. It is also rule 2: whichever way it goes changes what a user may write.

The options:

- **(a) Add them to II.2's table, scoped to the backends that read them.** `@size`, `@quota`,
  `@mount`, `@options` become legal on `lvm:`/`zfs:`/`btrfs:` and are refused by name everywhere
  else — the shape `@url` (U39) and `@download_only` (D3b) already use, with
  `capability::INSTALLS_FROM_SOURCE` as the precedent for one table both the grammar and the
  install path read. Makes the storage paragraph true. Costs one row per option.
- **(b) Delete the option reads from the three backends.** Volumes get created at a default size
  and never mounted or quota'd, which contradicts the storage paragraph and leaves `lvm:` still
  unusable (its size is required by LVM itself, not by LiNix).
- **(c) Delete the three backends.** They have never worked and nothing depends on them. `NO
  LEGACY` and *prefer deleting to fixing* both point here, and it is the only option that costs
  nothing to maintain.

**The builder's recommendation was (a)**, scoped by capability rather than globally: the paragraph
already rules that a volume has a size, the mechanism for a backend-scoped option already exists
and is used twice, and `btrfs:` and `zfs:` are otherwise working backends whose only defect is
that half their surface is unreachable. (c) was put to the owner anyway, because "never worked,
nothing depends on it" is the strongest argument this repo recognises.

---

**RULED (owner, 2026-07-31): (a), and the table is the half that was wrong.** `@size`, `@quota`,
`@mount` and `@mount_options` are legal on the backends that read them and refused by name
everywhere else. The direction given with the ruling binds the shape of it: **broaden so that
everything the code can do can be written — never restrict the code to fit the table.** So the
narrowed form the builder had offered — ship `@size` and `@quota` now, leave `@mount` refused
until the fstab path has been exercised — was **not** taken. `@mount` ships, and the fstab path it
reaches was fixed and given a real lifecycle in the same change rather than left legal on paper.

**`@options` is spelled `@mount_options`.** It is what an fstab entry's option field carries, and
in a flat namespace a key called `options` is a collision waiting for the next feature. Nobody
could have written the old spelling — the parser refused it — so there is nothing to migrate and
no second spelling kept alive.

**The ruling was applied to the whole family, not to the three backends that raised it.** Q18's
real defect is not storage: it is that `PACKAGE_OPTION_KEYS` and the keys backends actually read
were two lists with nothing holding them together. Reading every `options.get(` in the tree found
three more keys in the same state, and all three ship legal here:

| key | read by | what it had been doing |
|---|---|---|
| `classic` | `snap` | `--classic` is how an unconfined snap is installed; the branch had never run |
| `shim` | `sync` | a shim asked for on the tool's own line — **the form `R3` named** when it deleted the imperative `shim` command (2026-07-19) |
| `sandbox` | `sync`, `run` | the same shim, plus confinement for `linix run` |

`R3` is the one that stings. **Measured rather than asserted:** a standalone `shim:NAME` statement
still parsed and is still reconciled (`app/apply/dependents.rs`), so shims were not unmakeable —
the first draft of this entry said they were, and reading the code disproved it. What *was* true
is worse in a quieter way: R3 deleted the imperative command and pointed at `@shim=true` on the
package line as the declarative form, and a different change in the same month closed the option
table into a whitelist that did not contain `shim`. **The ruling pointed at the one form that did
not parse.** Neither change was wrong alone and nothing connected them, which is exactly why the
two lists are now one list with a test across the join (`backends::capability`,
`every_scoped_option_is_a_legal_option_key`).

**What the ruling cost beyond the grammar.** Making `@mount` writable made `btrfs`'s fstab code
reachable for the first time, and it was not fit to be reached: it dropped every fstab line
*containing* the mount point as a substring (so declaring `/mnt` would have deleted `/mnt/data`
and `/mnt/home`), it wrote `subvol=` as the declared path instead of the path from the filesystem
root (the same offset bug `list` was fixed for the day before, mirrored), and removal left the
entry behind — an fstab line naming a subvolume that no longer exists is a machine that stops in
the initramfs. All three are fixed here, with unit tests, and `@mount` now also collapses the
second name it creates for one subvolume so `remove-orphans` cannot offer to destroy a declared
volume under its other path.

**Running it found three defects reading it had not**, all of them in the newly-reachable mount
path: the UUID parser wanted a line *starting* `uuid:` from a report that reads
`Label: none  uuid: …`; the query was put to the **subvolume** when `btrfs filesystem show` only
answers for a filesystem; and `info()` — which is what the planner asks, not `list_installed` —
answered `Path::exists`, so any directory was an installed subvolume and the record it built
carried no properties at all. A half-applied declaration therefore reported itself satisfied for
ever, so a declared `@mount=` that does not match the machine is drift now, decided beside
`@version` and `@channel`.

**And the harness stopped being deliberately red.** The `lvm:VG/LV@size=64M` canary that failed
by name every run now passes; `btrfs`'s canary carries `@quota` and `@mount` so the fstab path
has a real lifecycle on a real device rather than a legal spelling. `lvm` then failed once more
for a reason that was **not** LiNix — `device not cleared`, reproduced with a hand-run
`lvcreate`, because the image's udev workaround `sed`-ed keys Ubuntu ships commented out and the
`grep` meant to verify it matched the comments and passed. Fixed in the image, and 13c now
probes with a real volume before claiming lvm has a canary. The storage job moves to the fast
matrix, which is what its own comment said to do the moment this was ruled.

**Final measurement, on real block devices: `pass=274 fail=0 soft=7`.** The image's
real-lifecycle floor goes 3 → 5 — apt, github, btrfs, cargo, lvm. `zfs:` is still not among
them and is not excused: this kernel ships no ZFS module, which `Q4` counts as a release
blocker. So `@quota` and `@mount` are proven on btrfs and `@size` on lvm; on `zfs:` they are
argv-tested and read the same options through the same table, and that is the honest limit of
what has been run.

---

## Q19

**Status: ANSWERED — ruled 2026-07-31.** Raised the same day by building `Q18`: the keys were
writable and applied, and nothing decided what a *changed* one meant.

**Q19 — When a declared size changes, does LiNix resize the volume?**

`@mount` converges: a declared mountpoint that does not match the machine is drift, and `sync`
re-applies it (that shipped with `Q18`, because a mount that silently never happened was the
defect being fixed). **`@quota` and `@size` do not.** Editing `@quota=100M` to `200M`, or
`@size=10G` to `20G`, changes nothing on the next sync — the volume is present under its name,
so there is no drift to act on.

Two questions, and they are not the same size:

- **`@quota` on `btrfs:`/`zfs:` is safe to re-apply.** `btrfs qgroup limit` and `zfs set quota=`
  are idempotent property writes, and lowering a quota below current usage is refused by the tool
  rather than destroying anything. The only real work is comparing a declared `100M` against a
  reported byte count without a normalisation bug — and a wrong comparison here has a specific
  failure mode worth naming: **every sync reports a change for ever**, which is exactly what D13
  warned about when it required a *readable* current value.
- **`@size` on `lvm:` is not.** Honouring it means `lvextend` to grow — and its mirror image,
  `lvreduce` to shrink, **destroys data** unless the filesystem is shrunk first, which LiNix
  does not do and should probably never do unattended. So this is not "add a comparison"; it is
  a decision about whether a declaration may resize a filesystem at all, and whether shrinking
  is refused by name.

**RULED (owner, 2026-07-31): it edits — and where editing can lose data, the line has to say
so.** A declaration is the machine's description of itself, so an edited `@quota` or `@size` is
drift like any other and `sync` converges it. The owner declined the half of the recommendation
that refused shrinking outright: shrinking is *allowed*, behind a flag, because the register's
job is to record what the user decided and not to decide it for them. What the flag buys is that
nobody shrinks a filesystem by editing a number and pressing enter.

**The rule, as built:**

- **`@quota` re-applies** on `btrfs:` and `zfs:`. Idempotent property writes; lowering one below
  current usage is the tool's refusal to make, not ours.
- **`@size` resizes on `lvm:`.** Bigger runs `lvextend --resizefs`. Smaller is **refused unless
  the line carries `@allow_shrink=true`**, and the refusal names the actual size, the declared
  size, and the way out.
- **`--resizefs` on both directions, never a bare `lvreduce`.** It shrinks the filesystem before
  the volume, so the bytes given up are ones nothing is using — which is what makes the flag a
  permission to *resize* rather than a permission to truncate. A filesystem that cannot shrink
  (xfs) fails there, before the volume is touched.
- **`@allow_shrink` without `@size` is a parse error**, the same one level in `@mount_options`
  gets: a line that reads "shrinking is allowed here" while nothing can shrink is worse than a
  line that does nothing, because someone will believe it.

**The comparison is by value, in bytes, and only the declared side is ever parsed.** Every tool
is asked for raw bytes — `zfs list -p`, `lvs --units b --nosuffix`, `btrfs qgroup show --raw` —
so `@quota=10240M` against a reported `10737418240` is not a change. `D13`'s failure mode was the
one to design against here: a comparator that had to reconcile `10.00GiB`, `10.00g` and `10G`
reports a change on every sync, for ever.

**Three states, not two.** A byte count is a limit; `none` is a backend that looked and found no
limit, which against a line declaring one is drift; **no property at all** is a backend that
could not look, and that is left alone. Reading "could not read" as "no limit" is how a quota
gets re-applied on every sync for ever; reading it as "satisfied" is how one never gets applied
at all.

**The sibling this fix would have shipped past.** `@mount` used to `return` out of the drift
check, so a line carrying both a mount and a quota had only the mount looked at — the second
option was dead the moment anyone wrote the two together, which is the ordinary way to write
them. The facets are OR-ed now. `@mount_options` was dead the same way and converges too: a
changed option field rewrites the fstab entry, where before it kept yesterday's options through
every sync and every reboot.

**Run, not argued — on a real logical volume, `docker/integration/run-in-container.sh` §14b.**
The storage image goes **`pass=274 fail=0 soft=7` → `pass=279 fail=0 soft=7`**, and the five are
these; the real-lifecycle ratchet holds at 5 ≥ 5 (`btrfs`, `lvm` both on real devices, `zfs`
still short a kernel module):

```
PASS  lvm: a bigger @size grew linixvg/resizer, 67108864 -> 134217728 bytes
PASS  lvm: a second sync over the same declaration left the volume alone
PASS  lvm: a smaller @size is refused by name and the volume is untouched at 134217728 bytes
PASS  lvm: @allow_shrink shrank linixvg/resizer, 134217728 -> 67108864 bytes
PASS  lvm: the resize canary uninstalls
```

The edit is made **in the module file and applied by `sync`**, never by re-running `install` —
`install` hands a named spec to the backend, so it would prove the resize argv and skip the half
that was broken, which was the planner deciding a volume already present under its name still
needs work. **The second line is the one that could not be got any other way:** D13's failure
mode is a comparison that reports a change on every sync for ever, and a harness that syncs once
cannot see it. `btrfs:` and `zfs:` did not run here — this kernel has neither module (Q4's
release blocker, unchanged) — so `@quota` re-apply is argv-and-unit-tested and not yet executed.

**Deleting the option is not declaring "no limit".** A line that drops `@quota=` stops declaring
a quota; it does not ask for the existing one to be lifted. That is the same reading `@mount`
already has — an option nobody wrote is an option nobody is managing — and the opposite reading
would make removing a word from a config file silently uncap a filesystem.

**What is not covered.** `lvm:` grows through `fsadm`, so a volume carrying **no** filesystem
fails loudly rather than growing silently — the honest limit of resizing by declaration, and
better than applying half of one. And a resize appears in the preview as an install of the
object, the same shape `@mount` and `@channel` already take; a `Resize` node of its own would be
a fourth spelling of "this line needs work" threaded through the guard, the transaction and the
preview.

---

## Q20

**Status: ANSWERED — ruled 2026-07-31.** Raised the same day by `Q19`'s sibling sweep: the fix
for storage geometry found the identical defect on `snap:`, and the owner ruled it the same way
in the same session.

**Q20 — When `@classic` changes, does LiNix re-confine the snap?**

`@classic` is read in exactly one place: when the install argv is built. So a snap that gains the
option *after* it was installed stays strictly confined for ever — `snap list` shows the name,
the planner finds nothing to do, and `sync` reports success over a declaration it never applied.
That is `Q19` word for word with a different noun, which is why it was found by looking for
siblings rather than by anyone hitting it.

**RULED (owner, 2026-07-31): yes, the same.** An edited `@classic` is drift and `sync` converges
it.

**But the two directions are not symmetric, and snapd is what makes them asymmetric.**
`snap refresh --classic` relaxes confinement in place; **there is no switch that narrows it
back.** The only way from classic to strict is remove-and-reinstall — a removal, of a package the
user declared, to satisfy an option. That is the guard's decision and not a backend's, so:

- **`@classic=true` on a strictly-confined snap** → `snap refresh --classic`. Automatic, and
  nothing is destroyed.
- **`@classic=false` on a classic snap** → **refused by name**, with the message saying snapd
  cannot narrow confinement and naming the by-hand path. The same shape as `Q19`'s shrink
  refusal, and for the same reason: the direction that removes something says so out loud.
- **No `@classic` at all** → **unmanaged**, exactly as a dropped `@quota` is. A line that says
  nothing about confinement is not asking for strict, so it can never schedule that removal. This
  is what keeps the refusal above from firing on configs nobody edited.

**The sibling inside the sibling.** `@channel`'s drift check `return`ed from the function, so a
snap carrying a channel *and* `@classic` had only the channel looked at — the identical fault
`Q19` fixed for `@mount`, in the branch immediately above it, found because the fix for one was
being written next to the other. Both are folded into one accumulator now. The argv had the
matching bug: the refresh was built from `@channel` alone, so a line asking for both changes
would have dropped one. One refresh carries both switches.

**And `snap info` is now read once.** `is_installed` asked `snap list` and `current_channel`
asked `snap info` for facts printed on the same page of the same report — two subprocesses per
snap per sync, and two answers that could disagree across the gap between them. Presence is the
`installed:` line, not the exit code: `snap info` answers just as happily for a snap that only
exists in the store, and reading that as installed would send every first install down the
refresh path.

**Not run.** No image here has a working snapd — a container cannot run it — so this is argv- and
unit-tested against real `snap info` output and has never been executed. It is named that way
rather than counted, exactly as `zfs:` is.

---

## Q21

**Status: ANSWERED — ruled 2026-07-31.** Raised by `Q19` and `Q20` landing in the same session
from the same cause, which is the definition of a class rather than two bugs.

**Q21 — Must every option a backend reads converge when it changes?**

`Q19` found four options applied at creation and never again (`@quota`, `@size`, `@mount`,
`@mount_options`). `Q20` found a fifth on a different backend (`@classic`) by asking the rest of
the tree the same question. Neither was reported by a user; both were found by looking. **The
question is therefore not about those five — it is whether "an option changes the machine when
you change it" is a property of every option, or a thing five options happen to have.**

**RULED (owner, 2026-07-31): every option, and the builder proves it per option.** An option a
backend reads is a declaration; a declaration that stops applying the moment it is first applied
is the defect `Q18` existed to prevent, arriving one layer in. So:

- **Changing an option changes the machine**, or the line is refused with a reason. There is no
  third outcome, and "nothing happens" is not one of them.
- **The proof is per option, not per backend.** A lifecycle is install → list → remove, which by
  construction never edits a declaration — which is exactly why all five sat dead through
  thousands of green checks. A backend with a real lifecycle is *not* covered for this.
- **Where the change cannot be applied, it is refused by name** with the by-hand path
  (`Q19`'s shrink, `Q20`'s narrowing), never ignored.
- **An option the line omits manages nothing.** Absence is not a declaration of the default, or
  every existing config acquires refusals it never asked for (`V.107`, `V.108`).

**In the tree today: NOT swept.** The five named above converge and are tested. Every other
option a backend reads is unaudited against this rule — `PACKAGE_OPTION_KEYS` and
`capability.rs`'s tables are the list to work from, and the work is in `plan.md`'s Tier 0. The
grader carries the same sweep as §3.5, because it is the check and this entry is the obligation.

## Q22

**Status: ANSWERED — ruled 2026-07-31.** Raised by the round-8 builder after reading `linix eval`
on a module saved the way Windows saves files.

**Q22 — A config file that begins with a byte-order mark: refuse it, or read it?**

Notepad writes UTF-8 **with** a BOM by default, and so does PowerShell 5.1's `Set-Content
-Encoding utf8` — the editor and the shell this project is developed in. The three bytes are an
encoding artefact: no editor displays them, and the user has no way to see what is wrong.
LiNix read them as part of the first name on the first line:

```text
$ linix eval
Error: .../modules/starter.txt:1: `<U+FEFF>cargo` is not a backend LiNix uses
  add `<U+FEFF>cargo` to your `priority` file, or check the spelling.
```

Two names that render identically, and advice the user has already followed. The alternative to
reading the file was refusing it by name — *"this file begins with a byte-order mark; save it as
UTF-8 without one"* — which is loud, honest, and makes every Windows user's first config fail.

**RULED (owner, 2026-07-31): read it.** A leading BOM is stripped where config text enters a
parser, for every file: modules, profiles, `priority`, `active`, `vars`, `schedules`,
`preferences.toml` and the settings file. The rule is II.1's, the reason is `V.112`.

**Only the mark, and only at the start.** A U+FEFF anywhere else in a line is a zero-width
character that nothing but a paste puts there, and it is still refused by name — stripping every
occurrence would be the silent repair this codebase exists as a reaction to. The refusal names
the codepoint rather than drawing it, which is the same session's fix to `GrammarError`.

**Where it is applied is part of the ruling.** At the parser, not at the read: `model/edit.rs`
reads the same files in order to rewrite them, and II.16 says LiNix must not rewrite your files
— which includes their encoding. A file that arrived with a mark keeps it.



## Q23

**Status: ANSWERED — ruled 2026-07-31.** Raised by CI: both `Build` jobs were red on
`tests/grade2_info_tests.rs`, on two platforms, for one cause.

**Q23 — Is an `@` that opens a package name part of the name, or the start of the options?**

npm's scoped packages are named `@scope/name` — `@angular/cli`, `@vue/cli`, `@bazel/bazelisk` —
and `npm ls -g` prints them, so `linix list` reports them. Writing one back was impossible:

```text
$ linix info npm:@bazel/bazelisk
Error: `@bazel/bazelisk` is not a list of `key=value` options
  commas need the block form.
```

`@` introduces an option (`@version=1.2`), so a name starting with `@` was read as an empty name
followed by nonsense options. **A name LiNix lists has to be a name LiNix accepts**, and the
error was advice about a mistake nobody had made.

**RULED (owner, 2026-07-31): the leading `@` is part of the name.** Only the first character of
the name is special; every later `@` still opens the options, so `npm:@angular/cli@version=17.3.0`
is a pinned scoped package. Nothing existing can break — a line beginning `npm:@...` did not
parse, so no config anywhere contains one. Rule in II.2, reason in **V.113**.

**The owner also named a fallback and it is deliberately NOT built:** *"if it gets confusing, we
can have them quote it."* Quoting was rejected once as **V.10** — a quote needs an escape, which
needs a backslash rule, which needs a newline rule — so it stays a decision to be re-argued
rather than a convenience to be assumed. Nothing today is ambiguous enough to need it: the rule
is positional, and there is exactly one special position.

## Q24

**Status: BUILT, NEVER RULED — built 2026-08-02.** The owner said *"find the linix bug (not just
symptom but root) and fix"*, which rules that a bound must exist. It does not rule the number, and
the number is the whole cost of being wrong in either direction.

**Q24 — How long may a command say nothing before LiNix stops waiting for it?**

`linix -y uninstall choco:bat` ran 76 minutes and removed nothing. The child was
`Checkpoint-Computer`; Windows event 8194 records the restore point created **18 seconds in**, and
the process then emitted nothing at all for the remaining 76 minutes without exiting. Nothing in
LiNix bounded it: the only timeout in the tree wraps the transaction DAG, and snapshots, state
reads, guards and `plan` all run outside it. Two earlier hangs (`gem:colorize` at eight minutes,
`github:sharkdp/fd` at fifteen) were killed by hand and recorded as undiagnosed; the fix then went
into the *harness*, not the product.

**The shape is settled and is not what needs a ruling.** The bound is on **silence, not
duration** — a working `cargo install` prints for an hour and must never be touched, and no
wall-clock cap can be set above that and below a hang. Rule in II.12, reason in **V.114**.

**What is built, pending confirmation:**

- `command_idle_timeout_secs`, default **900**, `0` removes the bound.
- 900 because the adversarial case is a command that is legitimately silent for its whole run —
  `Checkpoint-Computer` is exactly that — so the number has to clear a real one. It is a
  judgement, not a measurement: **nobody has measured the longest legitimate silence in LiNix's
  own workload.** The observed hang was 76 minutes and the observed real snapshot was 18 seconds,
  which brackets it loosely and no better than that.
- Killing is `Retryability::Permanent`. Retrying spends another full bound per attempt on a
  command that has already proved it does not finish, and three silences teach the user nothing
  the first did not.

**What could reverse it:** a lower default (a hang costs less, a slow silent command breaks), or
per-class bounds keyed to `latency.rs`'s `Class` (a read bounded in seconds, a mutation in the
quarter hour) — which is better and is not built, because the classes describe LiNix's own verbs
and the bound is per *child process*.

---

## Y1

**Status: ANSWERED — ruled 2026-08-02.** Raised by `docs/INEFFICIENCIES.md`, measured with
counting shims around each manager binary rather than argued.

**Y1 — Does LiNix put several packages on one manager command line?** It did not. Every install
and every removal was its own DAG node and every node called its backend with a one-element
slice, so installing 50 apt packages was 50 `apt-get install <one>` invocations. Measured in a
disposable Ubuntu container: six declared packages produced six `apt` processes and 12,465 ms,
against 3,161 ms for `apt install` of *eight* packages as one command. One at a time, the same
packages scaled 1 → 2,131 ms, 2 → 4,017 ms, 4 → 7,372 ms, 8 → **31,901 ms**.

**RULED: batch.** Everything ready at the same moment, for the same manager, with no edge
between any two of them, goes on one command line. Rule in **II.19**, reason in **V.115**.

**What the ruling binds:**

- A dependency edge splits the wave; an install and a removal are two commands.
- The line is bounded — 100 names or 6000 bytes, whichever comes first — because `cmd.exe` caps
  a command line at 8191 characters and every manager has some limit.
- **Rollback granularity is unchanged.** What each package looked like before is still captured
  per package, before the command runs, and the compensation loop still walks per package. A
  batch that fails fails every package in it, which is what a single node failure already meant:
  any failure rolls the whole transaction back.
- The WAL still records per package, before the manager is invoked.
- `before_install` still fires per package and a failing one takes that package out of the batch
  rather than out of the run.
- **The telemetry says when packages shared a command.** Several packages reporting the same
  duration to the millisecond was previously the signature of a fully serialised run reported
  under a heading reading `Parallel Task Breakdown`; it is now the signature of a batch, and the
  line says which.

**What could reverse it:** a manager that mis-reports which package in a batch failed, making a
failure less locatable than it was. None found in the sixteen hand-written backends, all of which
already accept multiple names.

---

## Y2

**Status: ANSWERED — ruled 2026-08-02.**

**Y2 — Is one concurrency number enough?** `max_parallel` was doing three jobs: CPU/process
fan-out, transaction concurrency, and pure network fan-out. It defaults to the core count, which
is right for the first two and arbitrary for the third — on a four-core laptop `linix search` ran
its ~22 registry queries in six sequential waves.

**RULED: split the knob, do not remove it.** `max_parallel` stays the process knob (owner ruling
2026-07-17 kept it as a user-settable cap). **`network_parallel`** is new, defaults to **16**
regardless of cores, and bounds concurrent network requests. Rule in **II.19**, reason in
**V.116**.

**What the ruling binds:**

- Nothing that fans out reads a third number. The three hardcoded caps — `installed_sets`'s 8,
  the health probe's 4, `TransactionConfig::patient`'s 4 — read a knob now.
- Where two fan-outs nest, the cap is held by the leaf that talks to the network, so they do not
  multiply.
- **`upgrade` fans out across the managers that contend with nothing.** The `needs_root()` set
  stays strictly sequential; `cargo`, `npm`, `pipx`, `uv`, `yarn`, `pnpm`, `vscode`, `emacs`,
  `krew` and `go` overlap. This narrows a rule recorded in `history.md:2055` to the case its
  reason actually covers.
- **Variables resolve once per invocation** — which II.6b already required and the code did not
  do. Measured: one `linix check` ran the user's `vars.sh` three times.

**What could reverse it:** a registry that reads 16 concurrent queries as abuse. The default is a
judgement, not a measurement, and it is a key precisely so a machine can say otherwise.

---

## Y3

**Status: ANSWERED — ruled 2026-08-02.**

**Y3 — May a read-only command give up on a backend that will not answer?** `search` had no
per-backend deadline, so its latency was the maximum over ~22 registries rather than the median:
one rate-limited GitHub call set the whole runtime, and the command measured 15.5s / 25.5s /
48.0s / 160.2s across four runs. `check health` had already answered this question for its own
probe, with the reasoning for its number written beside it.

**RULED: yes, and it says so.** A backend that has not answered within twice the configured
network timeout (floor 30s) contributes nothing to a search and is named in the "backends that
failed and were skipped" line. Rule in **II.19**, reason in **V.117**.

**What the ruling binds:**

- A `@health=` port probe gives up after 5s. A *closed* localhost port refuses immediately, but a
  **filtered** one — which `apply/firewall.rs` can itself create — waits out the OS default:
  ~21s on Windows, ~130s on Linux, while deciding whether to roll a sync back.
- **A download still carries no whole-request timeout.** A release asset can legitimately take an
  hour, and a bound sized for an API call turns a slow link into a corrupt install. Where a wait
  is deliberately unbounded, that is stated rather than left to be inferred.

**What could reverse it:** a legitimately slow registry that a user needs and that now reports as
having failed. The alternative is the command that took 160 seconds without saying why.

---

## Y4

**Status: ANSWERED — ruled 2026-08-02.**

**Y4 — Must the pre-sync restore point finish before anything else starts?** It was awaited as a
barrier before any work, and on Windows it is `Checkpoint-Computer` — measured at **50.8s**, with
no faster API to swap to. So every install and every uninstall on Windows paid a fixed ~51-second
tax in front of work that had to happen anyway, and **nothing in the output said it was
happening**, so the pause read as a hang. It was reported as one, twice, and killed by hand both
times.

**RULED: it starts first and is joined last, and it announces itself.** The snapshot begins
before the read-only pre-flight and is joined immediately before the first mutating command —
which is the whole requirement, since a snapshot taken after the change would revert to the
change. Rule in **II.19**, reason in **V.118**.

**What the ruling binds:**

- A refused sync aborts it, so a refusal leaves nothing half-taken.
- The snapshot provider's PowerShell passes `-NoProfile -NonInteractive`. It passed neither; a
  user's profile ran on every snapshot operation. `psresource.rs` and `executor.rs` had passed
  `-NoProfile` all along, and this was the third of three.
- **The write-ahead journal is `journal.jsonl`**, one JSON value per line, appended. It used to
  re-serialise the entire map, pretty printed, through a temp file and a rename, on every state
  change — O(n²) bytes in the number of actions, under the one mutex every concurrent DAG worker
  has to take, which made it a throttle that got worse as `Y1` widened the graph. A pre-existing
  `journal.json` is not read: under NO LEGACY there is no old-format reader, and a wholly
  unreadable WAL is still moved aside and named rather than swallowed (S10).

**What could reverse it:** a snapshot provider whose work is not safe to overlap with reads. All
three built-in providers and the config-driven one only read while creating.

---

## Y5

**Status: ANSWERED — ruled 2026-08-03.** Raised by the owner while asking whether LiNix was fast
enough to adopt, and the question could not be answered from inside LiNix.

**Y5 — Does LiNix say where its own time went?** It did not. `latency.rs` measured the *total*
and warned when a class crossed its budget — enough to notice the 98-second `info` (E14), and
not enough to act on one, because the next question is always *which manager*. Answering it
meant timing each manager by hand outside LiNix and subtracting, which is how an afternoon gets
spent proving that a 3.2-second `list` is 2.35 seconds of `winget list` and 0.8 seconds of
everything else. `-vv` printed a running commentary with no durations in it.

**RULED: `--timings`, off by default, on stderr.** It reports the wall clock, the summed child
time, and the ratio between them, then every child command slowest first. Rule in **II.19**,
reason in **V.119**.

**What the ruling binds:**

- **The ratio is the point, not the list.** LiNix's whole design is overlapping other people's
  processes, so 19.52s of child time inside a 3.15s wall clock — 6.2× — is the claim `Y1`–`Y3`
  make, stated as a number the user can check on their own machine. A breakdown printing only
  a sorted list would show the same seconds and hide whether any of them were overlapped.
- **Recording is off unless asked for.** A measurement nobody requested is the eager work this
  whole round exists to delete.
- **stderr, never stdout.** `linix eval --timings | jq` still gets JSON.
- **Instrumented at the choke point**, `CommandExecutor::run_on` — the one call every manager
  invocation funnels through — rather than per verb. A budget every verb has to remember to
  check is the shape `latency.rs` already rejected.
- **The one automatic probe that spawns outside that choke point is instrumented too**
  (`psresource`'s PowerShell cmdlet check, and the external `vars` providers). Interactive
  children are deliberately left out: `linix shell`, the history pager, `bisect`'s test command
  and `setup`'s installer are the user's own program in the foreground, and a duration for
  "how long you sat in your shell" is not a fact about LiNix.
- The label is the program and its **first argument** — `winget list`. Keyed by full argv, an
  install would produce one row per package instead of one row per thing the run waited on.

**What could reverse it:** a per-child JSON emission (a `--timings=json`) if anything ever wants
to gate on these numbers in CI. Deliberately not built now — nothing consumes it, and
`lifecycle-floor.txt` is the standing argument against a threshold nobody measured.

---

## Y6

**Status: ANSWERED — ruled 2026-08-03.** Raised by the owner: *"it should cache optionally."*

**Y6 — May a manager's answer outlive the run that asked for it?** `Y1`–`Y5` took every
question LiNix asks in one run down to one ask, overlapped: `linix list` puts 19.5 s of manager
work into ~3.2 s, 6.2×, and the floor is `winget list` at 2.35 s. There is nothing left to
overlap. The next `linix list` then asks all 24 managers the same question again, about a
machine that in the ordinary case nothing has touched since.

**RULED: yes, and off by default.** `installed_cache_secs` (0 = never) reuses a manager's
installed listing across runs. Measured on this Windows box: **3.99 s → 0.68 s**, with one child
command surviving instead of 24. Rule in **II.19**, reason in **V.120**.

**What the ruling binds:**

- **Off by default, and that is the ruling, not caution.** Every other speed-up in II.19 costs
  nothing but concurrency. This one costs *correctness* when it is wrong, and being wrong about
  the machine is how a declarative tool removes something it should not have. The user turns it
  on for their machine, knowing their machine.
- **Any mutation drops it**, on disk as well as in memory. `forget_all` clears both, because an
  invalidation that clears the memo and leaves the file is an invalidation that does nothing —
  the next question re-reads the pre-mutation answer straight off disk. Third time this repo has
  found that shape (the guard's tenth removal path, the run-scoped memos, this).
- **`--no-cache` bypasses it for one run**, and `linix clean-cache` forgets it outright — the
  latter unconditionally, since the files may have been written by a run that had it on.
- **It answers a command that reports, and never one that writes its answer down.** `list`,
  `search`, `check`, `outdated`, `info`, `why` — an allowlist, so a command added later has to
  say it is a reader. The setting says how long a *reading* may be reused; it never said a plan
  or an adoption may be built on one. A `sync` planned from a listing taken before the user
  removed something by hand skips the install and reports success, which is a declared package
  left absent with nothing saying so — the exact class the "off by default" clause above is
  about, and turning the setting on must not buy it. **V.120a.**
- **Written per manager, through a temp file and renamed, and the temp name carries the pid.**
  A half-flushed listing read back is a *shorter* machine, and a shorter machine is a list of
  things to remove. The rename is only atomic per writer: two runs sharing one temp path — a
  shell prompt hook and a terminal — write into each other's file and rename the interleaving,
  which is that same torn listing arrived at by the mechanism meant to prevent it.
- **Every read failure is a miss, never an error.** Corrupt, unreadable, or a clock that moved
  backwards all mean "ask the manager", never "this machine is empty".

**What could reverse it:** a cheap per-manager change token — `dpkg` status mtime, winget's
own database timestamp — which would let a listing be validated rather than merely aged, and
would make an on-by-default cache defensible. Not built: it is one investigation per manager,
and the TTL is the honest version of what is actually known today.

---

## Y7

**Status: ANSWERED — ruled 2026-08-03.** Raised by the owner: *"we need a way to declare all of
winget's things."*

**Y7 — How is a package name with a space in it written?** It was not. `winget list` answers
with `ARP\Machine\X64\Mozilla Firefox` — the identifier `winget install` takes back — and the
grammar's *a package name is one word* refused it. `adopt` held such names back and said *"its
manager reports a name no package line can hold"*, which was true and was a wall.

**The measurement corrected the diagnosis twice.** The backslashes were already accepted —
`2c51968` had taught the grammar and the validator about them. On this machine the names LiNix
could not write were **161: six winget names, every one of them a name with a space, and 155
`service:` names that are not a package-line question at all** (see below). The 185-backslash
figure in `GRADE-2026-07-31.md` §5 G-2 describes a defect that no longer exists.

**RULED: quote it.** `winget:"ARP\Machine\X64\Mozilla Firefox"`. Rule in **II.19**, reason in
**V.121**. After it, **zero** names winget reports on this machine are undeclarable, and the
manifest `adopt` writes parses.

**What the ruling binds:**

- **Quoting is what keeps VI.1 shut.** *An unrecognised line is an error* (II.2) rests on prose
  not being a package name. Prose is not quoted, so `apt:this is just prose` is still an error
  while `winget:"Mozilla Firefox"` is a name.
- **The quotes are syntax, not name.** What round-trips is what was inside them.
- **An `@` inside the quotes belongs to the name**; the options still open at the first `@`
  after the closing quote. `npm:@scope/pkg@version=1.2` is untouched.
- **One function decides both whether a name can be written and how it is spelled**
  (`grammar::declarable_line`). They were the same question asked in two places — a check that
  round-tripped `backend:name` and a writer that rendered it by hand — which is exactly how the
  grammar could learn to quote while `adopt` went on emitting the unquoted form and producing a
  manifest that does not parse. That is `2c51968`'s bug in the other direction, closed before it
  could happen rather than after.
- **The validator learned the space with the grammar.** V.113 is that a name is admitted by a
  grammar *and* a validator; a rule that admits the backslash and stops at the space carries
  most of an identifier and not the one the user has.

**The message is fixed; what it was hiding is still open.** 155 of the 161 held-back names were
`service:` lines, and the reason printed for them was wrong: `service:AppMgmt` parses perfectly.
`is_declarable` accepted only `Statement::Package`, and `service:` is its own statement kind, so
every service failed a test about package lines and was reported as an unwritable *name*. The
grammar answers three ways now — `Declared::Package` / `Resource` / `Nothing` — and `adopt` gives
each held-back name the reason that is true of it.

**The question under it is `Y7a`, ruled below.**

**What could reverse it:** nothing found. The alternative — taking everything after the colon
as the name — was rejected: it re-opens VI.1, and it makes `@` unusable as the option separator
on the most common line in the language.

---

## Y7a

**Status: ANSWERED — ruled 2026-08-03.** *Should `adopt` take Windows services at all?* Owner:
*"services too get put in with no comment, just like packages. You can guard it, but you need
not."*

**RULED: adopt them as live lines.** Rules in **II.19**, reason in **V.124**.

**What the ruling binds:**

- **A service is a line like any other.** `service:AppMgmt@status=running`, uncommented, next to
  the packages. Deleting it stops and disables the service, and the manifest header says that in
  those words rather than in the word *uninstall*, which was true of every line before this.
- **The owner's argument was the load-bearing one, and it is now written down as V.124.**
  `purge-unmanaged` builds its list from `list_installed`, which for `service` is every running
  service — so all 155 were already sweep candidates, refused only because `protection_of` opened
  with *could a package line hold this name?* and a service line is not a package line. Declaring
  them makes them managed, and the sweep only takes what is not. The guard need not carry them.
- **It carries them anyway, on purpose.** A service started after an adopt is unmanaged again.
  `Protection::NotAPackage` refuses that by a rule about resources instead of by an accident about
  package names — the accident being one honest tidy-up away from handing the sweep 155 services.
- **The line carries what was observed, and no more.** A bare `service:` line means *enable and
  start*, and enable on Windows rewrites the start type to automatic. The init reports only
  *running* services, so `status=running` is the whole of what was seen. `adoption_options` is the
  seam; it is empty for every package backend.
- **The register's other half stays as it is.** `service::list_manual` still answers with the
  running set and `tracks_manual()` stays `true`. That is what this ruling makes correct: the
  question `adopt` asks a resource backend is *what state is this machine in*, not *what did you
  choose*, and `manual_source` now says so in the manifest instead of claiming user intent.

**What could reverse it:** the asymmetry between what an adopted line declares
(`status=running`) and what deleting it does (stop **and** disable). It predates the ruling and is
disclosed rather than narrowed; narrowing removal to match the declared options would be a change
to Q7's teardown rule and is the owner's, not this ruling's.

---

## Y8

**Status: ANSWERED — ruled 2026-08-03.** Raised by the owner: *"identify why it did not
parallelize properly and fix that."*

**Y8 — Why did a run with 33 child commands and 20 slots still go in waves?** Not because
anything was slow. `check drift` on a 298-package config took **9.13 s** to do about 2.3 s of
critical path, and the `--timings` breakdown named the reason directly: nine managers — gem,
pip, emacs, luarocks, dotnet, dart, nimble, bun, service — **started at 5.4 s**, and nothing at
all was running for the second before they did. Y5 built the instrument that could say that; it
is the first question here answered by reading LiNix's own report rather than by guessing.

**Two faults, one shape.**

- **The report asks each manager when its section gets to it.** `check` plans drift, then crawls
  for unmanaged packages, then probes health. The crawl wants every manager on the machine; the
  plan wants the nine that are declared. So fifteen managers waited out a plan that had no
  question for them — and every one was going to be asked before the command could answer.
- **The plan's fan-out is over specs, not managers.** A spec's answer comes from its manager's
  whole listing, so 256 winget declarations put 256 futures in a queue `max_parallel` slots
  wide, every one waiting on the same `winget list`, while scoop, choco and cargo went unasked
  for want of a slot. Measured: three managers at 0.3 s, the other six at 1.9 s.

**RULED: ask every manager the run will ask, at once.** Rule in **II.19**, reason in **V.122**.
A command that crawls the whole machine warms every listing before its first section runs
(`App::warm_installed`); a plan asks each manager it consults once, before it asks anything
about a package. **9.13 s → 3.9 s, overlap 2.7× → 5.4×, every listing starting inside a 0.26 s
window instead of spread over 5.4 s, and the report identical line for line.**

**What the ruling binds:**

- **Neither half adds a question.** The once-per-run memo already collapsed the duplicates, so
  what changed is *when*, not *how many*: same 33 children, same answers.
- **Only for commands that already ask everyone.** `warm_installed` is called by name at the two
  call sites that crawl the machine, never from `App::new`. A command that consults three
  managers must still wake three — pinned by a test that fails if a plan touches a manager
  nothing declares.
- **A concurrency budget spent on duplicate questions is spent on nothing.** `max_parallel` was
  never the limit here; twenty slots holding twenty futures that want one answer is a width of
  one, and no knob the user can turn says so.
- **Ordering, alongside.** The registry was a `HashMap`, so which managers got the first slots
  was Rust's per-process hash seed — two `linix list` runs differed by 530 lines and sorted the
  same. It is a `BTreeMap` now (**V.123**), which is also what makes any of these measurements
  reproducible.

**Left standing, and measured rather than assumed:** two managers still start late — `npm
prefix` and `pipx environment` at ~3.2 s — because their `info` needs a *second* question that
cannot be asked until their listing lands. That is a per-backend follow-up, not a wave, and it
is what the remaining ratio is made of.

**What could reverse it:** a machine where warming costs more than it saves — a manager whose
listing is expensive and whose section is usually skipped. None of the two call sites has one:
both end in a crawl of every manager.

---

## Z1

**Status: OPEN.** Raised by the readiness audit, 2026-08-03 (`AU12`). Not blocking: nothing in
the tree reads it. Blocking for *distribution*, which is a different question and the one that
matters — `scripts/install.sh` and `linix self-upgrade` both hand this program to other people.

**Z1 — Under what licence is LiNix published?** There is no `LICENSE` file at the repo root and
no `license` key in `Cargo.toml`. Under the Berne Convention that is not "free to use" — it is
*all rights reserved by default*, so a user who runs the install script has no licence to the
copy they now hold, and `crates.io` will refuse the package outright.

**This is the owner's to answer and nobody else's.** It is a legal choice about someone else's
work, it cannot be inferred from the code, and picking one silently would be the worst possible
version of "the builder made the call": every later contribution inherits it and un-picking it
needs every contributor's agreement.

**What an answer has to say:** the licence, and whether `Cargo.toml` gains `license = "..."`
(needed for a crates.io release) or `license-file`. The two conventional answers for a Rust CLI
are `MIT OR Apache-2.0` (the Rust ecosystem default, permissive) and `GPL-3.0-or-later` (the
package-manager tradition — dpkg, dnf, pacman). No recommendation is offered here, because a
recommendation on this one is a decision wearing a suggestion's clothes.

---

## Z2

**Status: ANSWERED** (owner, 2026-08-03). Raised by the readiness audit the same day (`AU8`).
Built in the same change.

**Z2 — `lock` and `unlock` were not inverses, and the mis-pairing could move packages.**

| command | what it touched |
|---|---|
| `lock` | `locks/versions.json`, and approved hooks and adapters at their current hash |
| `unlock` | `locks/bare.HOST.toml` — which *manager* an unpinned bare name resolved to |

Different files, unrelated jobs. Someone who ran `linix lock`, changed their mind and typed
`linix unlock` did not undo the pin: they discarded the recorded backend resolution, and
`unlock`'s own help stated the consequence — *"sync uninstalls the cargo copy, because two of
the same package is what this avoids."* So the obvious undo for a harmless command could
uninstall software.

**Reading the code to answer it found the surface was wider than the report.** There were not two
things called "the lock" but *three* — version pins, backend resolutions, and the approval hashes
for everything the config can execute — plus `hold`/`unhold`, which is a different question
(exemption from `upgrade`, not a freeze) and stays where it is. Only one of the three had an
inverse at all: **version pins could not be released by any command**, only by hand-editing
`locks/versions.json`. `sync --upgrade` is a per-run bypass, not an undo.

**And a second defect, in the same family.** `locks/versions.json` is written by exactly two
things — `lock`, and `heal`. Not by `sync`, not by `upgrade`. So `linix upgrade` moved a package
from 7.81.0 to 8.0.1, the pin still said 7.81.0, and the next ordinary `sync` — which converges
to the lock (U11) — read the old version back as `@version=`, found the installed one did not
satisfy it (an unadorned version is an equality constraint), and planned the package straight
back down. The upgrade did not stick. `sync --upgrade` had the same hole.

### The ruling

**Both verbs name the axis they act on**, as a positional value with `all` as the default:

```
linix lock   [versions|backends|scripts|all] [NAME…] [--list]
linix unlock [versions|backends|scripts|all] [NAME…] [--list]
```

- `versions` — the pins in `locks/versions.json`. `lock versions` records what is installed;
  `unlock versions` drops the pins, which nothing could do before.
- `backends` — `locks/bare.HOST.toml`. `unlock backends` is the old `unlock`, unchanged in
  behaviour and now unmistakable in name. `lock backends` records the resolutions explicitly,
  which previously only happened as a side effect of a sync.
- `scripts` — `locks/hooks.toml`: hooks, event hooks, adapters, `exec:`, `generate:`,
  health-check commands and the `vars` provider. `lock scripts` is the old approval half of
  `lock`; `unlock scripts` withdraws approval, which nothing could do before.
- **A bare `lock` or `unlock` means `all`** (owner ruling): a bare `lock` does what `lock` always
  did, plus recording backend resolutions, and a bare `unlock` releases all three. It is not
  gated behind a confirmation — the axis is what a user types to be careful, and a prompt on a
  command whose whole job is this would be the asking that II.15 already rejects.
- `NAME` scopes any axis and matches the whole ledger key or its tail: `unlock versions curl` and
  `unlock versions apt:curl` both pick out `apt:curl`.
- A name that picks nothing out **warns and changes nothing**, naming the ledger. It is not an
  error, matching what `unlock` already did for an unfrozen name.

**And every path that deliberately moves a version forward re-records it** — `upgrade` in all its
modes, and `sync --upgrade`. **Only entries already in the lock are refreshed.** A package nobody
pinned has no stale record to fight, and pinning it would make every `upgrade` a silent `lock`.

**What was rejected.** Renaming `unlock` to `forget-backend` and leaving `lock` without an
inverse was the recommendation this entry carried, and the owner went the other way: the axis
belongs in the grammar rather than in a verb name, because there were three ledgers rather than
two and a per-ledger verb name would have needed six. `NO LEGACY` applies — there is no `unlock`
that takes a bare name any more, and a name where the axis goes is refused with the three axes
listed rather than guessed at.
