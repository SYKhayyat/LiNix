//! **The three backends that download report a package under the key their state file uses.**
//!
//! `github:`, `web:` and `appimage:` are the only backends whose "installed" answer comes from a
//! JSON file LiNix wrote rather than from a manager. That makes the identity rule theirs to get
//! right, and one of them got it wrong in the expensive direction: `appimage:` reported the
//! *basename* while keying its state by the *URL*, so `info(url)` never matched its own record.
//! Every declared AppImage read as absent, `sync` re-downloaded all of them on every run for
//! ever, and a removal could never find the row it was meant to delete. Its own doc comment
//! records the fix and names `btrfs:` and `web:` as the same shape.
//!
//! One rule, asserted the same way three times: **whatever `fetch_installed` calls a package,
//! `remove` must find under that name.** A test per backend, because they have three different
//! record shapes and the drift is always in one of them.

use std::path::PathBuf;

use linix::app::sync::guard::{GuardScope, Reaped};
use linix::backends::{appimage, github, web};
use linix::core::{CommandExecutor, Installable, Queryable};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "linix-artifact-identity-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temp directory");
    dir
}

fn exec() -> CommandExecutor {
    CommandExecutor::new(false, false)
}

/// Names reported, sorted, so a map's iteration order is not part of the assertion.
fn reported(pkgs: Vec<linix::core::Package>) -> Vec<String> {
    let mut names: Vec<String> = pkgs.into_iter().map(|p| p.name).collect();
    names.sort();
    names
}

const URL: &str = "https://example.invalid/downloads/tool-1.2.3.AppImage";

#[tokio::test]
async fn an_appimage_is_reported_under_the_url_its_state_is_keyed_by() {
    let dir = scratch("appimage");
    std::fs::write(
        dir.join("state.json"),
        format!(
            r#"{{"{URL}": {{"url": "{URL}", "local_path": "{}", "symlink_path": ""}}}}"#,
            dir.join("tool").display().to_string().replace('\\', "/")
        ),
    )
    .expect("write state");

    let core = std::sync::Arc::new(appimage::AppImageBackendCore::new(
        exec(),
        dir.clone(),
        dir.join("bin"),
        true,
        false,
        vec![],
    ));
    let q = appimage::AppImageQueryable { core: core.clone() };
    assert_eq!(
        reported(q.fetch_installed().await.expect("a readable state file")),
        vec![URL.to_string()],
        "the basename here is the bug: `sync` compares this against the declaration, which is \
         the URL"
    );

    // …and the name it reported is a name removal answers to. A no-op removal is the failure:
    // it means the record was not found and the file stays on disk with no state row.
    let i = appimage::AppImageInstallable { core };
    i.remove(&[URL.to_string()], false, Reaped::for_reason(GuardScope::Sync, "an identity test for the effector, not for the guard"))
        .await
        .expect("removing a recorded AppImage");
    assert_eq!(
        reported(q.fetch_installed().await.expect("a readable state file")),
        Vec::<String>::new(),
        "the record survived a removal by the name the backend itself reported"
    );
}

#[tokio::test]
async fn a_web_resource_is_reported_under_the_url_its_state_is_keyed_by() {
    let dir = scratch("web");
    std::fs::write(
        dir.join("installed.json"),
        format!(
            r#"{{"{URL}": {{"url": "{URL}", "local_path": "{}", "bin_link": null,
                "etag": null, "last_modified": null}}}}"#,
            dir.join("tool").display().to_string().replace('\\', "/")
        ),
    )
    .expect("write state");

    let core = std::sync::Arc::new(web::WebBackendCore::new(
        exec(),
        dir.clone(),
        dir.join("bin"),
        true,
        false,
        vec![],
    ));
    let q = web::WebQueryable { core: core.clone() };
    assert_eq!(
        reported(q.fetch_installed().await.expect("a readable state file")),
        vec![URL.to_string()]
    );

    let i = web::WebInstallable { core };
    i.remove(&[URL.to_string()], false, Reaped::for_reason(GuardScope::Sync, "an identity test for the effector, not for the guard"))
        .await
        .expect("removing a recorded web resource");
    assert_eq!(
        reported(q.fetch_installed().await.expect("a readable state file")),
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn a_github_package_is_reported_under_the_repo_its_state_is_keyed_by() {
    let dir = scratch("github");
    let repo = "BurntSushi/ripgrep";
    std::fs::write(
        dir.join("installed.json"),
        format!(
            r#"{{"{repo}": {{"repo": "{repo}", "version": "14.1.0", "install_path": "{}",
                "artifacts": [{{"asset": "ripgrep-14.1.0.tar.gz", "format": "TarGz",
                "bin_path": null}}]}}}}"#,
            dir.join("ripgrep").display().to_string().replace('\\', "/")
        ),
    )
    .expect("write state");

    let core = std::sync::Arc::new(github::GithubBackendCore::new(
        exec(),
        dir.clone(),
        dir.join("bin"),
        dir.join("locks.toml"),
        true,
        false,
        vec![],
        None,
        std::time::Duration::from_secs(0),
    ));
    let q = github::GithubQueryable { core: core.clone() };
    assert_eq!(
        reported(q.fetch_installed().await.expect("a readable state file")),
        vec![repo.to_string()]
    );

    let i = github::GithubInstallable { core };
    i.remove(&[repo.to_string()], false, Reaped::for_reason(GuardScope::Sync, "an identity test for the effector, not for the guard"))
        .await
        .expect("removing a recorded GitHub package");
    assert_eq!(
        reported(q.fetch_installed().await.expect("a readable state file")),
        Vec::<String>::new()
    );
}
