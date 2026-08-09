//! `shim:jq@source=cargo:jq` runs *cargo's* jq (Y18.3).
//!
//! `source` was a legal option the grammar listed and a test asserted parses, and no apply path
//! read it: the dependent phase called `create_shim(name)` and dropped the rest. So a line that
//! named a provider got whichever provider the resolver happened to pick for the bare name —
//! silently, which is the defect class this repo hunts, sitting inside the repo.
//!
//! The shim itself cannot carry the answer: it is a copy of the linix binary under another name,
//! with nowhere to keep data. It does not need to. The config that declared the shim is the same
//! config the shim process loads on its way in, and it still says what the source is.

mod mock_providers;
use mock_providers::TestKernel;

/// Write a module the active profile uses, so its lines reach the resolved model.
async fn declare(kernel: &TestKernel, lines: &str) {
    let root = kernel.app.config.config_root();
    tokio::fs::write(root.join("modules/tools.txt"), lines)
        .await
        .unwrap();
    tokio::fs::write(root.join("profiles/Main"), "use tools\n")
        .await
        .unwrap();
}

#[tokio::test]
async fn a_shim_line_with_a_source_runs_that_provider() {
    let kernel = TestKernel::new().await;
    declare(&kernel, "cargo:jq\nshim:jq@source=cargo:jq\n").await;

    assert_eq!(
        kernel.app.runner().shim_spec("jq").await.unwrap(),
        "cargo:jq",
        "the line named a provider and the shim ran whatever the bare name resolved to"
    );
}

/// The option is not required, and a shim without one keeps resolving the bare name — the
/// behaviour every existing `shim:` line already has.
#[tokio::test]
async fn a_shim_line_without_a_source_still_resolves_the_bare_name() {
    let kernel = TestKernel::new().await;
    declare(&kernel, "cargo:jq\nshim:jq\n").await;

    assert_eq!(kernel.app.runner().shim_spec("jq").await.unwrap(), "jq");
}

/// Each line answers for its own name. One `@source=` reaching a second shim would run the
/// wrong binary under a name that never asked for it.
#[tokio::test]
async fn one_line_does_not_answer_for_another_shims_name() {
    let kernel = TestKernel::new().await;
    declare(
        &kernel,
        "cargo:jq\ncargo:rg\nshim:jq@source=cargo:jq\nshim:rg\n",
    )
    .await;

    let runner = kernel.app.runner();
    assert_eq!(runner.shim_spec("jq").await.unwrap(), "cargo:jq");
    assert_eq!(runner.shim_spec("rg").await.unwrap(), "rg");
    // A name no line declares is not a shim at all, and resolving it must not inherit anyone
    // else's source.
    assert_eq!(runner.shim_spec("fd").await.unwrap(), "fd");
}

/// `@scope=` is the other option on this line and it is read at deploy time, not here — the
/// point being that `SHIM_OPTION_KEYS` no longer holds a key nothing reads.
#[tokio::test]
async fn every_option_a_shim_line_accepts_is_read_by_something() {
    let kernel = TestKernel::new().await;
    declare(&kernel, "cargo:jq\nshim:jq@source=cargo:jq,scope=user\n").await;

    assert_eq!(
        kernel.app.runner().shim_spec("jq").await.unwrap(),
        "cargo:jq",
        "a second option on the line must not displace the source"
    );
}
