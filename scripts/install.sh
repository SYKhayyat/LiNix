#!/bin/sh
# Shall bootstrap installer — the 30-second first run.
#
#   curl -fsSL https://raw.githubusercontent.com/SYKhayyat/Shall/HEAD/scripts/install.sh | sh
#
# It installs the `shall` binary, runs a health check, and offers to adopt the packages
# already on this machine into a Shall manifest. Override defaults with env vars:
#   SHALL_REPO      git source           (default: the SYKhayyat/Shall repo)
#   SHALL_REF       tag or branch        (default: the newest release tag)
#   SHALL_BIN_DIR   install location     (default: cargo's bin dir)
#   SHALL_NO_ADOPT  set to skip the `adopt` prompt
#
# Every name in that list is read below. It documented `SHALL_BIN_DIR`, which nothing read, and
# omitted `SHALL_REF`, which everything did — in the file users pipe from the internet, where the
# list is the only interface anyone sees.
set -eu

REPO="${SHALL_REPO:-https://github.com/SYKhayyat/Shall}"
BIN_DIR="${SHALL_BIN_DIR:-}"

say() { printf '\033[1;36mshall\033[0m %s\n' "$1"; }
err() { printf '\033[1;31mshall\033[0m %s\n' "$1" >&2; }

say "bootstrapping — detecting toolchain..."

# The build target this machine wants, empty when no release binary is published for it.
# `uname -m` on arm64 macOS says `arm64`; the Rust triple says `aarch64`.
#
# **On Linux the architecture is only half the question — the C library is the other half.**
# A `-gnu` binary is dynamically linked and names `/lib64/ld-linux-x86-64.so.2` as its
# interpreter. NixOS does not have that file, and neither does Alpine; on both, execve fails
# with a message that names nothing a reader can act on. Measured 2026-08-16 with Ubuntu's own
# `/bin/echo`: exit 127 *"cannot execute: required file not found"* in `nixos/nix`, and
# *"not found"* in `alpine:3.20` on a file that is plainly there.
#
# So the loader is asked for rather than assumed, and where it is missing the statically linked
# musl build is fetched instead — one artifact measured to start on nixos/nix, alpine and ubuntu
# alike. `fetch_binary` re-checks by running what it downloaded, so a host neither branch
# predicts still degrades to a source build rather than to a broken install.
target_triple() {
  case "$(uname -s 2>/dev/null || echo unknown)" in
    Linux)  case "$(uname -m)" in
              x86_64)
                if [ -e /lib64/ld-linux-x86-64.so.2 ] || [ -e /lib/ld-linux-x86-64.so.2 ]; then
                  echo x86_64-unknown-linux-gnu
                else
                  echo x86_64-unknown-linux-musl
                fi ;;
              # No aarch64 musl build is published yet, so an arm64 NixOS or Alpine box still
              # falls through to a source build. Named here rather than left as a silent gap:
              # the row is one line in `ci.yml`'s matrix the day somebody wants it.
              aarch64|arm64) echo aarch64-unknown-linux-gnu ;;
            esac ;;
    Darwin) case "$(uname -m)" in
              arm64|aarch64) echo aarch64-apple-darwin ;;
              x86_64)        echo x86_64-apple-darwin ;;
            esac ;;
  esac
}

# Download the published binary for this platform. Prints nothing and returns non-zero when
# there is no asset, no downloader, or no network — every one of which means "build it".
#
# **The reason this exists.** The header of this file promises a 30-second first run and the
# script's only path was `cargo install --git`, which resolves 448 crates and compiles them
# under fat LTO on a stranger's laptop. Nobody measured 30 seconds doing that. A published
# release makes the promise keepable, so the promise is what runs first and the compiler is the
# fallback.
fetch_binary() {
  triple="$(target_triple)"
  [ -n "$triple" ] || return 1
  # `/releases/latest/download/` resolves to the newest release; a pinned `SHALL_REF` asks for
  # exactly that tag, because "install v0.7.0" must not quietly hand over v0.8.0.
  if [ -n "$REF" ]; then
    url="$REPO/releases/download/$REF/shall-$triple"
  else
    url="$REPO/releases/latest/download/shall-$triple"
  fi
  out="$1"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$out" 2>/dev/null || return 1
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$out" "$url" 2>/dev/null || return 1
  else
    return 1
  fi
  # A 404 saved to a file is still a file. An ELF/Mach-O binary is megabytes; an error page is
  # not, and running one is a confusing failure three steps later.
  [ -s "$out" ] || return 1
  size=$(wc -c < "$out")
  [ "$size" -gt 1000000 ] || return 1
  chmod 755 "$out"

  # **And it has to RUN, which is the only question that was never asked.** Everything above
  # checks that a file arrived; none of it checks that this machine can execute what arrived.
  # The published Linux binaries target `-gnu`, and a dynamically linked glibc binary needs
  # `/lib64/ld-linux-x86-64.so.2` — which **NixOS and Alpine do not have**. Measured, not
  # reasoned: Ubuntu's own `/bin/echo` mounted into `nixos/nix` exits 127 with *"cannot execute:
  # required file not found"*, and into `alpine:3.20` with *"not found"* on a file that is
  # plainly there. That error names nothing a reader can act on.
  #
  # Without this line the installer downloaded a perfectly valid binary onto a host that cannot
  # start it, chmod'd it, reported the 30-second success this file promises, and left the user
  # with a `shall` that answers every invocation with `not found`. Alpine is in the integration
  # matrix, so that was a live platform, not a hypothetical.
  #
  # A failure here is not fatal: it returns non-zero like every other branch, and the caller
  # falls back to building from source — which is slow and *works*. Being slow is a promise
  # broken; being broken is a tool.
  "$out" --version >/dev/null 2>&1 || {
    say "the published binary for $triple does not run here — building from source instead."
    return 1
  }
}

# WHICH Shall. `HEAD` is whatever was pushed last, which is not a thing anyone can ask for
# twice — two machines installed an hour apart got different programs and neither could say
# which. The default is the newest release TAG, and `SHALL_REF` overrides it:
#
#   SHALL_REF=main   ...install.sh | sh     # follow the branch, deliberately
#   SHALL_REF=v0.8.0 ...install.sh | sh     # a specific release
#
# A repo with no tags yet falls back to the branch and SAYS SO, rather than silently
# installing something else than it promised.
# **A branch is not a tag, and cargo is told which.** `SHALL_REF` is documented above as "tag or
# branch" and both examples were passed to `--tag`, so the branch example — the one written in
# this file — could not work: `cargo install --git X --tag main` asks libgit2 for
# `refs/remotes/origin/tags/main` and is told `NotFound`. The nightly job runs the documented
# line with `SHALL_REF=main` and has never once been green.
#
# Classified by asking the remote, because the two spellings are indistinguishable from the
# string. `git` stays optional: with no git there is nothing to ask, and `--tag` is the right
# guess for a variable whose default and whose other example are both tags.
REF="${SHALL_REF:-}"
REF_FLAG=--tag
if [ -z "$REF" ]; then
  # No classification needed on this path: whatever comes back came out of the tag list.
  REF="$(git ls-remote --tags --refs --sort=-v:refname "$REPO" 'v*' 2>/dev/null            | head -1 | sed 's#.*/##')"
  if [ -z "$REF" ]; then
    say "no release tag published yet — installing from the default branch instead."
  fi
elif command -v git >/dev/null 2>&1; then
  if [ -n "$(git ls-remote --tags --refs "$REPO" "refs/tags/$REF" 2>/dev/null)" ]; then
    REF_FLAG=--tag
  elif [ -n "$(git ls-remote --heads "$REPO" "refs/heads/$REF" 2>/dev/null)" ]; then
    REF_FLAG=--branch
    say "$REF is a branch, not a release tag — following it."
  fi
fi

# The published binary first. Where it lands is `SHALL_BIN_DIR` if given, and cargo's bin
# directory otherwise — the same two places the source path installs to, so a user who set the
# variable gets the same answer whichever path ran.
CARGO_BIN="${SHALL_BIN_DIR:-${CARGO_HOME:-$HOME/.cargo}/bin}"
STAGE="$(mktemp -d)"
if fetch_binary "$STAGE/shall"; then
  mkdir -p "$CARGO_BIN"
  mv "$STAGE/shall" "$CARGO_BIN/shall"
  rm -rf "$STAGE"
  say "installed the published binary to $CARGO_BIN."
  DOWNLOADED=1
else
  rm -rf "$STAGE"
  DOWNLOADED=
  say "no published binary for this platform — building from source."
fi

if [ -z "$DOWNLOADED" ]; then

# Only the source path needs a compiler, and the check belongs where the need is. Demanding
# Rust before knowing whether a binary was available turned "install this program" into
# "install a toolchain first" for every user on a platform that has a published build.
if ! command -v cargo >/dev/null 2>&1; then
  err "Rust/cargo not found, and no published binary matched this platform."
  err "Install Rust from https://rustup.rs and re-run this script."
  exit 1
fi

if [ -n "$REF" ]; then
  say "building and installing $REF from $REPO (this can take a minute)..."
else
  say "building and installing from $REPO (this can take a minute)..."
fi

# `--locked`, and no fallback. This was a `--locked` attempt with its stderr sent to
# /dev/null and, on *any* non-zero exit, a second run without it — described in the comment as
# "fall back if the lock is unavailable". `Cargo.lock` is tracked in this repository, so the
# case the fallback named cannot happen; what it actually caught was a network blip or a
# compile error, and its response was to resolve 448 dependencies fresh, with the reason
# hidden. That is a supply-chain downgrade triggered by bad wifi, in the script a user pipes
# from the web.
#
# A ref only when there is one: `cargo install --git X --tag ""` is not the same command. Which
# flag carries it was decided above, where the remote could still be asked.
#
# `--root` when the caller named a directory. cargo installs into `$root/bin`, so a
# `SHALL_BIN_DIR` of `/usr/local/bin` is a root of `/usr/local` — computed here rather than
# demanded of the user, who was told this variable names the install location.
set -- --git "$REPO" --locked
# An `if`, not `[ -n "$REF" ] && set -- …`: under `set -e` a trailing `&&` list whose test fails
# is a failing command, so the no-tag path would have exited here.
if [ -n "$REF" ]; then
  set -- "$@" "$REF_FLAG" "$REF"
fi
if [ -n "$BIN_DIR" ]; then
  case "$BIN_DIR" in
    */bin) ROOT="${BIN_DIR%/bin}" ;;
    # Any other directory: cargo cannot be told to use it directly, so install under a root
    # beside it and move the binary. Saying nothing and installing somewhere else would be the
    # variable documented-but-unread all over again.
    *) ROOT="" ;;
  esac
  if [ -n "$ROOT" ]; then
    cargo install "$@" --root "$ROOT"
    CARGO_BIN="$BIN_DIR"
  else
    STAGE="$(mktemp -d)"
    cargo install "$@" --root "$STAGE"
    mkdir -p "$BIN_DIR"
    cp "$STAGE/bin/shall" "$BIN_DIR/shall"
    chmod 755 "$BIN_DIR/shall"
    rm -rf "$STAGE"
    CARGO_BIN="$BIN_DIR"
    say "installed to $BIN_DIR (SHALL_BIN_DIR)"
  fi
else
  cargo install "$@"
  # cargo installs into ~/.cargo/bin; make sure the user can find it.
  CARGO_BIN="${CARGO_HOME:-$HOME/.cargo}/bin"
fi

fi  # end of the build-from-source path

# The shell caches where it found a name. Upgrading over an older `shall` on PATH leaves the
# cache pointing at the binary that was just replaced, and every line below would then run
# the old one — including the health check that is supposed to vouch for the new.
hash -r 2>/dev/null || true
if ! command -v shall >/dev/null 2>&1; then
  case ":$PATH:" in
    *":$CARGO_BIN:"*) : ;;
    *) err "Add $CARGO_BIN to your PATH to use \`shall\`." ;;
  esac
fi

# The binary just installed, by path, in preference to whatever `shall` resolves to on this
# PATH — that could be an older install elsewhere, and the health check is supposed to vouch
# for the one this script produced.
if [ -x "$CARGO_BIN/shall" ]; then
  SHALL="$CARGO_BIN/shall"
else
  SHALL="$(command -v shall || echo "$CARGO_BIN/shall")"
fi

say "running health check..."
"$SHALL" check health || true

if [ -z "${SHALL_NO_ADOPT:-}" ]; then
  printf '\033[1;36mshall\033[0m adopt the packages already installed on this machine into a manifest now? [y/N] '
  # Read from the terminal even when the script itself arrived over a pipe.
  if [ -r /dev/tty ]; then read -r ans </dev/tty; else read -r ans || ans=n; fi
  case "$ans" in
    y | Y | yes | YES) "$SHALL" adopt ;;
    *) say "skipped — run \`shall adopt\` whenever you're ready." ;;
  esac
fi

say "done. Try \`shall check\` or \`shall sync\`."
