//! Which backends have never met the manager they are named after, and why.
//!
//! **A user cannot currently tell a backend with a real lifecycle behind it from one that has
//! never run.** They are listed side by side, described in the same words, and one of them is a
//! claim nothing has ever checked. That is the whole of this module: the claim is still made,
//! and it is now made *with its standing attached*.
//!
//! This table used to live in `tests/lifecycle_coverage_union_tests.rs`, which is the only place
//! that sees both harnesses at once. Keeping it there meant the repository knew the answer and
//! the program could not say it — and a fact known only to a test is a fact no user gets. The
//! test now reads **this** table rather than a copy of it, which is `F7`'s lesson applied rather
//! than repeated: a gate that reads a transcription reports on the transcription.
//!
//! **The bar for an entry is that a harness genuinely *cannot* do it** (`Q17`). A cost is not a
//! reason — "it downloads 2 GB" is an argument for baking it into an image, not for an
//! exemption — and "it touches the real machine" is not a reason either, because every package
//! manager does. `stack` is listed with its reason stated as a cost precisely so the ceiling
//! counts it instead of a harness exemption hiding it.
//!
//! **What it does not mean.** An entry here is not "this backend is broken" and not "this
//! backend is untested": every one of them has unit coverage, a plan-smoke, and a place in the
//! registry audit. It means exactly one thing — *no harness in this repository has driven a real
//! install → list → binary-on-PATH → remove through it* — so if it is wrong about the manager,
//! nothing here would know.

/// A backend no harness can reach with a real lifecycle, and why.
///
/// Ordered as a reader meets them: the Linux managers first, then the BSD and Apple ones that
/// need a kernel or hardware we do not have, then the dependent statements that are not package
/// declarations at all.
pub const UNPROVEN: &[(&str, &str)] = &[
    (
        "nixos",
        "no CI leg is NixOS — a cost rather than an impossibility, which this table is supposed \
         to refuse, so it is recorded with a receipt and a price instead of an excuse. THE \
         RECEIPT: the whole lifecycle was driven by hand on NixOS 26.05 (Yarara) under WSL on \
         2026-08-16 — `nixos:hello` declared, the generated module written into a root-owned \
         /etc/nixos, the import inserted into the real configuration.nix, `nixos-rebuild switch` \
         run, and `hello` found at /run/current-system/sw/bin/hello saying `Hello, world!`; then \
         `list` read it back, undeclaring it took it off PATH, and the machine still dry-builds. \
         That run found FOUR defects every hermetic layer had passed, one of which reported \
         `Status: SUCCESS` over a machine where nothing had been installed. THE PRICE: \
         `nixos/nix` cannot stand in — probed the same day, it is the Nix package manager on a \
         minimal base with no /etc/NIXOS, no /run/current-system, no `nixos-rebuild` and no \
         systemd. Closing this wants a NixOS image that can run `nixos-rebuild`, or NixOS-WSL on \
         a Windows runner. NARROWER AGAIN, 2026-08-16: the services-and-perimeter module has now \
         been handed to a real `nixos-rebuild` on that same NixOS 26.05. `nixos-rebuild build` \
         over a configuration importing a module carrying `hello`, `services.cron.enable`, \
         `networking.firewall.enable` and both port lists EVALUATED AND BUILT a complete system \
         closure (`nixos-system-nixos-26.05pre-git`), and the negative control — one option \
         nixpkgs does not have — failed with `The option ... does not exist`, so the check is \
         not vacuous. `switch` on that distro fails at ACTIVATION with a dbus error, and the \
         control settles whose fault that is: `nixos-rebuild switch` with the machine's own \
         configuration and no Shall in it fails identically (exit 4). What that same run DID \
         prove about Shall is the rollback — the failed switch put both /etc/nixos files back \
         and named them, which no hermetic test can stage. WHAT IS LEFT: activation, and only \
         activation. Automatically, `scripts/nix-validate.sh --evaluate` now merges every \
         generated module into a real NixOS module system (`<nixpkgs/nixos>`), which is where a \
         wrong option name arrives — parsing never saw one. An evaluation is not an activation.",
    ),
    (
        "emerge",
        "Gentoo is SMOKE_ONLY: its image installs nothing, so crediting it would be a caption. \
         The stated reason — a source build costs hours — is no longer the whole truth and the \
         correction is written here rather than left to be rediscovered: probed 2026-08-14, \
         `gentoo/stage3:latest` ships `/etc/portage/binrepos.conf/gentoo.conf`, so an official \
         BINARY host is configured out of the box, and what the image lacks is the portage tree \
         itself (`/var/db/repos/gentoo` is absent, so emerge refuses every action but --sync). \
         The closing move is therefore an `emerge-webrsync` at build time plus `FEATURES=\
         \"getbinpkg\"`, which is a price in image size and not an impossibility. Unproven \
         because nobody has paid it, which is a different sentence from the one that was here.",
    ),
    (
        "eopkg",
        "no Solus image exists on any public registry — probed 2026-07-30 and again 2026-08-14; \
         neither getsolus/solus:latest nor solus/solus:latest has a manifest. The only entry \
         here whose re-derivation changed nothing.",
    ),
    // `guix` was here, through three successively less wrong reasons: first "no published base
    // image, and Guix needs a running guix-daemon" (both halves false — metacall/guix:latest is
    // published and the daemon runs fine), then "the manager works but SHALL has not driven it".
    // Shall drives it now: `Dockerfile.guix` plus `--security-opt seccomp=unconfined`, because
    // guix's build sandbox calls `personality(2)` and Docker's default profile blocks it.
    //
    // Two things the image had to be taught, and both are worth keeping because neither is about
    // guix being unusual — they are about what an image owes a harness. `guix install` writes to
    // `/root/.guix-profile/bin` and tells you to put it on PATH yourself, so the canary was
    // installed and invisible; and a guix profile starts EMPTY, so the history proofs had no git
    // and the removal guard had no protected package to refuse. The fixtures go in at run time
    // rather than in a build layer, because that same seccomp wall blocks `guix install` during
    // `docker build`.
    // `slackpkg` was here, exempted because "Slackware images exist but are community-built and
    // ship a Rust too old to build Shall in-image". Both halves were wrong. `Dockerfile.slackware`
    // bootstraps the toolchain THROUGH slackpkg — installing the `d`, `l` and `n` series, which is
    // the lesson the file records: slackpkg resolves series, not package names — and the Rust it
    // gets builds Shall. It now runs the full harness, drives its own install → list → binary →
    // remove, and joins the CI matrix.
    //
    // Recorded rather than deleted silently, because what it cost is the point: while slackpkg had
    // never met its own manager, two defects hid behind it — it sorted below the language managers
    // (VI.7) and it was sent a `--` it reads as the search pattern (VI.9). Neither is visible from
    // an argv test, and both were found in the first hour the image existed.
    // `yay` and `paru` were here, exempted because "AUR helpers refuse to run as root and the
    // container sweep runs as root", with the closing move named in the entry itself: "closable
    // with a non-root leg on the arch image". That leg exists now — `Dockerfile.arch` creates an
    // unprivileged user and Shall escalates through `sudo -n` for the backends whose
    // `needs_root` is true, which is the behaviour `register_aur_helper` always assumed. Both
    // have canaries in `run-in-container.sh` and are driven for real.
    //
    // Recorded here rather than deleted silently: the entry was an honest cost, and what makes
    // it worth reading now is that the cost was a property of the harness the whole time.
    (
        "pkg",
        "FreeBSD userland. A container shares the host's Linux kernel, so this needs a VM and \
         not an image.",
    ),
    (
        "pkg_add",
        "OpenBSD userland — a VM, for the same reason as pkg.",
    ),
    (
        "pkgin",
        "NetBSD/SmartOS userland — a VM, for the same reason as pkg.",
    ),
    (
        "mas",
        "needs a signed-in Mac App Store account on real Apple hardware. No container and no VM \
         can hold one legitimately.",
    ),
    (
        "macports",
        "this reason was WRONG, and the correction is the entry: it said MacPorts needed \
         a runner we do not have, while CI has run `Integration (macOS native, nightly)` and \
         the macOS argv-drift leg on `macos-latest` the whole time. Apple forbids VIRTUALISING \
         macOS, which is true and says nothing about a real Apple runner. What is missing is a \
         step that installs MacPorts on it — work nobody has done, not hardware nobody has.",
    ),
    // `link` was exempted here as "not a package statement ... a harness lifecycle — which
    // builds a `backend:name` package declaration — cannot express one". That is true of
    // `canary()`'s shape and false of the subject: a symlink needs no manager, no network, no
    // init system and no privileges, so every image could always have driven one. Section 14b
    // of the container harness does — declare, sync, assert the symlink AND its contents,
    // undeclare, assert it is gone — and `dependent_lifecycle()` declares it where the union
    // gate reads it.
    // `service` and `setting` were here, and both reasons named one init system and one settings
    // store as if they were the only ones. `init_providers.toml` and `setting_stores.toml` each
    // hold five rows.
    //
    // `service`'s reason — "starting one needs an init system a plain container does not run" —
    // is true of systemd and false of SysVinit, whose enable is `update-rc.d` writing rc symlinks
    // and whose start executes a shell script. Neither cares what PID 1 is, and the Debian-family
    // images have shipped both commands all along. Section 14c of the container harness drives
    // the full enable → start → disable → stop and asserts each against the machine.
    //
    // `setting`'s reason — "a live desktop settings store (dconf/gsettings) with no bus here" —
    // was asked only of Linux. On Windows the store is `reg`, which needs no bus, no session and
    // no desktop; section 12b of the Windows sweep writes a value under HKCU, reads it back,
    // changes it, and asserts the store's `reset` on teardown.
    //
    // Kept as a comment rather than deleted: both entries were honestly written, and what makes
    // them worth reading is that each described the platform its author was standing on.
    (
        "stack",
        "its first install downloads a whole GHC toolchain (~2 GB), which is a COST and can be \
         baked into an image. What cannot: `stack install` builds the PACKAGE from source too, \
         so the smallest thing on Hackage is minutes per run on every image for ever. A cost \
         plus a recurring cost, named here rather than hidden in a harness exemption — the \
         earlier wording said the toolchain was the whole of it and made this look closable by \
         a build-time download it is not.",
    ),
];

/// Why this backend has never met its manager, or `None` when it has.
pub fn unproven_reason(backend: &str) -> Option<&'static str> {
    UNPROVEN
        .iter()
        .find(|(name, _)| *name == backend)
        .map(|(_, why)| *why)
}

/// Whether a real lifecycle has ever been driven through this backend by some harness.
pub fn is_proven(backend: &str) -> bool {
    unproven_reason(backend).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_answers_both_ways() {
        assert!(is_proven("apt"), "apt has a lifecycle in every Linux image");
        assert!(!is_proven("mas"), "mas needs hardware no runner here has");
        assert!(unproven_reason("mas").is_some_and(|w| w.contains("Apple")));
        assert!(unproven_reason("apt").is_none());
    }

    /// Every reason says something. A blank excuse is the exemption this table exists to stop
    /// being possible — `Q17`'s bar is that a harness *cannot*, and a reason nobody wrote is
    /// indistinguishable from a reason nobody has.
    #[test]
    fn every_entry_states_a_reason() {
        for (backend, why) in UNPROVEN {
            assert!(
                why.len() > 40,
                "`{backend}` is exempted with {} characters of reason; say why a harness \
                 cannot reach it",
                why.len()
            );
            assert!(!backend.is_empty());
        }
    }
}
