# Taking over

**For the person who now owns this and did not write it.** `SPEC.md` is the map,
`ARCHITECTURE.md` is where things live, `DEVELOPMENT.md` is the commands. This is the one you
open when something has gone red and you need to know whether it matters.

It assumes you would rather not read Rust. Most of what follows is reading, deciding, and
occasionally pasting one line into a file.

---

## The five-minute version

1. **Look at the board, not at the code.** `https://github.com/SYKhayyat/Shall/actions`
2. **Find out which JOB failed, not which run.** A run says "failure" and tells you nothing; the
   job names the subsystem.
3. **Ask the three questions below, in order.** Most red boards are answered by the first.

| question | how to tell | what to do |
|---|---|---|
| Is it a **dependabot PR**? | the run's branch is `dependabot/...` | Nothing. A bump that breaks the build is the bump's problem. Close the PR or leave it. |
| Is it **only the nightly**, with push runs green? | `event: schedule` or `workflow_dispatch` red, `push` green | Not urgent. Nightly covers slow images and mutation testing; the code is fine. |
| Did **you** just push? | the red run's `head_sha` is your commit | It is yours. Read the failing step's log. |

**A run marked `cancelled` is neither green nor red.** It was killed, almost always because
something else was pushed or dispatched while it ran. It proved nothing. Do not read it as pass.

---

## Reading CI when `gh` is not installed

It is not installed here, and on this machine GitHub is reachable only from inside WSL.

```sh
# the token, from the credential helper
TOK=$(printf 'protocol=https\nhost=github.com\n\n' | git credential fill | sed -n 's/^password=//p')

# recent runs
curl -sS -H "Authorization: Bearer $TOK" \
  "https://api.github.com/repos/SYKhayyat/Shall/actions/runs?per_page=10"

# the JOBS of one run — this is the useful one
curl -sS -H "Authorization: Bearer $TOK" \
  "https://api.github.com/repos/SYKhayyat/Shall/actions/runs/<RUN_ID>/jobs?per_page=100"

# a failing job's log
curl -sSL -H "Authorization: Bearer $TOK" \
  "https://api.github.com/repos/SYKhayyat/Shall/actions/jobs/<JOB_ID>/logs"
```

Two things that waste an hour if nobody tells you:

* **Query the jobs, not the run.** A run's own conclusion appears long after its jobs have
  finished. The jobs endpoint answers while it is still going.
* **`head_sha` needs the full forty characters.** A short sha silently returns an empty list,
  which reads exactly like "no runs" rather than "you asked wrong".

---

## The failures you will actually see

### "One backend's install failed" in an integration job

Read the classification Shall printed. It is on the line above, and it is the whole answer:

* **`shall-failure-class: transient`** or **`exhausted`** — the ecosystem, not the code. A
  registry rate-limited, a mirror dropped, a signing key rotated. **Nothing to fix in Shall.**
* **`shall-failure-class: permanent`** — the request is wrong. A name that does not exist, an
  unsignable source.
* **`shall-failure-class: unknown`** — *nobody looked*. This is the interesting one: it means
  that manager has no exit policy for what it just said. See "adding a marker" below.

### "coverage: N real lifecycle(s), below the recorded M"

A backend that used to complete a real install→remove round trip did not. The run prints which
ones it could not measure. If the cause is an ecosystem that is down and not the code, the run
prints a line to paste into `scripts/lifecycle-floor.txt`, with today's date:

```
drift container-linux-tools-local cabal 2026-08-21
```

Paste it, commit it, done. **It does not expire** — every later run just reports how many days it
has stood. Delete the line when the ecosystem recovers and the backend is measured again.

### A mutation shard is red

`cargo mutants` changed one line of the source and **no test noticed**. The log names it:

```
MISSED  src/core/transaction.rs:147:13: delete field batch_recovery from ...
```

This is not a bug in the program — it is a hole in the tests. It is safe to leave for a while,
and it is the highest-signal thing in the whole board when you do get to it: every mutation
survivor found on 2026-08-21 was a real, unasserted branch.

### A flake

Some jobs touch the network. An install that failed once and worked on retry is reported as
`soft`, not a failure. If a job is red and the log shows the operation succeeding on a second
attempt, re-run the job before believing it.

---

## When to reach for Docker

CI tells you **that** something is red. A container tells you **why**, and it is much faster than
waiting for a nightly:

```sh
./scripts/docker-restore.sh --check      # what images exist
./scripts/docker-restore.sh              # rebuild the missing ones (slow: hours for all)

# run one image's harness against the current scripts
docker run --rm \
  -v "$PWD/docker/integration/run-in-container.sh:/src/docker/integration/run-in-container.sh:ro" \
  -v "$PWD/scripts/lifecycle-floor.txt:/src/scripts/lifecycle-floor.txt:ro" \
  -e SHALL_IT_IMAGE=tools shall-it-tools apt jq
```

Docker lives **inside WSL** on this machine — there is no Docker Desktop, and `docker` missing
from the Windows PATH means nothing. `scripts/docker-restore.sh` re-execs itself into the distro
for you.

**If you change anything in `docker/integration/run-in-container.sh` or
`scripts/integration-windows.sh`, run one image before pushing.** Those scripts are not covered by
the Rust suite. On 2026-08-21 a one-line change to them broke twelve integration jobs, and both
`shellcheck` and `harness-logic-test.sh` passed it — the harness itself catches it in ninety
seconds.

---

## Making a change without breaking the board

The full chain, in order. Each step is cheap except the third:

```sh
cargo build --all-targets
cargo test --no-fail-fast      # ~15 min. --no-fail-fast always: without it one failing
                               # target abandons the rest and you learn about one defect
cargo clippy --all-targets     # must be zero; CI treats warnings as fatal
cargo fmt -- --check           # the one gate a change with no logic in it can break
sh scripts/unix-check.sh       # compiles the 45 cfg(unix) blocks Windows cannot see
```

Then, only if you touched a shell script:

```sh
sh scripts/harness-logic-test.sh                      # the harnesses' own predicates
docker run --rm -v "$PWD:/mnt" -w /mnt \
  koalaman/shellcheck:stable -S warning scripts/*.sh docker/**/*.sh
```

**Three ways to waste an afternoon, all learned the hard way:**

* **Never run two `cargo` commands at once.** They contend on the target lock and produce
  `LNK1104` / `failed to remove shall.exe`, which look like real failures and are not.
* **Never push twice in a row.** The second push cancels the first run. Push once, then watch.
  The same is true of dispatching a nightly while one is running.
* **Verify an edit by reading the file back**, not by whatever wrote it reporting success.

---

## Adding a marker, which is the most likely thing you will need to do

When a manager answers `shall-failure-class: unknown`, it means `src/core/exit_policy.rs` has no
entry for what it said. Adding one is mechanical, and the method matters more than the code —
**ask the manager three questions, not one:**

1. install a name that **does not exist** → the `absent_markers` candidate;
2. the same name with **`--network none`** → the `transient_markers`;
3. a package that **does** exist, at an impossible version → **the one everybody forgets**.

If (1) and (3) print the *same sentence*, that manager **cannot have an absent marker** — believing
it would withdraw the declaration for a real package over a wrong version pin. That is why
`conda` and `luarocks` are deliberately uncovered, and it is exactly how a marker that had shipped
for `pipx` was found to be wrong.

A backend listed in `capability::CANNOT_PIN_VERSION` has no version axis at all, so question 3
does not apply to it.

`tests/absent_marker_coverage_tests.rs` holds the list of what is still uncovered, with the
reason for each, and will tell you when a backend has earned its way off it.

---

## What is a decision and what is a bug

`docs/spec/decisions.md` is the register: 226 entries, none open. **A question with an ID in it
is the owner's to answer, not the implementer's.** If you find yourself deciding something that
changes what a user sees, it belongs there first — write the entry, then build it, in the same
commit.

Everything in `docs/spec/target-state.md` (Part II) has a matching entry in
`docs/spec/why.md` (Part V) explaining the bug it exists to prevent. **Do not change a Part II
rule without reading its Part V entry** — most of them look arbitrary and are not.

`docs/spec/bugs.md` is the graveyard: what was killed, and what is carried forward.

---

## Things that are true and surprising

* **A profile's name is its filename.** `active` holds `Work.txt`, not `Work`.
* **`scripts/` is excluded from the Docker build context**, so any gate living there reaches a
  container only by being mounted. A gate that is not mounted is a gate not in force — this repo
  has already shipped a ratchet that was mounted on one leg of five and green on all of them.
* **The integration suite is one binary.** A new file in `tests/` does not run until it is a
  `mod` line in `tests/main.rs`; `every_test_file_is_in_the_suite` fails when they disagree.
* **The pre-commit hook is not automatic.** `git config core.hooksPath .githooks`, once per
  clone. It checks formatting only, deliberately — a hook that takes minutes gets bypassed.
* **Windows drops the executable bit.** A new script commits `100644` and CI's `./script.sh` dies
  `126`. `git update-index --chmod=+x <file>` before committing.

---

## If you want to stop maintaining it

Nothing needs doing. The nightly runs on a schedule, the repo is public so Actions is free, and
the drift register no longer expires — an ecosystem that breaks upstream turns the board red
once, prints the line that silences it, and never nags again.

The one thing that rots quietly is **absent-marker coverage**: 25 of 49 backends can currently
tell Shall a package name does not exist. The other 24 leave a mistyped name in the config, where
it fails every later command. That is the backlog, and the method above is the whole of it.
