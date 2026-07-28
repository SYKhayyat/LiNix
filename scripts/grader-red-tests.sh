#!/usr/bin/env bash
# ============================================================================
# The GRADER's failing tests, 2026-07-28. Every check here is RED on the tree
# it was written against (213973a) and each one has a reproduction in the
# grader's report. Red is the deliverable: this file is the list of things that
# are still wrong, in a form that goes quiet as they are fixed.
#
#   bash scripts/grader-red-tests.sh
#
# Exit 0 only when every finding below is fixed.
# ============================================================================
set -u

ROOT="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT" || exit 2

PASS=0; FAIL=0
red()  { FAIL=$((FAIL + 1)); echo "  RED   $1"; }
green(){ PASS=$((PASS + 1)); echo "  ok    $1"; }

echo "== G1: the mutation gate must fail when nothing ran"
# `grep -c` prints 0 AND exits 1 when there are no matches, so `|| echo 0` runs too and
# the count becomes the two-line string "0\n0". Both guards in harness-mutation-test.sh
# then die on "integer expected" and the gate prints ok and exits 0 — in exactly the
# total-collapse case the guards exist for.
EMPTY="$(mktemp)"; : > "$EMPTY"
COUNT="$(grep -c "  PASS  " "$EMPTY" 2>/dev/null || echo 0)"
rm -f "$EMPTY"
if [ "$(printf '%s' "$COUNT" | wc -l)" -eq 0 ] && [ "$COUNT" = "0" ]; then
    green "the survivor/caught counter yields a single integer"
else
    red "the counter yields [$COUNT] — both of the gate's guards die on 'integer expected' and it exits 0"
fi

echo "== G2: the coverage audit must not pass over an empty registry"
# Both harnesses take ALL_BACKENDS and HELP_CMDS from the program under test, then assert
# set-containment with no floor. A binary that enumerates nothing scores perfect coverage:
# measured under a do-nothing stub, the audit printed "0 in --help ... 0 registered" and
# PASSed both meta-checks.
for h in scripts/integration-windows.sh docker/integration/run-in-container.sh; do
    if grep -q "coverage: the registry came back empty\|registry is empty\|-lt 1 \]" "$h" 2>/dev/null; then
        green "$(basename "$h") refuses an empty registry"
    else
        red "$(basename "$h") passes its coverage audit when the registry enumerates nothing"
    fi
done

echo "== G4: the argv-drift gate must not be blind to shimmed managers"
# help_text() uses raw std::process::Command, which cannot launch a .cmd/.ps1 shim, while
# on_path() uses which::which, which finds one. So scoop, npm, yarn, gem and pipx are all
# "installed" and all skipped as "its help could not be read" — and a skip is not a failure.
if grep -q "windows_effective_command\|shim" tests/argv_drift_tests.rs; then
    green "argv_drift_tests routes through the executor's shim wrapping"
else
    red "argv_drift_tests uses raw Command: every script-shimmed manager on Windows is skipped, not checked"
fi
if grep -q "could not be read" tests/argv_drift_tests.rs; then
    red "a manager that IS installed but whose help cannot be read is scored as a skip, not a failure"
else
    green "an unreadable help on an installed manager is a failure"
fi

echo "== G5: the E5 catch-all must be gone from BOTH sections, not just section 12"
if grep -q 'network/ecosystem variance' scripts/integration-windows.sh; then
    red "integration-windows.sh section 5 still soft-passes any install failure as 'variance' and skips the rest of the lifecycle"
else
    green "no unclassified install-failure catch-all remains"
fi

echo "== G6: every interactive prompt needs the non-interactive refusal"
if grep -q "is_terminal" src/app/snapshot_restore.rs; then
    green "snapshot_restore guards its Select prompt"
else
    red "src/app/snapshot_restore.rs run_interactive() prompts with no is_terminal guard (E22/E23's unfixed sibling, on a restore path)"
fi

echo "== G7: the container harness must be mutation-tested too"
if grep -q "run-in-container" .github/workflows/ci.yml && \
   grep -A6 "harness-mutation" .github/workflows/ci.yml | grep -q "run-in-container"; then
    green "CI mutation-tests the container harness"
else
    red "CI runs harness-mutation-test.sh only against integration-windows.sh; the container harness (4 distros, every push) is never mutation-tested"
fi

echo "== G8: the local gates must match CI in both directions"
if grep -q "harness-logic-test.sh" scripts/release-check.sh; then
    green "release-check.sh runs the harness predicates, as CI and release-check.ps1 do"
else
    red "release-check.sh omits harness-logic-test.sh, which CI runs and release-check.ps1 runs — a gate weaker than CI (E4's class, in the twin script)"
fi

echo "== E6c: an install whose binary cannot be reached must say so"
if grep -rq "not on your PATH\|is not on PATH, so the binary" --include=*.rs src/app src/verbs 2>/dev/null; then
    green "LiNix reports a bin directory that is not on PATH"
else
    red "pub installs sass, list agrees, and the binary is unreachable because ~/.pub-cache/bin is not on PATH — LiNix never mentions it"
fi

echo "== E12: luarocks must pin a Lua version it can resolve"
if grep -A20 "fn register_luarocks" src/backends/registry.rs | grep -q "lua-version\|check-lua-versions"; then
    green "luarocks pins a Lua version"
else
    red "luarocks install fails with 'No results matching query were found for Lua 5.5' — no Lua version pinned"
fi

echo "== G9: --dry-run must not write to the config"
# Measured, on a package really installed and really declared through LiNix:
#
#   $ linix --dry-run uninstall scoop:sd
#     Planned changes:  install 0  remove 1
#   $ diff manifest.before manifest.after
#     6d5
#     < scoop:sd                      <-- the line is gone, and the package is still installed
#
# `handle_uninstall` (src/verbs/packages.rs:213) calls `app.undeclare(pkg)` unconditionally;
# only `handle_sync` below it consults the flag. `handle_install` guards correctly, and so
# does the `--temp` path in `suspend_for_session`. `unmanage` has the same shape as uninstall.
# This repo's flagship historical bug was a `--dry-run` that performed the removal; this is
# that bug moved from the machine to the manifest. A hand-written manifest is NOT a
# reproduction — the line must be declared by a real `linix install` — so this asserts the
# missing guard at its source rather than a setup that silently fails to reach the condition.
if awk '/^pub\(crate\) async fn handle_uninstall/,/^pub\(crate\) async fn suspend_for_session/' \
        src/verbs/packages.rs | grep -q "dry_run"; then
    green "handle_uninstall consults the dry-run flag before it undeclares"
else
    red "handle_uninstall undeclares with no dry-run guard: --dry-run uninstall/unmanage delete the declaration for real"
fi

echo "--------------------------------------------------------------"
echo " grader findings: $PASS fixed, $FAIL still red"
[ "$FAIL" = 0 ] || exit 1
