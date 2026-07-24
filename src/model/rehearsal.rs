//! `linix try` — rehearse this config on a clean machine (XIII.11, U12).
//!
//! The question `plan` cannot answer: *would this config work on a machine that is not mine?*
//! A plan is computed against the host's installed set, its backends and its quirks, so a
//! config that only works because of something already on this laptop looks fine right up
//! until it reaches the second machine.
//!
//! **Reusing the Phase 6 images (U12, ruled 2026-07-24).** debian/alpine/arch are already built
//! and cover most hosts; a config-named base is the second step, not the blocker. The value is
//! the rehearsal existing at all.
//!
//! **The host is not touched.** The config goes in read-only and LiNix's data directory inside
//! the container is a throwaway, so the rehearsal cannot install anything here, cannot write to
//! the repo, and cannot leave a registry behind.
//!
//! Pure: choosing a runtime, building the argv, and the refusals. Running it is the caller's.

/// A container runtime, in the order `try` prefers them.
///
/// Docker first because it is what most machines have; podman second because it is what the
/// machines that deliberately do not have Docker have. Both take the same arguments for
/// everything `try` needs, which is why this is a name rather than a trait.
pub const RUNTIMES: [&str; 2] = ["docker", "podman"];

/// The images Phase 6 already builds, and what each is for.
pub const IMAGES: [(&str, &str); 3] = [
    ("linix-it-ubuntu", "debian/ubuntu, apt"),
    ("linix-it-alpine", "alpine, apk"),
    ("linix-it-arch", "arch, pacman"),
];

/// The image `try` uses when none is named.
pub const DEFAULT_IMAGE: &str = "linix-it-ubuntu";

/// The first runtime that is actually here.
pub fn pick_runtime(present: &dyn Fn(&str) -> bool) -> Option<&'static str> {
    RUNTIMES.into_iter().find(|r| present(r))
}

/// What to say when there is no container runtime.
///
/// 7h's exit condition: it **refuses and names what is missing**, rather than running anywhere.
/// The alternative — quietly rehearsing on the host — would be a command whose entire purpose
/// is "not on this machine" doing the thing on this machine.
pub fn no_runtime_refusal() -> String {
    format!(
        "refusing to rehearse: `try` needs a container runtime and found neither {}.\n  \
         The whole point of `try` is to answer \"would this config work on a machine that is \
         not mine\", so running it here would answer a different question and call it the same \
         one.\n  \
         Install one, or use `linix check` to validate the config against this machine.",
        RUNTIMES.join(" nor ")
    )
}

/// What to say when the runtime is here but the image has not been built.
pub fn missing_image_refusal(runtime: &str, image: &str) -> String {
    let known: Vec<String> = IMAGES
        .iter()
        .map(|(name, what)| format!("    {}   ({})", name, what))
        .collect();
    format!(
        "refusing to rehearse: `{}` has no image named `{}`.\n  \
         `try` reuses the integration images, which are built from `docker/integration/`:\n\n\
         {}\n\n  \
         Build one with:  {} build -f docker/integration/Dockerfile.ubuntu -t {} .",
        runtime,
        image,
        known.join("\n"),
        runtime,
        DEFAULT_IMAGE
    )
}

/// The command that runs the rehearsal.
///
/// `--rm` so the container is gone afterwards, `:ro` on the config so the rehearsal cannot
/// write to the repo, and a data directory inside the container so nothing it records survives.
/// The check runs as the container's own `linix`, which is the point: a different machine's
/// binary, backends and installed set.
pub fn argv(runtime: &str, image: &str, config_host_path: &str) -> Vec<String> {
    [
        runtime,
        "run",
        "--rm",
        "-v",
        &format!("{}:/linix-config:ro", config_host_path),
        "-e",
        "LINIX_CONFIG_DIR=/linix-config",
        "-e",
        "LINIX_DATA_DIR=/tmp/linix-try-data",
        "--entrypoint",
        "linix",
        image,
        "check",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// What the rehearsal's exit code meant, in U21's vocabulary.
///
/// 0 and 2 are both answers — the config resolved, and either the container matches it or it
/// does not, which on a bare container it never will. Anything else means the config did not
/// survive contact with a machine that is not this one, which is what `try` exists to find.
pub fn verdict(code: Option<i32>) -> Verdict {
    match code {
        Some(0) | Some(2) => Verdict::Valid,
        Some(c) => Verdict::Rejected(c),
        // Killed by a signal: no exit code, and no evidence the config is fine.
        None => Verdict::Rejected(1),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The config resolved on a clean machine.
    Valid,
    /// It did not, with the container's exit code.
    Rejected(i32),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_is_preferred_and_podman_is_the_fallback() {
        assert_eq!(pick_runtime(&|c| c == "docker"), Some("docker"));
        assert_eq!(pick_runtime(&|c| c == "podman"), Some("podman"));
        assert_eq!(pick_runtime(&|_| true), Some("docker"));
        assert_eq!(pick_runtime(&|_| false), None);
    }

    /// 7h: with no runtime it refuses and names what is missing, rather than running anywhere.
    #[test]
    fn the_refusal_names_both_runtimes() {
        let msg = no_runtime_refusal();
        assert!(msg.contains("docker"), "{}", msg);
        assert!(msg.contains("podman"), "{}", msg);
        assert!(msg.contains("refusing"), "{}", msg);
    }

    /// The config is mounted READ-ONLY. Without this the rehearsal could write to the repo it
    /// was asked to inspect — and `try`'s whole claim is that it touches nothing on the host.
    #[test]
    fn the_config_is_mounted_read_only() {
        let cmd = argv("docker", "linix-it-ubuntu", "/home/a/.config/linix");
        let mount = cmd
            .iter()
            .find(|a| a.contains(":/linix-config"))
            .expect("the config must be mounted");
        assert!(mount.ends_with(":ro"), "{}", mount);
    }

    /// The container's data directory is inside the container. A rehearsal that wrote a
    /// registry to the host would make "it touched nothing" false.
    #[test]
    fn the_rehearsals_data_stays_in_the_container() {
        let cmd = argv("docker", "linix-it-ubuntu", "/cfg");
        let data = cmd
            .iter()
            .find(|a| a.starts_with("LINIX_DATA_DIR="))
            .expect("a data dir must be set");
        assert!(!data.contains("/cfg"), "{}", data);
        assert!(data.contains("/tmp/"), "{}", data);
    }

    /// `--rm`: a rehearsal that piled up stopped containers would be a command you stop running.
    #[test]
    fn the_container_removes_itself() {
        assert!(argv("docker", "i", "/cfg").iter().any(|a| a == "--rm"));
    }

    #[test]
    fn the_runtime_and_image_land_in_the_command() {
        let cmd = argv("podman", "linix-it-alpine", "/cfg");
        assert_eq!(cmd[0], "podman");
        assert!(cmd.contains(&"linix-it-alpine".to_string()));
    }

    /// U21 again: a read-only command that found differences exits 2, and on a bare container
    /// it always will — nothing is installed there. Reading that as "your config is broken"
    /// would make `try` reject every config that is completely fine.
    #[test]
    fn differences_on_a_bare_container_are_not_a_rejection() {
        assert_eq!(verdict(Some(0)), Verdict::Valid);
        assert_eq!(verdict(Some(2)), Verdict::Valid);
    }

    #[test]
    fn a_config_that_does_not_resolve_is_rejected() {
        assert_eq!(verdict(Some(1)), Verdict::Rejected(1));
        assert_eq!(verdict(Some(3)), Verdict::Rejected(3));
        // Killed by a signal: no code, and no evidence the config is fine.
        assert_eq!(verdict(None), Verdict::Rejected(1));
    }

    #[test]
    fn the_missing_image_refusal_says_how_to_build_one() {
        let msg = missing_image_refusal("docker", "linix-it-ubuntu");
        assert!(msg.contains("docker build"), "{}", msg);
        assert!(msg.contains("docker/integration/Dockerfile.ubuntu"), "{}", msg);
        for (name, _) in IMAGES {
            assert!(msg.contains(name), "{} missing from {}", name, msg);
        }
    }
}
