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
        "emerge",
        "Gentoo is SMOKE_ONLY by design: a source-building install→remove costs hours, so its \
         image installs nothing and crediting it would be a caption, not coverage.",
    ),
    (
        "eopkg",
        "no Solus image exists on any public registry — probed 2026-07-30, \
         getsolus/solus:latest is not published.",
    ),
    (
        "guix",
        "no published base image; Guix installs via a script that needs a running guix-daemon. \
         Closable with an image built from that script.",
    ),
    (
        "slackpkg",
        "Slackware images exist but are community-built and ship a Rust too old to build Shall \
         in-image. Closable by copying in a statically-linked binary.",
    ),
    (
        "yay",
        "AUR helpers refuse to run as root and the container sweep runs as root. Closable with \
         a non-root leg on the arch image.",
    ),
    (
        "paru",
        "the same as yay, and it closes with the same non-root leg.",
    ),
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
        "needs a real Mac. Apple's licence forbids virtualising macOS off Apple hardware, so \
         this is a runner we do not have rather than a thing we have not done.",
    ),
    (
        "link",
        "not a package statement. `link:SRC @target=…` is its own grammar branch, so a harness \
         lifecycle — which builds a `backend:name` package declaration — cannot express one. \
         Covered today by its teardown test and the plan-smoke.",
    ),
    (
        "service",
        "a dependent statement like link, AND starting one needs an init system a plain \
         container does not run. Two independent blocks, and the second is real.",
    ),
    (
        "setting",
        "a dependent statement like link, AND it writes to a live desktop settings store \
         (dconf/gsettings) that no image here runs a bus for.",
    ),
    (
        "stack",
        "its first install downloads a whole GHC toolchain (~2 GB). That is a COST and not an \
         impossibility — Q17 says so — and the fix is the one every other manager in the tools \
         image already got: bake the toolchain in at build time. Named here so the ceiling \
         counts it rather than hiding it in a harness exemption.",
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
