#!/usr/bin/env bash
# release-check.sh — the single "am I ready to ship?" gate for Linux/macOS.
#
# Runs, in order, everything a release must pass and prints one go/no-go verdict:
#   1. HERMETIC gates (any host, fast, no Docker):
#        cargo fmt --check          (HARD)
#        cargo clippy -D warnings   (HARD)
#        cargo test --release --no-fail-fast   (HARD — all unit/integration tests)
#        cargo build --release      (HARD)
#   2. REAL integration matrix (needs Docker on Linux): every distro image + the broad `tools`
#        image, each doing real install→list→remove for every feasible backend, full feature
#        coverage, and a self-checking coverage audit. On macOS (no Docker for Linux images)
#        this step runs the native sweep instead.
#
# Usage:
#   ./scripts/release-check.sh                 # full: hermetic gates + Docker matrix (incl. gentoo)
#   SKIP_DOCKER=1 ./scripts/release-check.sh   # hermetic gates only
#   DISTROS="ubuntu tools" ./scripts/release-check.sh   # subset of images
#
# Exit code is non-zero if any HARD gate failed — wire it straight into CI or a pre-release hook.
set -u
REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || (cd "$(dirname "$0")/.." && pwd))"
cd "$REPO_ROOT" || { echo "cannot cd to repo root"; exit 2; }

GREEN=""; RED=""; YEL=""; RST=""
if [ -t 1 ]; then GREEN="$(printf '\033[32m')"; RED="$(printf '\033[31m')"; YEL="$(printf '\033[33m')"; RST="$(printf '\033[0m')"; fi
step()  { echo; echo "############### $* ###############"; }
result="" ; hard_fail=0
pass()  { echo "${GREEN}[PASS]${RST} $1"; result="${result}\n  ${GREEN}PASS${RST}  $1"; }
fail()  { echo "${RED}[FAIL]${RST} $1"; result="${result}\n  ${RED}FAIL${RST}  $1"; hard_fail=1; }
info()  { echo "${YEL}[INFO]${RST} $1"; result="${result}\n  ${YEL}INFO${RST}  $1"; }

OS="$(uname -s)"
SKIP_DOCKER="${SKIP_DOCKER:-0}"

# ------------------------------------------------------------------ 1. hermetic
step "1. HERMETIC GATES (cargo fmt / clippy / test / build)"

if cargo fmt --check >/dev/null 2>&1; then pass "cargo fmt --check (formatting clean)"
else fail "cargo fmt --check reports diffs — run \`cargo fmt\`"; fi

echo "-> cargo clippy --all-targets --all-features -- -D warnings"
if cargo clippy --all-targets --all-features -- -D warnings; then pass "clippy: no warnings"
else fail "clippy reported warnings/errors"; fi

echo "-> cargo test --release --no-fail-fast"
# `--no-fail-fast` because CI does: without it cargo stops at the first test TARGET that
# fails, and this suite has dozens. A local gate that measures less than CI turns a local GO
# into a NO-GO on the push (G-4).
if cargo test --release --no-fail-fast; then pass "cargo test: all tests pass"
else fail "cargo test: failures"; fi

echo "-> cargo build --release"
if cargo build --release; then pass "release build succeeds"
else fail "release build FAILED"; fi

# The `supply-chain` and `msrv` CI jobs, run locally — because a CI job nothing local drives is a
# gate a developer finds out about from a red push, which is what `grade6_gate_parity` asserts
# against. Both are soft here and hard in CI: `cargo-deny` and a pinned toolchain are installs a
# contributor may not have, and a release script that refuses to run without them stops being run.
echo "-> cargo deny check (advisories, bans, licences, sources)"
if command -v cargo-deny >/dev/null 2>&1; then
    if cargo deny check advisories bans licenses sources; then pass "cargo deny: clean"
    else fail "cargo deny: findings — see above"; fi
else
    info "cargo-deny not installed (cargo install cargo-deny --locked); CI runs it regardless"
fi

# The `shell` CI job. Two files in this repo were already writing `# shellcheck disable=`
# directives for a linter nothing ran, which is a suppression addressed to nobody — and the
# harnesses that decide pass/fail for every backend are shell, where a quoting bug is a wrong
# verdict. Soft here for the same reason cargo-deny is: an install a contributor may not have.
echo "-> shellcheck (scripts/*.sh, docker/**/*.sh)"
if command -v shellcheck >/dev/null 2>&1; then
    # shellcheck disable=SC2046  # word-splitting the file list is the point
    if shellcheck -S warning $(ls scripts/*.sh docker/*/*.sh 2>/dev/null); then
        pass "shellcheck: clean"
    else fail "shellcheck: findings - see above"; fi
else
    info "shellcheck not installed (apt/brew install shellcheck); CI runs it regardless"
fi

echo "-> cargo check on the declared MSRV"
MSRV="$(grep -m1 '^rust-version' Cargo.toml | cut -d'"' -f2)"
# The guard the `.ps1` twin has and this did not. An empty `$MSRV` — `rust-version` renamed,
# moved, or quoted differently — makes the test below `grep -q "^"`, which matches every line of
# `rustup toolchain list`, so the check reports GO having run `cargo +"" check`. A gate that
# passes hardest when its input is missing is the shape this script exists to catch elsewhere.
if [ -z "$MSRV" ]; then
    fail "could not read rust-version from Cargo.toml — the MSRV check cannot run"
elif rustup toolchain list 2>/dev/null | grep -q "^$MSRV"; then
    if cargo "+$MSRV" check --all-targets --locked; then pass "builds on the declared MSRV ($MSRV)"
    else fail "does NOT build on rust-version = $MSRV — raise it deliberately or fix the use"; fi
else
    info "toolchain $MSRV not installed (rustup toolchain install $MSRV); CI runs it regardless"
fi

# CI runs both of these and this script ran neither, so a local GO could still be a CI NO-GO —
# the same asymmetry E3/E4 found in `cargo fmt`, one file over. release-check.ps1 runs the
# predicates; parity between the two release scripts is asserted from ci.yml by
# `tests/the_review_apparatus_is_rust_tests.rs`, so a gate added to CI fails until it is added
# here too.
echo "-> scripts/harness-logic-test.sh"
if SHALL_BIN="$REPO_ROOT/target/release/shall" bash scripts/harness-logic-test.sh; then
    pass "harness predicates"
else fail "harness predicates FAILED"; fi

# A harness is trustworthy because its checks can go red, not because they are green. This
# runs them against a `shall` that does nothing and fails if too many still pass.
echo "-> scripts/harness-mutation-test.sh --check"
if bash scripts/harness-mutation-test.sh --check; then
    pass "harness mutation budget"
else fail "harness mutation budget EXCEEDED — checks that examine nothing"; fi

# And the OTHER harness (G-4). CI mutation-tests both; this script tested one, and the parity
# gate reported ok because it compared basenames. The four-distro
# harness runs on every push against 136 checks and was measured in exactly one place.
# Needs no Docker: the harness is shell, and the point is to run it against a stub.
echo "-> scripts/harness-mutation-test.sh docker/integration/run-in-container.sh --check"
if bash scripts/harness-mutation-test.sh docker/integration/run-in-container.sh --check apt jq; then
    pass "container harness mutation budget"
else fail "container harness mutation budget EXCEEDED — checks that examine nothing"; fi

# ------------------------------------------------------------------ 2. integration
if [ "$SKIP_DOCKER" = "1" ]; then
    info "SKIP_DOCKER=1 — skipped the real integration matrix (hermetic gates only)"
elif [ "$OS" = "Darwin" ]; then
    step "2. NATIVE INTEGRATION (macOS — no Linux containers; sweeping brew)"
    if SHALL="$REPO_ROOT/target/release/shall" bash scripts/integration-windows.sh brew wget; then
        pass "native brew integration sweep PASS"
    else fail "native brew integration sweep FAILED"; fi
elif command -v docker >/dev/null 2>&1; then
    step "2. REAL INTEGRATION MATRIX (Docker: every distro + tools + gentoo)"
    # Full release coverage includes gentoo (emerge, SMOKE-ONLY). Override with DISTROS=…
    # Every image CI drives, plus gentoo, which is nightly there. `opensuse`, `void` and
    # `storage` were missing: CI has run all three on every push since 2026-07-31, and the
    # parity checker could not see it because a matrix row is not a script name (G-4).
    export DISTROS="${DISTROS:-ubuntu fedora arch alpine opensuse void storage tools gentoo}"
    if ./docker/integration/run.sh; then pass "integration matrix ($DISTROS) PASS"
    else fail "integration matrix ($DISTROS) had FAILURES"; fi
else
    fail "Docker not found — cannot run the real integration matrix (install Docker, or SKIP_DOCKER=1 for hermetic-only)"
fi

# ------------------------------------------------------------------ verdict
step "RELEASE VERDICT"
printf "%b\n" "$result"
echo
if [ "$hard_fail" -eq 0 ]; then
    echo "${GREEN}=====> GO: every hard gate passed. Ready to release.${RST}"
    exit 0
else
    echo "${RED}=====> NO-GO: at least one hard gate failed (see above).${RST}"
    exit 1
fi
