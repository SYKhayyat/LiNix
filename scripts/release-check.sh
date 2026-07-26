#!/usr/bin/env bash
# release-check.sh — the single "am I ready to ship?" gate for Linux/macOS.
#
# Runs, in order, everything a release must pass and prints one go/no-go verdict:
#   1. HERMETIC gates (any host, fast, no Docker):
#        cargo fmt --check          (HARD)
#        cargo clippy -D warnings   (HARD)
#        cargo test --release       (HARD — all unit/integration tests)
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

echo "-> cargo test --release"
if cargo test --release; then pass "cargo test: all tests pass"
else fail "cargo test: failures"; fi

echo "-> cargo build --release"
if cargo build --release; then pass "release build succeeds"
else fail "release build FAILED"; fi

# ------------------------------------------------------------------ 2. integration
if [ "$SKIP_DOCKER" = "1" ]; then
    info "SKIP_DOCKER=1 — skipped the real integration matrix (hermetic gates only)"
elif [ "$OS" = "Darwin" ]; then
    step "2. NATIVE INTEGRATION (macOS — no Linux containers; sweeping brew)"
    if LINIX="$REPO_ROOT/target/release/linix" bash scripts/integration-windows.sh brew wget; then
        pass "native brew integration sweep PASS"
    else fail "native brew integration sweep FAILED"; fi
elif command -v docker >/dev/null 2>&1; then
    step "2. REAL INTEGRATION MATRIX (Docker: every distro + tools + gentoo)"
    # Full release coverage includes gentoo (emerge, SMOKE-ONLY). Override with DISTROS=…
    export DISTROS="${DISTROS:-ubuntu fedora arch alpine tools gentoo}"
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
