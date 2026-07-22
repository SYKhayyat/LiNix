#!/bin/sh
# LiNix bootstrap installer — the 30-second first run.
#
#   curl -fsSL https://raw.githubusercontent.com/OWNER/linix/HEAD/scripts/install.sh | sh
#
# It installs the `linix` binary, runs a health check, and offers to adopt the packages
# already on this machine into a LiNix manifest. Override defaults with env vars:
#   LINIX_REPO      git source           (default: the OWNER/linix repo)
#   LINIX_BIN_DIR   install location     (default: cargo's bin dir)
#   LINIX_NO_ADOPT  set to skip the `migrate` prompt
set -eu

REPO="${LINIX_REPO:-https://github.com/OWNER/linix}"

say() { printf '\033[1;36mlinix\033[0m %s\n' "$1"; }
err() { printf '\033[1;31mlinix\033[0m %s\n' "$1" >&2; }

say "bootstrapping — detecting toolchain..."

if ! command -v cargo >/dev/null 2>&1; then
  err "Rust/cargo not found."
  err "Install Rust from https://rustup.rs and re-run this script."
  exit 1
fi

say "building and installing from $REPO (this can take a minute)..."
# Prefer the reproducible, lockfile-pinned build; fall back if the lock is unavailable.
if ! cargo install --git "$REPO" --locked 2>/dev/null; then
  cargo install --git "$REPO"
fi

# cargo installs into ~/.cargo/bin; make sure the user can find it.
CARGO_BIN="${CARGO_HOME:-$HOME/.cargo}/bin"
# The shell caches where it found a name. Upgrading over an older `linix` on PATH leaves the
# cache pointing at the binary that was just replaced, and every line below would then run
# the old one — including the health check that is supposed to vouch for the new.
hash -r 2>/dev/null || true
if ! command -v linix >/dev/null 2>&1; then
  case ":$PATH:" in
    *":$CARGO_BIN:"*) : ;;
    *) err "Add $CARGO_BIN to your PATH to use \`linix\`." ;;
  esac
fi

# The binary just installed, by path, in preference to whatever `linix` resolves to on this
# PATH — that could be an older install elsewhere, and the health check is supposed to vouch
# for the one this script produced.
if [ -x "$CARGO_BIN/linix" ]; then
  LINIX="$CARGO_BIN/linix"
else
  LINIX="$(command -v linix || echo "$CARGO_BIN/linix")"
fi

say "running health check..."
"$LINIX" doctor || true

if [ -z "${LINIX_NO_ADOPT:-}" ]; then
  printf '\033[1;36mlinix\033[0m adopt the packages already installed on this machine into a manifest now? [y/N] '
  # Read from the terminal even when the script itself arrived over a pipe.
  if [ -r /dev/tty ]; then read -r ans </dev/tty; else read -r ans || ans=n; fi
  case "$ans" in
    y | Y | yes | YES) "$LINIX" migrate ;;
    *) say "skipped — run \`linix migrate\` whenever you're ready." ;;
  esac
fi

say "done. Try \`linix status\` or \`linix doctor\`."
