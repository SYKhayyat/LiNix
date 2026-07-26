# The decision register — all 104, and which are answered

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
- **K1–K16, N1–N7, T1–T5, U1–U38.** *Blocking* means one thing in all four: **this cannot be
  built without an answer, because two reasonable implementations differ.** **U27–U38 are the
  extension-surface round (XIII.23–XIII.36): opening the snapshot/rollback layer, macOS/BSD
  filesystems, declared storage objects, custom health checks, the "as open as Lisp" set
  (parameterized modules U32, generated declarations U33, a REPL U34, user verbs U35), and the
  three more closed provider-lists the code review found — init systems (U36), notification
  channels (U37), and secret decryption (U38). None blocks. They share one mechanism (XIII.33: a
  declared provider, argv from a file, capability-by-declaration) and one line (XIII.32: open a
  surface only where the added thing is data LiNix cannot hear, never behaviour it cannot see).
  **Direction (owner, 2026-07-25):** for the *provider-list* surfaces — U27, U30, U31, U36, U37,
  U38 — the *whether* is settled (they are to be opened, reachable without a source change,
  XIII.33); what stays open on each is the *how* and the *safety order*. U33 (generated
  declarations) is explicitly **not** in that set — XIII.32 still refuses it.**

---

## Index

### Open, and blocking — 13

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

### Open, not blocking — 46

| | question | feature |
|---|---|---|
| **K18** | Should LiNix use a backend's own atomic swap where one exists (nix, rpm-ostree)? | rebuild |
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
| **U27** | Is the snapshot/rollback layer opened to a registry + config-driven providers? | next |
| **U28** | One snapshot provider or several, chosen by capability not list order? | next |
| **U29** | Is APFS the macOS safety net, and is its restore `Live` or not? | next |
| **U30** | Declare storage objects (zfs/lvm/btrfs) as a family — and does the guard cover destroying one? | next |
| **U31** | Should health checks be an open vocabulary — a user-declared check command? | next |
| **U32** | Do modules take parameters (the macro), and are parameter types checked? | next |
| **U33** | Are generated declarations — a config that runs a program to produce state — wanted at all? | next |
| **U34** | Is `linix repl` worth a second entry point, or is `eval \| jq` enough? | next |
| **U35** | May a user name a new verb, strictly as a composition of built-ins? | next |
| **U36** | Are init systems a declared-provider kind (s6/dinit/runit/Shepherd), or stays a closed enum? | next |
| **U37** | Are notification channels their own declared kind, or is an event hook the answer? | next |
| **U38** | Is secret decryption a declared-provider kind, and behind which T-series rulings? | next |

### Built to the recommendation, never ruled — 0

**Empty, as of 2026-07-23.** All fifteen were put to the owner and ruled. The heading stays
because the category refills on its own: it is what happens whenever a recommendation gets
implemented before anyone rules on it.

### Answered — 43

| | question | feature |
|---|---|---|
| **T6** | Must there be a way to opt out of `backup_once`, or bound how many pile up? | secrets |
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

**In the tree today:** Not reachable: `github.rs:225` says a `.deb` *"would have to be handed to `dpkg`"* — the backend does not install one today, which is why this is still askable.

**D5 — A `deb` installed by `github` — who owns it?** `dpkg -i` puts it in apt's database. Now
`apt` can upgrade it out from under LiNix, `linix check` may see it twice (once as a github
declaration, once as an apt-visible package), and the removal path has to know which tool to
call. **This is the "two of everything" failure at the package level**, and `purge-unmanaged`
(II.11) will have an opinion. *Recommendation:* the lock records the installing backend and
that backend owns removal; `check` must not double-count. Needs a real test against a real apt
box, not a mock.


**RULED (owner, 2026-07-24): the installing backend owns it.** When `github:`/`web:` installs a file (a `.deb` handed to `dpkg`, etc.), the lock records which backend installed it, and that backend owns removal, upgrade and dedup — `check` does not report it twice, and `purge-unmanaged` defers to the recorded installer. This is the existing per-backend ownership (every managed package carries its backend); the `github:`-installs-a-`.deb` capability itself is separate and unbuilt, but the ownership rule is settled for when it lands.
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

# Open, not blocking

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

---

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

**Status: OPEN — create ruled, destroy/guard open.** **Direction (owner, 2026-07-25): declaring a
storage object is opened (Phase 7p).** But the *remove* path destroys a filesystem — **do not ship
it until this rules what the guard owes it**; that half is genuinely open, and the family-vs-
separate-backends shape with it.

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

---

## U31

**Status: OPEN — whether ruled (build), how open.** **Direction (owner, 2026-07-25): open it —
build per XIII.33 and Phase 7p**, on the fail-loud constraint below. The how (exact schema) is
implementation, not a further ruling.

**U31 — Should health checks be an open vocabulary — a user-declared check command — rather than
a fixed set? (XIII.26.)** A health-checked upgrade (XIII.5) rolls back when the machine is
"unhealthy", but health is only what LiNix already knows how to test; a user whose service must
answer on a port, or whose config file must parse, cannot express it. A check is argv with exit
0 = healthy — the most check-shaped extension there is. *Recommendation:* open it, on the II.12
hook trust model (a check command is argv from a file, and the file may travel), and fail loud —
a check that cannot run is a failed check, not a passed one, or "healthy" quietly comes to mean
"the check was broken" (V's silent-wrongness). Not blocking: the built-in checks work; this is
the difference between a safety net LiNix designed and one the user can shape.

---

## U32

**Status: OPEN — not blocking.**

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

---

## U33

**Status: OPEN — not blocking.**

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

---

## U34

**Status: OPEN — not blocking.**

**U34 — Is `linix repl` worth a second entry point, or is `linix eval | jq` enough? (XIII.31.)** A
read-only prompt that resolves a name against *this* machine, evaluates a `when`, and expands a
`use workstation(gpu=nvidia)` — answering "what does this resolve to here" by trying it. It is
`eval` (XIII.15) with a cursor and must share the same parser and resolver, never a second
implementation (the U20 rule). *Recommendation:* low priority — real value for anyone authoring a
config, but `eval` already exposes the model, so this is ergonomics, not capability. Worth it only
if it stays a thin front end over the existing engine.

---

## U35

**Status: OPEN — not blocking.**

**U35 — May a user name a new verb, strictly as a composition of built-ins? (XIII.31.)** LiNix has
~sixty commands (XIII.8) and no way to add the sixty-first. A verb that *sequences* existing verbs
— `linix refresh` = `sync`, then `upgrade`, then the fleet report — is `defun` over the command
surface, and safe because it composes audited operations rather than producing new ones. **The
line:** a user verb sequences built-in verbs and nothing else; the moment it runs arbitrary argv it
is `exec:` wearing a command's clothes, which U4 already settled as no. *Recommendation:* build it
with that boundary hard-coded — composition only, no shell — so the safe 90% ships without
reopening the `exec:` trust question the dangerous 10% would.

---

## U36

**Status: OPEN — whether ruled (build), how open.** **Direction (owner, 2026-07-25): open it —
build the `[[init]]` kind per XIII.33 and Phase 7p** (the K17 rows move). The how is
implementation.

**U36 — Are init systems a declared-provider kind, or does the built-in enum stay closed? (XIII.34.)**
`backends/service.rs` is a fixed `enum InitSystem` (Systemd, OpenRC, SysVinit, launchd, Windows
`sc`) behind a hardcoded command table; s6, dinit, runit, GNU Shepherd and appliance inits are
unreachable, and a `service:` line on such a host has no branch to take. It is the snapshot vec's
problem in another file, and the **lowest-risk** surface to open — start/stop/enable are ordinary
reversible operations with no data to destroy. *Recommendation:* open it as a `[[init]]` block on
XIII.33's mechanism; it is the cleanest fit the mechanism has, and P7 is better served by "write
six lines" than by "unsupported". Not blocking — the five built-ins cover most machines.

---

## U37

**Status: OPEN — not blocking.**

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

---

## U38

**Status: OPEN — ruled in principle, GATED on the T-series.** **Direction (owner, 2026-07-25):
open it eventually — but do NOT build until the T-series settles how plaintext is handled.**
Opening this surface before that is decided hands an unaudited command the one thing LiNix guards.
Phase 7p lists it under STOP AND ASK for exactly this reason.

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

# Answered

## T6

**Status: ANSWERED — ruled 2026-07-23 (the blocking half; two sub-questions remain open below).**

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

