# The 2026-08-18 audit, answered

**What this is.** Every finding in `docs/AUDIT-2026-08-18.md` (F1-F16), fixed at its root and
with its family, in one change. Read that document first: it states each defect and the
reasoning. This one records what was done about it, what the sweep found beyond the report, and
the one finding whose premise did not survive checking.

## The findings

| # | What was done |
|---|---|
| F1 | `bin_destination`'s four hand-written checks became one `Component::Normal`-and-nothing-else match, plus a Windows reserved-device-name refusal. Tests extended to drive-relative (`C:evil`), UNC, `.`, absolute, and `NUL`/`CON`/`COM1`/`NUL.txt`. |
| F2 | `parse_duration` splits by `len_utf8`, not by one byte, and multiplies with `checked_mul`; `now + seconds` is `checked_add`. Tests cover multi-byte units and both overflow directions. The two siblings (`dated.rs`, `schedule.rs`) were checked and are correct. |
| F3 | The three byte-identical `loaded()` readers now go through one `ledger::load_json_records`: absent is empty, unparseable is an error. `owned_system_packages` became fallible for the same reason - reporting "none" for an unreadable record tells `purge-undeclared` those packages are drift. |
| F4 | `write_capped_to` wraps the whole stream so *every* error exit removes the partial file, not only the cap refusal; a body shorter than its declared `Content-Length` is now an error. `appimage:` downloads to a `.shall-part` sibling and renames after verification, as `web:` and `github:` already did. |
| F5 | The PowerShell shim wraps the invocation in `try`/`catch` and exits 1 on a terminating failure, so a missing command, a missing `.ps1` or a thrown exception is no longer exit 0. Deliberately no `$ErrorActionPreference = 'Stop'`: scoop emits non-terminating errors on installs that succeed. Measured against the installed scoop - same output, same exit codes, plus the failures the old form reported as success. |
| F6 | `StateRegistry::packages` is a `BTreeMap<(backend, name), _>` behind serde that still writes the same JSON array. `add` is an insert rather than a `retain` + push; `is_managed` is a lookup. `managed_index()` is deleted and its two callers, plus the five sites it never reached, ask the registry directly. |
| F7 | `backends.usable()` is probed once per call rather than once per package - lazily, so `shall uninstall apt:jq`, which never needs it, still cannot be broken by an unresolvable `priority`. |
| F8 | **Not a defect.** See below. |
| F9 | One `utils::file::url_filename` parses the URL and takes the last path segment, refusing an empty, `.`, `..` or otherwise non-bare-name result; it shares F1's confinement check. All five sites use it, including `teardown`'s `cached_url`, which has to agree with what the install wrote. |
| F10 | 56 runaway indentation runs across 22 files repaired, and `tests/a_string_carries_no_source_indentation_tests.rs` is the standing gate. |
| F11 | `@sha256` and `@bin` refuse a repeated value, as `@channel` and `@asset` already did. |
| F12 | The program name is escaped like every argument and invoked through `&`. |
| F13 | `dry_run::active()` moved inside `write_capped` and `deploy_executable`, so the rule is structural rather than five verbs remembering it; the three backends also short-circuit after their pre-flight refusals, so a preview still reports what it refused. |
| F14 | `shall.gen` is written through `utils::file::persist`. An unreadable counter is `None`, not `0`, and `spans_one_moment` refuses to conclude from two unknowns. |
| F15 | `blocking::command_output_bounded` is the bounded third door; the external vars provider and the Rhai `sh()`/`sh_ok()` use it. Whole-command bound from `command_idle_timeout_secs`, `0` still meaning none. |
| F16 | `http_get` runs `download::check_scheme` on the seed URL, not only on the redirect hops. |

## F8: the premise did not hold

The report reads `executor.rs`'s `#[cfg(unix)] command.kill_on_drop(false)` as leaving the
snapshot child detached, so that a refused sync's `taking_snapshot.abort()` stops nothing on
Unix. The child is not detached: `wait_watched` wraps it in `supervise::Stopping`, whose `Drop`
sends SIGTERM, and `kill_on_drop` is off *precisely so that* SIGTERM happens instead of tokio's
SIGKILL. The comment three lines above the flag says so. The promise holds on both platforms.

What was true is the audit's underlying complaint: the mechanism is three files away and nothing
at the abort site says so. That is now written down there.

## Beyond the report

- **Fourteen line-number citations lived inside string literals**, where
  `a_citation_in_a_comment_still_points_at_its_claim_tests` could not see them - it read only
  `//` lines. Two were as stale as the load-bearing pair that gate was written for. The gate now
  reads string literals too, and all fourteen name symbols instead of numbers.
- **`@bin` joined `@sha256`** in refusing a duplicate value. Same shape, same reason: one value
  silently won and it was whichever was typed first.

## Behaviour a user can notice

Called out because these are the four that are not invisible from outside the program:

1. A repeated `@sha256=` or `@bin=` on one line is now a parse error. Previously the first value
   won in silence.
2. An external `vars.<ext>` provider or a `sh()` in a Rhai script is now stopped after
   `command_idle_timeout_secs` (default applies). Previously unbounded. `0` removes the bound.
3. `http_get("http://...")` from a script is refused. There is no per-call opt-out; if one is
   wanted it should be a second builtin whose name says so.
4. A download whose body ends short of its declared `Content-Length` is now an error rather than
   a completed install.
5. Listings that walk the managed set - `shall status`'s `@unverified` list among them - come out
   in `(backend, name)` order rather than the order rows were recorded in. A consequence of F6,
   and a stable order is the better one, but it is a visible change.

## Verification

`cargo build --all-targets`, `cargo test --no-fail-fast`, `cargo clippy --all-targets`,
`cargo fmt -- --check`, `scripts/unix-check.sh` - all five, all clean. The last one matters
here: the `#[cfg(unix)]` blocks this change touches are in `deploy_executable` and the bounded
command door, and four of the five steps run on Windows and cannot see them.

Not run: the container integration harness and `cargo mutants`. The properties only the harness
can verify - the removal guard against a real manager, crash and fault injection, a backend's
real install/list/PATH/remove lifecycle - are unchanged by this work, but they are unverified
here as always.
