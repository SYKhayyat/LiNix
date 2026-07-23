# The decision register — all 91, and which are answered

**One file, six features.** Every decision this design forces lives here, with its status. The
registers used to sit at the tail of six proposal parts and **none of them recorded whether they
had been answered**, so the same question could be argued twice and a question already settled in
code could be re-opened by anyone reading the register instead of the tree.

**A recommendation is not a ruling.** Where an entry carries one, it is the author's reading and
nothing more; the owner decides. When a decision is ruled, rewrite its entry as the rule, put the
rule in [Part II](target-state.md) and its reason in [Part V](why.md), and move the row here to
*Answered*.

## The five statuses

| status | means | what it needs |
|---|---|---|
| **OPEN — blocking** | Unanswered, and the feature cannot be built without it. | A ruling. |
| **OPEN** | Unanswered, and something can still be built around it. | A ruling, eventually. |
| **BUILT, NEVER RULED** | Nobody ruled — but code shipped that implements the recommendation. | Confirm or reverse. Reversing costs a change now and more later. |
| **ANSWERED** | The owner ruled, or another decision closed it. | Nothing. Kept because later work cites it. |
| **PARKED** | Deliberately not asked yet, and it says what it waits on. | Nothing. |

**Every status below was checked against the tree, not against the sentence.** Where an entry
says *built*, the file and line are named. That distinction is the point of this file: fifteen
entries the old registers listed as unanswered questions were in fact already answered by shipped
code, and two the old registers implied were fine are live defects.

## What each feature's questions cost if answered late

The six registers each opened with a note on *when* their questions get expensive. Grouping by
status loses that, so it is kept here:

- **D1–D17, artifacts.** D1–D6 block the backend outright. **D7–D10 are grammar shape: cheap
  now, expensive after the first real `formats` line exists in somebody's repo.** D11–D14 are
  behaviour over time. D15–D17 are parked on purpose.
- **W1–W14, `vars`.** W1–W5 blocked implementation and are all closed. W6–W10 are scope and
  grammar; W11–W14 behaviour and tooling.
- **K1–K16, N1–N7, T1–T5, U1–U26.** *Blocking* means one thing in all four: **this cannot be
  built without an answer, because two reasonable implementations differ.**

---

## Index

### Open, and blocking — 14

| | question | feature |
|---|---|---|
| **D3b** | Download-only artifacts: what is the option called, and does `check` count one as software? | artifacts |
| **D5** | A `.deb` installed by `github:` — does apt own it or does LiNix? | artifacts |
| **K2** | What is `rebuild`'s default scope? Is a bare `linix rebuild` an error? | rebuild |
| **K4** | Is `clean_cache_on_remove` every backend, or only the ones whose file LiNix knows? | rebuild |
| **U1** | Where does a custom backend definition live — the repo, or machine-local? | next |
| **U3** | What does removing an `exec:` line mean when a script has no inverse? | next |
| **U9** | Do the ten status commands collapse into one `linix check`? | next |
| **U14** | Is sharing wanted, and what makes a vendored module safe to run? | next |
| **U19** | Is LiNix acting for a user or for the machine? (`HKCU` vs `HKLM`) | next |
| **U22** | Does the dotfiles tree link files, or whole directories? | next |
| **U23** | What happens when a dotfile destination already holds the user's own file? | next |
| **U24** | Is a `.age` file inside the dotfiles tree a secret to decrypt? | next |
| **U26** | Is BSD supported, and what does `when family` answer there? | next |
| **T6** | Must there be a way to opt out of `backup_once`, or bound how many pile up? | secrets |

### Open, not blocking — 33

| | question | feature |
|---|---|---|
| **T7** | Runtime injection of secrets into process memory — reopened. | secrets |
| **D8** | May a `when` block appear inside an options body? | artifacts |
| **D11** | The default format order is detected, so a LiNix upgrade can silently change it. | artifacts |
| **D12** | Network, GitHub rate limits, and whether `sync` works on a plane. | artifacts |
| **D13** | Changing a `channel` — refresh, or remove and reinstall? | artifacts |
| **D14** | Does `why` explain which of the three levels chose the artifact? | artifacts |
| **D17** | What does `github:re:…@formats=` mean across repos with different assets? | artifacts |
| **W9** | Interpolation outside `when` — stay banned? | vars |
| **W10** | May a variable reference another variable? | vars |
| **K6** | Does LiNix learn per-backend group syntax (`pacman -S plasma`)? | rebuild |
| **K12** | Is a symlink at the default config path still supported? | rebuild |
| **N4** | Is `default/incoming` a statement or a preference key? | firewall |
| **N5** | What does removing a firewall rule restore? | firewall |
| **N6** | What if a config declares both `firewall:` lines and a `link:` to the ruleset? | firewall |
| **N7** | Does `watch` revert firewall drift unattended, or only report it? | firewall |
| **T3** | What does a missing hardware token look like — prompt, hang, or error? | secrets |
| **T4** | May an unattended `watch` tick decrypt with a touch-required key? | secrets |
| **U2** | Is a custom backend a full peer of a built-in (repos, orphans, dependencies)? | next |
| **U4** | Is `exec:` a licence to put a shell script where a backend belongs? | next |
| **U6** | Does this document mark its Linux-only guarantees (snapshots, rollback)? | next |
| **U7** | Is a health check per-package or per-sync? | next |
| **U8** | Is the removal preview a flag or a new verb? | next |
| **U10** | Where does a backend's bootstrap live — `priority` or the definition file? | next |
| **U11** | Does `watch` imply `--locked`? | next |
| **U12** | Does `try` reuse the Phase 6 images, or build from a base the config names? | next |
| **U13** | Does `@runs=always` exist? | next |
| **U15** | Where do LiNix-level event hooks live, and are they per-machine? | next |
| **U16** | May a custom backend's `binary` be an absolute path? | next |
| **U17** | Is `linix eval`'s JSON versioned from the first release? | next |
| **U18** | Are grouped backends with per-group priority worth building at all? | next |
| **U20** | Is a language server wanted, and may it be a second implementation? | next |
| **U21** | Is the exit-code table settled once, up front? | next |
| **U25** | One dotfiles tree, or several? | next |

### Built to the recommendation, never ruled — 0

**Empty, as of 2026-07-23.** All fifteen were put to the owner and ruled. The heading stays
because the category refills on its own: it is what happens whenever a recommendation gets
implemented before anyone rules on it.

### Answered — 42

| | question | feature |
|---|---|---|
| **N1** | Is a declared perimeter exclusive (undeclared rules are drift) or additive? | firewall |
| **N2** | What happens when the change would close the SSH session running it? | firewall |
| **N3** | Which adapters ship — and is one adapter enough to justify the backend at all? | firewall |
| **T1** | `backup_once` leaves a plaintext copy of the previous secret forever. | secrets |
| **T2** | Nothing stops `@target=` writing a plaintext secret back inside the git repo. | secrets |
| **T5** | Is the plaintext 0600 at creation, or chmod'd after? And on Windows? | secrets |
| **K17** | How does `setting:` reach a store nobody wrote an adapter for? | rebuild |
| **D2** | How is a format recognised from a filename? — built as extension match plus `binary`. | artifacts |
| **K5** | A level-3 reset with a config repo — built as refuse unless `--force`. | rebuild |
| **K11** | May the settings file hold more than the repo path — built as no, parser-enforced. | rebuild |
| **K14** | Does `rebuild` produce a git commit — built as no, and asserted by no test. | rebuild |
| **K16** | Does `clean-cache --all` need the guard — built as no; `reset` does. | rebuild |
| **U5** | Does `setting:` get a Windows registry and a macOS `defaults` adapter? | next |
| **D1** | What is "the release"? — built as latest non-draft, non-prerelease; `v` prefix tolerated. | artifacts |
| **D10** | Where the closed vocabulary lives — built as one table in `artifact/format.rs`. | artifacts |
| **W1** | The sigil — built as `$role`, never bare. | vars |
| **W6** | One `vars` file or a directory — built as one file; `vars.d/` ignored. | vars |
| **K7** | Which desktops `setting:` adapts to — built as GNOME only, KDE refused by name. | rebuild |
| **K7b** | The `setting:` key syntax — built as the statement form, not a backend prefix. | rebuild |
| **K8** | How a git-less LiNix announces it — built on the affected commands plus `doctor`. | rebuild |
| **K10** | `linix edit` and `linix path` — built as two commands. | rebuild |
| **K11b** | Where that file lives — built in the platform config dir. | rebuild |
| **K13** | Does `rebuild` appear in `schedules` — built as refused by name. | rebuild |
| **D3** | Two assets, same format — RULED 2026-07-20: shortest name, `@asset=` glob, `@asset=all`. | artifacts |
| **D4** | What installing a tarball does — RULED 2026-07-20: extract, find, shim, `@bin=`. | artifacts |
| **D6** | `@sha256` per machine — RULED 2026-07-20: checksums live in `locks/`, generated. | artifacts |
| **D7** | Does a `formats` block enable the backend — ADOPTED and built: yes. | artifacts |
| **D9** | A line's `formats` replaces the backend's — ADOPTED and built: replace, both seams. | artifacts |
| **W2** | Are values typed — RULED 2026-07-20: full JSON types, no coercion. Built. | vars |
| **W3** | Is a bare `$flag` a condition — ADOPTED and built: no, it is a parse error. | vars |
| **W4** | Where `vars` loads in resolution — built: once, before any `when`. | vars |
| **W5** | What `check` does with an unused variable — built: a note, from a static scan. | vars |
| **W7** | The undetectable variable — ANSWERED by the provider model: `env()` is the hatch. | vars |
| **W8** | Do variables work in `active` — built, including every path that edits your files. | vars |
| **W11** | Does `why` explain a variable — built as a gate chain. | vars |
| **W12** | A command to print resolved variables — built: `linix vars`. | vars |
| **W13** | Does changing a variable hit the guard — RULED 2026-07-20: yes, plus a run-level note. | vars |
| **W14** | Does `vars` belong in `diff` — built: yes, the line file and every provider file. | vars |
| **K1** | `rebuild`'s granularity — RULED 2026-07-20: batch per backend, foundation first. | rebuild |
| **K3** | A failed reinstall after a good removal — RULED 2026-07-20: snapshot and revert. | rebuild |
| **K9** | Is the backup command `bundle` — RULED 2026-07-22: yes, plus `restore DIR`. Built. | rebuild |
| **K15** | Does `plan` distinguish a rebuild's removals — built: `Reinstalled`, never `Removals`. | rebuild |

### Parked or closed — 2

| | question | feature |
|---|---|---|
| **D15** | `.flatpak`/`.snap` assets in a release — PARKED until D5 is answered. | artifacts |
| **D16** | libc variants (`gnu` vs `musl`) — CLOSED by D3's ruling. | artifacts |

---

# Open, and blocking

## D3b

**Status: OPEN — blocking.**

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

---

## D5

**Status: OPEN — blocking.**

**In the tree today:** Not reachable: `github.rs:225` says a `.deb` *"would have to be handed to `dpkg`"* — the backend does not install one today, which is why this is still askable.

**D5 — A `deb` installed by `github` — who owns it?** `dpkg -i` puts it in apt's database. Now
`apt` can upgrade it out from under LiNix, `linix check` may see it twice (once as a github
declaration, once as an apt-visible package), and the removal path has to know which tool to
call. **This is the "two of everything" failure at the package level**, and `purge-unmanaged`
(II.11) will have an opinion. *Recommendation:* the lock records the installing backend and
that backend owns removal; `check` must not double-count. Needs a real test against a real apt
box, not a mock.

---

## K2

**Status: OPEN — blocking.**

**In the tree today:** Nothing in `app/rebuild.rs` requires a scope.

**K2 — What is `rebuild`'s default scope?** `--all` on a bare `linix rebuild` is a very large
default for a command whose failure mode is an unbootable machine. *Recommendation:* require a
scope; a bare `rebuild` errors and lists the forms.

---

## K4

**Status: OPEN — blocking.**

**In the tree today:** No `clean_cache_on_remove` key exists anywhere in `src/`.

**K4 — Is `clean_cache_on_remove` per-package on every backend, or only where LiNix knows the
artifact?** LiNix knows the file for `github:`/`web:`/`appimage:` (it is in `locks/`). For apt
or pacman it needs a new per-backend capability. *Recommendation:* download-backends only,
documented as such in the key's own description — a preference that silently does nothing on
most backends is worse than a narrower one that is honest.

---

## U1

**Status: OPEN — blocking.**

**In the tree today:** **Still machine-local.** `backends/onboarder.rs:323` reads `safe_config_dir().join("custom_backends.toml")`, never the repo.

**U1 — Where does a custom backend definition live?** Today `~/.config/linix/custom_backends.
toml`, machine-local, never in git — so a repo that uses `paru:` breaks on every machine but
the one where somebody hand-wrote the file. *Recommendation:* the config repo, as a
first-class file beside `priority` and `schedules`, with the machine-local path kept **only**
if there is a case for a definition that must not travel — and if there is not, deleted in the
same change rather than left as a second place to look. **The consequence that makes this a
decision and not an obvious fix:** a definition in the repo is argv that a shared repo can
execute, which is II.12's supply-chain surface. It must inherit the hook trust model, not a
new one.

---

## U3

**Status: OPEN — blocking.**

**U3 — What does removing an `exec:` line mean?** Every other statement's removal undoes
something. A script has no inverse. *Recommendation:* an optional `@undo=` command; without it,
removing the line removes only the record, and `plan` says so in those words rather than
implying a revert that will not happen.

---

## U9

**Status: OPEN — blocking.**

**U9 — Do the ten status commands collapse into one?** *Recommendation:* yes, one `linix check`
with sections and narrowing flags; `heal` stays separate because it acts. Old names deleted in
the same change (P2), not aliased.

---

## U14

**Status: OPEN — blocking.**

**U14 — Is sharing wanted, and what makes a vendored module safe to run?** Vendoring puts
someone else's files in your repo, and once `exec:` exists those files can contain a verb. The
defence on offer is that it lands as a reviewable diff, which is a real defence and a weak one —
nobody reads the whole diff. *Recommendation:* decide the safety story before deciding the
feature. The candidates are: vendor everything but refuse to run an `exec:` that arrived this way
without an explicit per-module opt-in; or vendor modules but never backend definitions and never
`exec:`; or do not build it. **This is blocking because building the convenient version first
and the safety story afterwards is how supply-chain incidents are written.**

---

## U19

**Status: OPEN — blocking.**

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

---

## U22

**Status: OPEN — blocking.**

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

---

## U23

**Status: OPEN — blocking.**

**U23 — What happens to a destination that already holds the user's own file?** `link:`
answers this one file at a time with `backup_once`. A tree asks it forty times on the first
sync of a new machine, which is precisely the machine where the home directory is full of
files a distribution's defaults put there. Silently backing up forty files is not a preview,
and refusing on the first collision leaves the sync half-applied. *Recommendation:* the plan
lists every colliding destination **before** anything is written and the run is refused until
the user says which way; `--adopt-existing` (or whatever it ends up called) is the one-word
answer for "back them all up". This must be settled before the walker is written, because a
tree that half-links is worse than one that does not run.

---

## U24

**Status: OPEN — blocking.**

**U24 — Is a `.age` file in the tree a secret?** XII's decrypt mode is an option on a `link:`
line, and this statement has no per-file options by construction. Either the extension decides
(magic, and magic that silently writes plaintext), or encrypted files are simply not this
statement's job and stay on explicit `link:` lines. *Recommendation:* the second — **the tree
never decrypts.** T2 is already an open finding about plaintext landing in the config repo, and
a folder walker that decrypts by filename convention is the same failure with more surface.

---

## U26

**Status: OPEN — blocking.**

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

---

## T6

**Status: OPEN — blocking.**

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

---

# Open, not blocking

## T7

**Status: OPEN.**

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

---

## D8

**Status: OPEN.**

**D8 — `when` inside an options body.** II.2 says a declaration's body is options, so
`github { when family == debian { … } }` is not legal today, and VIII.2's example wraps the whole
`github` block in a `when` instead. That works but gets repetitive across four families.
*Recommendation:* keep it illegal. The wrap form is uglier and does not need a new grammar rule,
and a new block kind here is how the grammar starts growing exceptions.

---

## D11

**Status: OPEN.**

**D11 — The default order is detected, so a LiNix upgrade can change it.** A machine with no
`formats` line that installs a `tarball` today could install a `deb` after an upgrade. The lock
protects an existing install; a fresh `linix lock` or a new machine does not. *Recommendation:*
treat the default order as versioned and say so in the changelog when it moves — or accept the
churn explicitly. Not decided.

---

## D12

**Status: OPEN.**

**D12 — Network, rate limits, and offline.** Listing assets is a GitHub API call per repo.
Unauthenticated is 60/hour, which a repo with thirty `github:` lines exhausts on the second
`sync`. `LINIX_GITHUB_TOKEN` exists (II.1). *Recommendation:* resolve from `locks/github` without
any API call when the lock is present and the version is pinned; only `linix lock` and an
unpinned line hit the network. Needs deciding because it determines whether `sync` works on a
plane.

---

## D13

**Status: OPEN.**

**D13 — Changing a `channel` — refresh or reinstall?** `snap refresh --channel=edge` is not
`snap remove && snap install`, and moving `edge → stable` is usually a downgrade. **A downgrade
is a removal-shaped event and the guard should see it.** *Recommendation:* refresh where the
backend supports it, and route the downgrade case through the plan and the guard like any other
destructive change.

---

## D14

**Status: OPEN.**

**D14 — Does `why` explain the selection?** When `github:x/y` installs a `.tar.gz` on a machine
the user expected a `.deb` on, the answer lives in three places (line, `priority`, built-in
default) and `linix why` is the command that should say which one won. *Recommendation:* yes,
and it is a small amount of work only if the resolver keeps the reason rather than just the
result. Decide before the resolver is written, not after.

---

## D17

**Status: OPEN.**

**D17 — Regex lines.** What `github:re:…@formats=` means when one pattern spans repos with
different asset sets is unspecified. *Probably:* the list applies to each match independently and
a match with no legal asset is the VIII.2 error, named per repo. Not decided, and low urgency —
`github:re:` is rare in practice.

---

## W9

**Status: OPEN.**

**W9 — Interpolation outside `when`.** IX.5 says no. Record the boundary explicitly so the
answer is a decision rather than an omission, because the first `link:` request will arrive
quickly. *Recommendation:* stay narrow; reopen only with a use case that cannot be expressed as
two `when` arms.

---

## W10

**Status: OPEN.**

**W10 — Variables referencing variables.** `tier = $role-heavy`. Introduces ordering, cycles
(the same walk as `use` loops and `@requires` loops, II.7), and interpolation-inside-a-value,
which collides with W9. *Recommendation:* no, and the cycle machinery already existing is not a
reason to invite the problem.

---

## K6

**Status: OPEN.**

**In the tree today:** No group syntax anywhere in `src/`.

**K6 — Does LiNix learn per-backend group syntax** (`@kde-desktop`, `pacman -S plasma`)? It
would make one line install a desktop. It also means `backend:name` has a third meaning on some
backends and not others, which is the kind of unification VIII.1 refused. *Recommendation:* no
for now; a `when family` block listing each distro's name is explicit, works today, and reads.

---

## K12

**Status: OPEN.**

**In the tree today:** No symlink handling in `app/locate.rs` or `config/settings.rs`.

**K12 — Is a symlink still supported for "my LiNix files live in my dotfiles repo"?** With X.6's
settings file the symlink is no longer the only answer, but it costs nothing and some users will
reach for it first. *Recommendation:* yes, documented, with the settings file as the primary
mechanism.

---

## N4

**Status: OPEN.**

**N4 — Is `default/incoming` a `firewall:` statement or a preference key?** As a statement it
inherits `when` and the plan; as a key in `preferences.toml` it is machine-local and invisible
to git. *Recommendation:* a statement — the default policy is the most important line in a
firewall and belongs in the repo with the rest.

---

## N5

**Status: OPEN.**

**N5 — What does removal restore?** X.4 ruled that a removed `setting:` resets to the schema
default rather than to the value the user had before LiNix. *Recommendation:* the same answer,
for the same reason — restoring a per-rule prior state means keeping a per-rule store of it,
and "undeclared means the firewall's own default" is the shape every other statement's removal
already has. The cost is the same one X.4 recorded and it must be documented, not hidden.

---

## N6

**Status: OPEN.**

**N6 — What happens when a config declares both `firewall:` lines and a `link:` to the
ruleset file?** *Recommendation:* an error at resolve time naming both files and lines, in the
class of II.7 rule 5. Two owners of one perimeter is the two-of-everything failure, and it
should be caught before any command runs, not discovered at 2am.

---

## N7

**Status: OPEN.**

**N7 — Does `watch` revert firewall drift unattended, or only report it?** Everything else
`watch` reconciles is software; this reconciles reachability. *Recommendation:* report by
default, revert only under an explicit key, and never revert a rule that would trip N2.

---

## T3

**Status: OPEN.**

**T3 — What does a missing hardware token look like?** The plugin may prompt on a terminal
nobody is watching. *Recommendation:* a timeout, and a message naming the token and the
identity file rather than passing the plugin's own text through.

---

## T4

**Status: OPEN.**

**T4 — May an unattended `watch` tick decrypt?** A touch-required key turns a background
reconcile into a silent block. *Recommendation:* `watch` skips `@decrypt` lines whose identity
is a plugin stub and says so once, rather than hanging.

---

## U2

**Status: OPEN.**

**U2 — Is a custom backend a full peer of a built-in?** Repos, orphans, dependency queries and
`is_essential` are `ManagerConfig` fields `CustomBackendDef` does not expose.
*Recommendation:* expose them as optional keys, absent meaning *this backend cannot answer
that* — the `ManualListing` distinction already made for exactly this reason: "not configured"
must not be read as "the answer is none".

---

## U4

**Status: OPEN.**

**U4 — Is `exec:` a licence to put a shell script where a backend belongs?** The onboarder is
the better answer for anything that installs software, and `exec:` should not become the way
people avoid writing eight lines of TOML. *Recommendation:* document the boundary in the
readme, and treat repeated `exec:` lines that install things as a sign the onboarder needs a
missing field (U2), not as usage to encourage.

---

## U6

**Status: OPEN.**

**U6 — Does this document mark its Linux-only guarantees?** The pre-sync snapshot, `rebuild`'s
revert and `rollback`'s safety net all assume a provider that exists only on Linux
filesystems. *Recommendation:* yes, immediately and independently of whether VSS or APFS is
ever adapted — an unqualified promise that silently does not hold on two of three platforms is
P3's failure in prose form.

---

## U7

**Status: OPEN.**

**U7 — Is a health check per-package or per-sync?** Per-package answers "did *this* upgrade
break it" and is precise; per-sync catches the breakage a package cannot see (the boot, the
network). *Recommendation:* both, and they are not alternatives — `@health=` on a line, plus a
`health` list in `preferences.toml` for the machine-wide checks, with the same revert path.

---

## U8

**Status: OPEN.**

**U8 — Is the removal preview a flag or a verb?** *Recommendation:* a flag on the commands that
already compute it. A new verb for an existing computation is how this repo got two of
everything.

---

## U10

**Status: OPEN.**

**U10 — Where does a backend's bootstrap live?** In `priority`, beside the backend it obtains,
or in `custom_backends.toml`, beside the definition. *Recommendation:* `priority` — it is the
file that already decides which backends this machine uses, and a custom backend's definition is
about *how to drive* a manager, not *how to get* one. The two files stay one-question-each.

---

## U11

**Status: OPEN.**

**U11 — Does `watch` imply `--locked`?** An unattended reconcile that silently accepts a new
upstream version is the least supervised place for a version to change. *Recommendation:* yes by
default, overridable by a key — a machine reconciling itself at 3am should be converging to what
was decided, not to what was published.

---

## U12

**Status: OPEN.**

**U12 — Does `try` reuse the Phase 6 images, or build from a base the config names?** Reusing
them is nearly free and covers debian/alpine/arch today; a config-named base is what a user with
an unusual host actually needs. *Recommendation:* start with the Phase 6 images, and treat a
config-named base as the second step rather than the blocker — the value is in the rehearsal
existing at all.

---

## U13

**Status: OPEN.**

**U13 — Does `@runs=always` exist?** It is the escape hatch inside the escape hatch, and every
such key eventually becomes the default somebody copies. *Recommendation:* yes, but it prints
what it is doing on every sync — a line that runs unconditionally must be visible in the run it
made non-idempotent, or the next person debugging a slow sync has no thread to pull.

---

## U15

**Status: OPEN.**

**U15 — Where do LiNix-level event hooks live, and are they per-machine?** `preferences.toml` is
machine-local, so `after_sync` on the laptop is invisible to the desktop. That is right for a
notification hook and wrong for a policy one. *Recommendation:* `preferences.toml` first —
machine-local behaviour is the honest default for something that talks to *this* machine's
Slack — and revisit only when a real case wants a fleet-wide event.

---

## U16

**Status: OPEN.**

**U16 — Does the field split (XIII.12) allow an absolute path as `binary`?** A prefix that runs
`/opt/vendor/thing` is more useful and is also a definition that only works on one machine.
*Recommendation:* allow it, resolve `~`, and have `doctor` report a custom backend whose binary
is missing — the failure should be a named diagnosis, not an unknown-backend error three layers
away.

---

## U17

**Status: OPEN.**

**U17 — Is `linix eval`'s output versioned from the first release?** *Recommendation:* yes, a
top-level schema version, decided before anything consumes it. P2 says there is no legacy to
carry, and this is the one output that will acquire consumers LiNix cannot see.

---

## U18

**Status: OPEN.**

**U18 — Are grouped backends with per-group priority worth building at all?** The workaround —
write the prefix — already works, and what it costs is the portability a bare name exists for.
*Recommendation:* build it only with the invariant attached: **a bare name still resolves once
per machine**, and two modules that would resolve the same name through different groups is an
error naming both, which is II.7 rule 5 reached by a new road rather than a new rule. Without
that, this feature ships two `ripgrep` binaries fighting over `$PATH` — the failure
`app/conflicts.rs` already exists to catch.

---

## U20

**Status: OPEN.**

**U20 — Is a language server wanted, and is it allowed to be a second implementation?** *This is
the whole question, not the feature.* *Recommendation:* wanted, but only as a thin front end
over the same parser and resolver the binary uses — the moment it re-implements the grammar it
becomes the second implementation this rewrite exists to end, and it will disagree with the
first within a release. If it cannot be thin, do not build it.

---

## U21

**Status: OPEN.**

**In the tree today:** No exit-code table; `main.rs:33` is the only `process::exit` and it is `0`.

**U21 — Is the exit-code table settled once, up front?** *Recommendation:* yes — 0 converged, 1
LiNix failed, 2 differences found, 3 refused by the guard — decided in one place before
`--locked`, `try` and `check` are written. An exit code decided per command is a convention no
script can rely on, and the separation that matters is 3: a guard refusal is neither a failure
nor a difference.

---

## U25

**Status: OPEN.**

**U25 — One tree or several?** Several (`dotfiles:./dotfiles-work` under a `when`) composes
with the model already and costs nothing; one is simpler to explain. *Recommendation:* several,
because the statement takes a path anyway and forbidding a second one would be a rule with no
mechanism behind it — but two trees that would link the same destination is an error naming
both, which is II.7 rule 5 reached by a new road rather than a new rule.

---

# Answered

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
registry and `defaults` adapters are owed rather than optional. **This does not unblock the
work** — the registry adapter's first decision is `HKCU` or `HKLM`, which is **U19**, still open,
and whatever it picks becomes the convention macOS inherits.

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

**This answers U5: yes.** It does not unblock it — the registry adapter's first line is `HKCU` or
`HKLM`, which is **U19**, still open.

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

# Parked or closed

## D15

**Status: PARKED.**

**D15 — `.flatpak`/`.snap` assets in a GitHub release.** They exist. Adding them to the
vocabulary means `github` installing something `flatpak` then does not own — **D5's ownership
question, one layer worse.** Parked until D5 is answered.

---

## D16

**Status: PARKED.**

**D16 — libc variants** (`gnu` vs `musl`, both valid for this machine). A real ambiguity
`formats` cannot express, and a fourth axis is not worth opening until someone hits it. **D3's
answer probably resolves this one for free**, which is the argument for answering D3 properly
rather than expediently.

---

