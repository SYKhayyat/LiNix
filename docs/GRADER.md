# YOU ARE THE GRADER

**Your job: independently verify LiNix and award a letter grade.** You need a machine, Docker,
and time. You are the adversary, not the author.

You are working in the LiNix repo at the path given to you. Read, in this order:

1. `docs/archive/READINESS-2026-07-27.md` — the prior assessment. **§0** is the defect index (`E1`–`E34`,
   each with a reproduction), **§5** the diagnosis, **§8.1** the rubric you will grade against.
2. `CLAUDE.md` and `docs/SPEC.md` — the repo's binding rules.
3. This document, in full.

### Your four obligations

1. **Re-run every reproduction in `READINESS` §3–§4 verbatim.** A defect is closed when **its
   original reproduction no longer reproduces** — never when a new test passes. Report each of
   `E1`–`E34` as *closed*, *still reproduces*, or *could not test* (+ why).
2. **Do not read the Builder's report until you have finished testing.** If a report exists
   (`docs/BUILDER.md` is what they worked from), it is a claim, not evidence. Reproduce each of
   its acceptance criteria yourself, from the repo. **A fix you cannot independently reproduce is
   not fixed.**
3. **Hunt beyond the list.** The 36 defects are a *sample* of the untested remainder, not an
   inventory — `READINESS` §5.1 explains why. **Finding nothing new means you did not look hard
   enough.** Sections 2–9 below are how to look.
4. **Award a grade** against `READINESS` §8.1, area by area, the way §8 does. State the bar you
   applied and what would move each area up one letter.

**Distrust every green light you are handed — including your own.** 1,359 unit tests passed,
clippy was clean, and four container lifecycles passed ~1,135 checks while every defect in
`READINESS` was live. Green is a floor, not a finding.

**Do not fix what you find** beyond the trivial, and never without writing the failing test
first. You are the check, not the second builder. A grader who patches becomes an author and
stops being able to grade.

---

---

## 0. Rules of engagement

1. **A test you did not watch fail is not a test.** Before asserting anything, break the thing
   deliberately and confirm the assertion goes red. This repo has shipped at least three
   assertions that could not fail (S36, S37, and the manifest-scrub in §2.1 below). Yours will
   be the fourth unless you check.
2. **Never assert on a claim; assert on the system.** "Every removal path calls the guard — A, B,
   C" is verified by enumerating removal paths *from the code*, never by reading A, B and C.
3. **Report in the negative.** Every section below ends with *what you could not test and why*.
   A report with no such list is incomplete, not clean.
4. **Do not fix what you find, unless it is trivial and you also write the failing test first.**
   Findings are more valuable than patches here; a patch without a diagnosis hides the family.
5. **Prefer disposable environments for anything destructive.** Containers, VMs, throwaway
   `LINIX_CONFIG_DIR`/`LINIX_DATA_DIR`. Never validate a removal path on a machine someone uses.
6. **Record every command and its real output.** Paste it. Summaries of test results are how
   this codebase acquired false ✅s.
7. **Check state at the right moment, or you will manufacture a finding.** Learned the hard way
   while producing the companion report: the native sweep uninstalls each package immediately
   after listing it, so inspecting the machine *after* the run shows empty directories that prove
   nothing. Two "blocker" findings evaporated on re-reading the log in order. **An assertion
   against a system in the wrong state is the auditor's version of an assertion that cannot
   fail** — and it fails toward false positives instead of false negatives, which is not better.
   Before believing your own result, ask what else changed between the event and your look.

---

## 1. Establish the baseline honestly

Before testing anything, measure what is actually covered. Do not accept the numbers in the
docs — regenerate them.

```bash
cargo build --all-targets && cargo test && cargo clippy --all-targets --all-features -- -D warnings
cargo fmt -- --check            # CI treats this as fatal; the local ship gate does not
git rev-list --left-right --count origin/main...HEAD    # unpushed = untested on Linux/macOS
```

Then produce a **coverage ledger** — a table, one row per registered backend, columns:

| backend | ever install/list/remove'd for real? | where | parser tested against *its own* real output? | argv verified against current upstream `--help`? |

Build it from the code (`src/backends/registry.rs`) and the harness ledgers, not from prose.
Expect the honest answer to be roughly *22 of 52 real, and far fewer with a parser fixture
captured from the tool itself.* **This ledger is your work queue for §3.**

---

## 2. Attack the test infrastructure first

You are auditing the instrument before you trust its readings. This is where the highest-value
findings are, because a defective check hides an unbounded number of defects.

### 2.1 Hunt assertions that cannot fail

The known instance, in **both** harnesses:

```sh
grep -v -F "linix-no-such-pkg-zzz" "$IMPERATIVE" > "$IMPERATIVE.tmp"
mv "$IMPERATIVE.tmp" "$IMPERATIVE"                       # deletes the line
nok "the unresolvable name is out of the manifest" \
    grep -q "linix-no-such-pkg-zzz" "$IMPERATIVE"        # then asserts it is gone
```

Find the rest. Techniques:

- **Mutation-test the harness.** Replace the `linix` binary with a stub that does nothing and
  exits 0. Run each harness. **Every check that still passes is a check that tests nothing.**
  This single experiment is worth more than reading the harness line by line.
- Then a stub that exits 1 on everything. Every check that still passes is inverted or unguarded.
- Grep for assertions preceded by a mutation of the thing being asserted: any `mv`, `rm`, `sed`,
  `grep -v`, `unmanage`, or `|| true` within ~5 lines above a `nok`/`ok`/`assert`.
- Look for `|| true`, `2>/dev/null`, and unquoted `$?` — each can convert a failure to a pass.
- `scripts/harness-logic-test.sh` lifts only `never_ran`, `assert_binary_gone`, `on_path`.
  Everything asserted *inline* is unexamined. Extend it, or explain why not.

### 2.2 Hunt the catch-all that launders failures

Worse than an assertion that cannot fail is a rule that turns *any* failure into a pass. The
native sweep has one:

```sh
soft "<backend>: install of <pkg> failed (ecosystem/network variance) — the checks after it did not run"
```

In one observed run it fired four times and **not once was it network variance**: `github` was
LiNix *correctly refusing* to overwrite a file it did not create, `helm` hit a real argv defect
(`plugin source does not support verification`), `luarocks` hit a real defect (no Lua version
pinned). Each time, the rest of that backend's lifecycle — list, PATH, remove, gone — was
skipped, and the run reported success.

**So coverage disappears precisely where the product is broken.** Find every catch-all of this
shape, in both harnesses. Then fix the category error: LiNix already computes
`Retryability::{Transient, Permanent, Unknown}` and has a distinct `Refused` exit. A harness
should soften only on `Transient`, fail hard on `Permanent`, and score `Refused` as its own
outcome — never collapse "the network flaked", "we have a bug", and "we refused on purpose and
were right" into one word.

**Test for it directly:** point a backend at a package that cannot exist, and assert the harness
reports a *failure*, not a soft pass. Then point it at something LiNix should refuse, and assert
it reports a *refusal*.

### 2.2b Hunt the checker that verifies the arithmetic and not the claim

A subtler relative of §2.2. A check can be **correct, running, and green every day** while the
thing it guards is wrong — because it verifies a number and the number was never the claim.

The known instance: `scripts/decision-count.sh` counts the decision register and fails CI if any
documented total disagrees. It worked perfectly. Meanwhile `D15` said *"parked until D5 is
answered"*, D5 was answered and built five days later, and the entry sat under a **met condition**
for a week — filed under the status that means *needs nothing from you*. The totals were right
the entire time the register was wrong. The same script also grepped only the shouted
`N ANSWERED` form, so `SPEC.md`'s lower-case *"125 answered, 2 parked"* was invisible to it and
went a week stale while the run printed `ok`.

So, for every checker, script and CI gate in this repo, ask two questions:

1. **What does it actually assert, as opposed to what its name says?** A count check asserts
   arithmetic. It does not assert that a status is still true, that a condition is still unmet,
   that a citation still points at the right line, or that a cross-reference resolves.
2. **What claim does the guarded document make that nothing checks?** Statuses with conditions,
   `waits on`/`until`/`blocked by` clauses, `file:line` citations, "see §N" pointers, and any
   sentence of the form *"X is true because Y has not happened yet."* Each is a claim with an
   expiry and no alarm.

Then test it the §0.1 way: make the guarded document wrong in the *claim* rather than the
*number* and confirm the checker still passes. If it does, that is a finding, and it is worth
more than a broken test — a green checker is trusted, and trusted is how a week goes by.

### 2.3 Verify the exemption lists

Both harnesses exempt subcommands by name. `undo` is exempted and **does not exist** (renamed to
`snapshot`/`rollback`). The audit checks that every *invoked* command exists; it does not check
that every *exempted* one does. Assert both. Then ask the harder question: does each exemption
still have a reason, or is it inherited?

### 2.4 Make the local gate match CI

`scripts/release-check.ps1` and `release-check.sh` call themselves the ship gate and rate
`cargo fmt --check` *informational*; `.github/workflows/ci.yml` rates it fatal. Any gate weaker
than CI is not a gate. Diff every check in the local gates against every step in CI and report
the asymmetries in both directions.

### 2.5 Test the harness's oracles

`assert_binary_gone` once passed by asking the shell's hash table (`command -v`). What else does
the harness ask a question whose answer can come from a cache, a shim, or a different tool's copy
of the same binary? Construct the adversarial case for each oracle: another manager's copy of the
same program earlier on `PATH`, a stale shim, a shell function shadowing the binary.

---

## 3. Test the backends nobody has run

This is the largest and most mechanical body of work, and it is where most latent bugs live.
Work the ledger from §1.

### 3.1 The container fleet

For every backend that can run in Linux, add it to a disposable image and drive a **real**
lifecycle: `install → list → binary-on-PATH → remove → gone`. Not a plan-smoke. A plan-smoke
proves the argv is *constructed*; `pixi global upgrade-all` passes one and does not exist.

Practical approach: one thin image per ecosystem (`node`, `python`, `go`, `rust`, `haskell`,
`ocaml`, `elixir`, `php`, `lua`, `nim`, `dart`, `k8s`, `conda`) rather than one giant `tools`
image that takes tens of minutes and therefore runs nightly at best.

### 3.2 Verify every argv against current upstream

For each backend, run the real tool's `--help` (or subcommand help) and diff it against the
argv LiNix builds in `src/backends/registry.rs` and the dedicated backend modules. Upstream CLIs
drift; `pixi global upgrade-all` was removed and nothing noticed.

**Make this a permanent, self-updating check**, not a one-off sweep: a nightly job that, for each
installed manager, asserts every subcommand LiNix will invoke still appears in that manager's
help output. This is the single highest-leverage test in this document — it converts silent
upstream drift into a named failure.

### 3.3 Capture real fixtures for every parser

The rule to enforce: **a parser is tested against output captured from the tool it parses, and
from no other tool.**

`names_only` currently serves opam, spack, pixi and emerge, and is tested with one spack fixture.
The test passes; pixi's real output is a detail record and the parser emits `-`, `...` and bare
version numbers as package names. Same class: choco parses its own `5 packages found.` summary
into a package named `5`.

Do this: for each backend, run its real `list`, `search`, and `info` in a container; save stdout
verbatim to `tests/fixtures/<backend>/<verb>.txt`; assert the parser's output against a
hand-checked expectation. **Include the empty case, the one-result case, the not-found case, and
the error case** — three of the four are where the junk rows come from.

### 3.4 Probe the probes

`psresource::is_available` checks that **PowerShell** exists, not that **PSResourceGet** — the
module supplying its cmdlets — is importable. So it reports `[READY]` and then fails every
command. `krew` does it correctly, checking both `kubectl` and `kubectl-krew`.

Audit every `is_available` and `check_health` against one question: *does this probe the thing
that actually has to work?* For each, construct the environment where the probe passes and the
backend fails, and assert LiNix says so.

### 3.5 Change every option after the install — the sweep no lifecycle performs

**A lifecycle is `install → list → remove`, and by construction it never edits a declaration.**
So an option that is read once when the install argv is built, and never again, passes every
lifecycle, every plan-smoke and every unit test — for ever. This is not hypothetical: on
2026-07-31 five of them were found this way in one afternoon (`Q19`, `Q20`), none reported by a
user, all of them shipped, all of them green.

The defect has a signature you can grep for and then must confirm by running: **an option
consumed inside an `install` path and referenced nowhere in the planner's drift check.** But do
not grade it by reading. Drive it:

For **every** key in `config/grammar/statement.rs::PACKAGE_OPTION_KEYS` and every table in
`backends/capability.rs`, on a backend that reads it:

1. Declare it with value A. Sync. Confirm the machine has A.
2. **Edit the line to value B and sync again.** Confirm the machine has B — by asking the tool,
   never by asking LiNix.
3. Sync a third time unchanged. Confirm **nothing happens.** This is the other half and it is
   the half a single edit cannot see: a comparison that gets units or formatting wrong reports a
   change on *every* sync, for ever, and a test that syncs once passes it.
4. Delete the option and sync. Whatever happens must be a decision someone made — reverting to a
   default and leaving it alone are both defensible, silently doing one while the docs say the
   other is not.

Three outcomes are legitimate for step 2: the machine changes, or the line is refused with a
reason and a way out, or the docs say the option is create-only *and mean it*. **"Nothing
happens" is not one of them**, and neither is "it changed but `sync` also reports a change next
time".

**Score the omission, not just the failure.** An option nobody can edit meaningfully is a feature
that exists in the docs and not on the machine. Report every option you could not drive this way
and why — that list is the finding.

---

## 4. Test as a human, not as a pipe

Everything in CI runs with pipes on every handle. A defect that emptied the output of every
command LiNix runs lived in exactly that gap for weeks. `tests/pty_tests.rs` now covers the
basic case and is Linux-only. Go further.

- **Drive a real pty** (`script -qec`, `expect`, `ptyprocess`, or a pty crate) and assert that
  *what LiNix parsed equals what LiNix printed*. Do it for every read command, not just `list`.
- **Test the four handle combinations** for a mutation: stdin tty/pipe × stderr tty/pipe. The
  mirroring path in `RawExecutor::execute` only engages when stderr is a terminal.
- **Test `sudo` for real** in a container with a password-protected user. Does the prompt reach
  the screen? Does the password reach sudo? Does a wrong password fail loudly? Does the keepalive
  work? This is untested and it is on every privileged path.
- **Test the pipe-closed path**: `linix search x | head -1`. There is a panic hook for EPIPE
  under `panic = "abort"`; confirm it, and confirm it did not swallow real panics.
- **Test a narrow terminal (`COLUMNS=40`), a dumb terminal (`TERM=dumb`), and no-color
  (`NO_COLOR=1`).** Tables are printed with fixed `{:<32}` widths; check they degrade sanely.
- **Read the output aloud as a new user.** One failed install currently prints the same sentence
  three times — once as a `WARN` naming `linix::core::journal` and a 32-hex WAL id, once as an
  `ERROR` naming `linix::core::transaction` and a "Node", and once as `Error:`. Flag every place
  internal vocabulary leaks: WAL, Node, DAG, backend-capability, module paths, UUIDs.

**A rule worth adopting:** every user-visible failure should name (a) what failed in the user's
words, (b) the file or command they can act on, and (c) exactly one place to look. Test for it.

---

## 5. Test the paths that destroy things

These are argv-tested and unrun: btrfs/zfs/lvm snapshot restore, `dpkg -i`/`rpm -U` handoff, U30
storage removal, and the whole rollback family. **They need disposable VMs, not containers** —
several require a real filesystem and some may require a reboot.

- **Build a loopback btrfs/zfs filesystem in a VM**, take a snapshot, mutate, restore, and assert
  the mutation is gone. Same for lvm. This is the only way these get tested.
- **Kill LiNix mid-transaction** — `SIGKILL` (not `SIGTERM`) between the install of package 3 and
  package 4 — then run `heal` and assert the machine and the journal agree. Do it at every step
  boundary, in a loop. This is the WAL's entire reason to exist and nothing tests it under a real
  crash.
- **Fail a compensating action.** Make the reinstall during rollback fail (remove the package
  from the repo mid-run) and assert LiNix reports the package as *left at the new version* by
  name, and returns an error, rather than reporting a clean rollback.
- **Prove the guard from the code, not the list.** Enumerate every call site that can reach a
  backend's `remove`/`purge`, from the code. For each, write a test that it refuses a protected
  package. Recovery paths (`heal`, rollback, expiry sweep, shell exit) need this *most* — nobody
  is watching when they run. The eighth removal path was found only by starting the binary.
- **Test `--dry-run` performs nothing.** Run every mutating command under `--dry-run` against a
  fully-instrumented fake backend and assert **zero** mutating calls reached it. The flagship bug
  in this repo's history was a `--dry-run` that performed the removal.
- **Test that `-y` does not mean "purge the system".** `--allow-mass-removal` is meant to be the
  only override. Assert `-y` alone cannot exceed `max_removals`, on every scope in `GuardScope`.

---

## 6. Test the model, not just the commands

The declarative core deserves property-based and model-based testing, which it currently has none
of.

- **Round-trip properties.** For any manifest `M`: `sync(M)` then `sync(M)` again ⇒ the second is
  a no-op. `install(p)` then `uninstall(p)` ⇒ the manifest is byte-identical to before. `bundle`
  then `restore` ⇒ identical resolved state. `export` then re-import ⇒ same package set.
- **Fuzz the grammar** (`config/grammar/`). Generate manifests: absurd nesting of `when`, unicode
  and RTL package names, 10k-line modules, cyclic `use`, a module that uses itself, CRLF, a BOM,
  no trailing newline, embedded NULs, names that look like flags (`--force`), names containing
  `:` and `@`. **Nothing should panic; everything should be a named refusal.** Compare against the
  §7 invariant: the parser is the one place a hostile string enters.
- **Model-based testing.** Write a tiny reference model of the desired state in a few hundred
  lines, generate random command sequences, and assert LiNix's `eval` output matches the model
  after each step. Divergence is a bug in one of them, and finding out which is the point.
- **Differential testing across backends.** The same declarative operation through apt, brew,
  scoop and pacman should produce the same *shape* of result. Where it does not, either a parser
  or a capability declaration is wrong.
- **Concurrency.** Two `linix sync` runs at once; one holding the data lock while the other
  starts; `SIGKILL` the lock holder and assert the next run recovers rather than waits forever.

---

## 7. Adversarial and security passes

The core is in decent shape here — argv everywhere, no `sh -c`, archive extraction rejects `..`,
a hook ledger — so look where that discipline could have gaps.

- **The `-Command` string builders.** `psresource` interpolates names into a PowerShell script
  and defends with an allowlist; `windows_shim_wrap` builds a `-Command` string for `.ps1` shims.
  Fuzz both with names containing `'`, `` ` ``, `$(...)`, `;`, newlines, and unicode
  look-alikes. Assert refusal, never execution.
- **The `.cmd` shim path swallows exit codes.** `cmd /C scoop.cmd install <bad>` returns 0. Any
  backend whose failure detection then rests on one marker string is one upstream wording change
  away from reporting failures as success. Enumerate every backend whose `ExitPolicy` carries
  fewer than, say, three failure markers, and test a failure mode outside them.
- **`@health=` commands** ride in with pulled config; U31's ruling that they pass the II.12 hook
  ledger is unbuilt. Write the test that a pulled config's health command does **not** run
  unapproved, and watch it fail.
- **Path handling.** Package names and `@target` paths that are absolute, UNC (`\\?\`), symlinks
  out of `$HOME`, reserved Windows names (`CON`, `NUL`, `AUX`), trailing dots and spaces,
  >260-char paths.
- **Untrusted input from managers.** A package whose *name in the manager's output* contains a
  newline, ANSI escapes, or a tab. Does it corrupt the parse, the table, or the manifest?

---

## 8. Cross-platform

- **macOS has never been run**, only compiled. The `macos-native` job has never gone green. Get a
  Mac or a runner and drive a real brew lifecycle. Expect the S34 class: assertions that quietly
  encode Windows or Linux path semantics.
- **Run the full unit suite on all three platforms every time.** Two of this repo's bugs (S33,
  S34) were invisible on the developer's box and appeared the moment CI saw them.
- **Test with no global git identity, no `$HOME`, no `$EDITOR`, a read-only config dir, and a
  full disk.** S33 was exactly "passes wherever a global git identity exists".
- **Windows specifically:** `PATHEXT` without `.PS1` (the default) changes which shim `which`
  resolves and therefore whether exit codes survive. Test with and without `.PS1` in `PATHEXT`,
  and with `pwsh` present and absent.

---

## 9. Measure the human path end to end

Script a **new user's first hour** and assert on it, because none of it is covered today:

1. Fresh machine, no config. `linix sync` — does it name `linix init`? *(Today: no.)*
2. `linix init` — does it create everything its `--help` promises? *(Today: no starter module.)*
3. `linix check health` on a normal machine — does it call 23 absent managers *critical*?
   *(Today: yes.)* Does the rollup agree with the detail view? *(Today: no.)*
4. `linix install <typo>` — is the config still usable afterwards? *(Today: no.)*
5. `linix info <a real package>` — how long, and is it right? *(Today: 98 seconds, and wrong.)*
6. Every error message: does it name a file or a command the user can act on?

**Then time everything.** Set a budget per command class — read-only commands under 2s, a
qualified `info` under 5s — and fail the build when it regresses. Nothing measures latency today,
which is how a 98-second `info` shipped.

---

## 9b. The four rulings of 2026-07-31, and how to catch them lying

Each shipped with a claim that a name a manager prints is a name LiNix accepts. Each is one
command, and the *interesting* half of every one of them is the control beside it — a rule that
admits everything is not a rule.

| ruling | the check | the control that must still refuse |
|---|---|---|
| `Q22` BOM | `printf '\\xef\\xbb\\xbfcargo:ripgrep\\n' > $LINIX_CONFIG_DIR/modules/starter.txt; linix eval` | a U+FEFF *mid-name* is still refused, and the refusal names `<U+FEFF>` rather than drawing it |
| `Q23` scoped `@` | `linix eval` over `npm:@angular/cli@version=17.3.0` — name `@angular/cli`, version `17.3.0` | `cargo:ripgrep@version=15.2.0` still splits at its first `@` |
| winget identifiers | `winget:ARP\Machine\X64\{GUID}` resolves | the same name under `cargo` is refused; `..`, `;`, backtick, `$`, `|` still refused for winget |
| `G-2` backslash | `winget:a\b` is a package | `apt:jq \ apt:vim` is still set math |

**Then attack the seam they share.** A name is admitted by a **grammar** and a **validator**, and
this session shipped a fix that taught one and not the other: `adopt` wrote 340 winget rows the
grammar accepted, the validator then refused them, and every later command failed to parse
`adopted.txt` — a wedged model, E1's class, found only by running the native sweep. So the
question to ask of any name rule is not "does it parse" but **"does it survive `adopt` →
`check`"**, which is two different pieces of code agreeing.

## 9c. Coverage claims made this session — verify by reading a build log, not a table

- **`nix` is installed in the `tools` image.** It was not, for months: the nixos.org script
  refuses to run as root and `|| echo "SKIP nix install"` swallowed it, while the ledger called
  `nix` a backend with *no path to a real lifecycle anywhere*. Check the image's own assertion
  (`RUN nix --version && nix profile list`) and the `/etc/linix-image-managers` manifest the
  sweep now reads — a manager that failed to install is reported MISSING, not impossible.
- **Five exemptions are now conditional on `disposable_host`** (`CI` set): `pip`, `vscode`,
  `emacs`, `mise`, `asdf`. On a developer's box they still skip, and the reason still says so.
  **The check worth making: does the CI leg actually lifecycle them, or does it skip for a second
  reason nobody noticed?** That is how `nix` hid.
- **`web:` and `appimage` have pinned canaries.** A moving artifact would make a red run mean two
  things; both are pinned to a tag. If either is flaky in your run, say which and why — a canary
  that is unstable is worse than one that is absent.
- **Still exempt, and now written as PRICES rather than walls:** `stack` (a Haskell package builds
  from source whatever is baked into the image) and `flatpak` (the smallest runtime is multi-GB).
  Read those two sentences against `Q17` and say whether they still hold; that is exactly the
  re-derivation nobody performed for `nix`.

## 10. What to deliver

1. **The coverage ledger from §1**, filled in. This is the most valuable artifact; it converts
   "how ready are we" from an argument into a number.
2. **A findings list**, each with: a reproduction command and its real output, the file:line, the
   *family* (which sibling sites you checked, including the ones you cleared and why), and a
   severity tied to user impact rather than to how surprising it was.
3. **The failing tests**, committed and red, for everything you found. Red is the deliverable.
4. **The negative report** — every area you could not test, and what it would take. Name the
   hardware, the credentials, the time.
5. **A one-page verdict** that does not use the word "green". Say what a user can rely on, what
   they cannot, and what you did not look at.

---

## The single idea behind all of this

This codebase's recurring failure is not bad code. It is **checks that examine the wrong thing
and then report success** — a probe that tests PowerShell instead of the module, a health message
that names the backend instead of the binary, a harness that deletes a line and then asserts the
line is gone, a ship gate that rates fatal things informational, and a plan-smoke that proves an
argv was built rather than that it works.

So for every check you write, and every check you inherit, ask the same question:

> **If the thing this is supposed to catch were happening right now, would this go red?**

Prove it by making it happen. That is the whole job.
