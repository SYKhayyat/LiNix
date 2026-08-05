//! Every download backend asks whether it may deploy *before* it spends the network.
//!
//! `deploy_executable`'s refusal — `is_ours(dest, owned_root, recorded)` — is a pure function of
//! the destination. It needs zero downloaded bytes. Called only at deploy time it still refuses
//! correctly, which is why reading the function finds nothing wrong: the defect is the **order
//! it is called in**, and that is not visible anywhere inside it.
//!
//! Measured before the fix, inside one `heal`: 60.9s and 119.1s back to back, both ending in
//! `refusing to deploy fd.exe`. 180 of that run's 201 seconds, silent, at zero CPU with no child
//! process — from outside, indistinguishable from a hang, and the reason three stalls were
//! misdiagnosed.
//!
//! A source scan, because the defect is an ordering and a **missing line**, the same reason
//! `prompt_guard_tests.rs` is one. A fourth download backend added tomorrow joins this test by
//! existing, which is the only property that stops the class coming back.

use std::path::Path;

/// The call that pulls the artifact down, per backend — not the metadata call beside it. The
/// release listing and the `HEAD` for an etag are one small request each and have to precede the
/// check, because the check needs the name they resolve.
const BACKENDS: &[(&str, &str)] = &[
    ("src/backends/github.rs", "github_get(&pick.asset.url)"),
    ("src/backends/web.rs", "client.get(&spec.name).send()"),
    ("src/backends/appimage.rs", "client.get(url).send()"),
];

fn source(rel: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel))
        .unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

#[test]
fn the_ownership_refusal_is_asked_before_the_first_byte_is_fetched() {
    for (rel, fetch) in BACKENDS {
        let src = source(rel);
        let guard = src
            .find("ensure_deployable(")
            .unwrap_or_else(|| panic!("{rel} never asks whether it may deploy before downloading"));

        // Without this the scan passes on a file it has stopped matching — the shape of check
        // this whole suite exists to replace.
        let first_fetch = src.find(fetch).unwrap_or_else(|| {
            panic!("{rel}: `{fetch}` no longer appears; this scan has stopped matching the code it audits")
        });

        assert!(
            guard < first_fetch,
            "{rel} downloads at byte {first_fetch} and only asks whether it may deploy at byte \
             {guard}. The refusal reads the destination and nothing else, so every byte before \
             it is spent on an artifact that may be thrown away — 180s of one `heal`, silent."
        );
    }
}

/// The refusal must stay one function with one wording. Two copies is how this directory came to
/// have opposite answers about the same `~/.local/bin` depending on which backend reached it
/// first — `deploy_executable` exists because the download backends each hand-rolled their own.
#[test]
fn the_pre_flight_and_the_deploy_ask_the_same_question() {
    let file = source("src/utils/file.rs");
    assert_eq!(
        file.matches("refusing to deploy `").count(),
        1,
        "the refusal is worded in more than one place, so the pre-flight and the deploy can \
         drift apart — which is the bug they exist to prevent, one level up"
    );
    let deploy = file
        .find("pub async fn deploy_executable")
        .expect("deploy_executable exists");
    let body = &file[deploy..];
    assert!(
        body[..body.find("\n}").unwrap_or(body.len())].contains("ensure_deployable("),
        "deploy_executable must route its refusal through `ensure_deployable`, or a backend \
         that asked early and one that did not are asking two different questions"
    );
}
