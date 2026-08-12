//! **A manager that can remove software without forgetting it needs a lister that can tell.**
//!
//! `shall list` reported an apt package as installed after Shall itself removed it, `check` saw
//! no drift, and `sync` refused to reinstall it — permanently, on every apt package that ships a
//! file under `/etc` (B0). The lister was `dpkg-query -W -f='${Package} ${Version}\n'`, which has
//! no status filter: it lists every package dpkg *knows about*, and dpkg keeps knowing about one
//! after `apt remove`, in the state `deinstall ok config-files`.
//!
//! The removal was right. `apt remove` rather than `apt purge` keeps the user's configuration,
//! which is the safe choice and stays. The question this file asks is the general one:
//!
//! > Does this manager have a state its listing reports and which is not "installed", and does
//! > our lister exclude it?
//!
//! **The marker for that class is already in the model.** A manager declares `purge_args` exactly
//! when it distinguishes *remove the software* from *remove its configuration too* — which is
//! the distinction that creates a listed-but-absent state in the first place. So the enumeration
//! is not "read 52 listers and hope"; it is "every manager that declares a purge, and why its
//! lister cannot lie". A manager added tomorrow with a purge and a naive lister fails here.

use std::collections::BTreeSet;
use std::path::PathBuf;

use shall::parsers::apt;

fn read(rel: &str) -> String {
    let p: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
        .replace("\r\n", "\n")
}

/// Every manager that declares a purge, with the reason its installed-listing cannot report a
/// removed package as present. Adding a manager with a purge means adding it here, having first
/// answered the question in the header for it.
const AUDITED_PURGERS: &[(&str, &str)] = &[(
    "apt",
    "the lister asks dpkg for `${db:Status-Status}` and keeps only the words that mean the \
     software is on the machine, so `config-files` — what `apt remove` leaves — is excluded",
)];

/// The scan that makes the table above a gate rather than a note.
///
/// Source text rather than a live registry, because the failure this guards against is a *row*
/// being written without the question being asked, and a row is text before it is a backend.
#[test]
fn every_manager_that_can_purge_has_been_asked_whether_its_lister_can_lie() {
    let src = crate::harness::registry_source();

    // The hand-written registrations: find each `purge_args: Some(` and walk back to the
    // `fn register_*` it sits inside.
    let mut declared: BTreeSet<String> = BTreeSet::new();
    for (at, _) in src.match_indices("purge_args: Some(") {
        let enclosing = src[..at]
            .rmatch_indices("fn register_")
            .next()
            .map(|(i, _)| {
                src[i + "fn register_".len()..]
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
                    .unwrap_or_default()
                    .to_string()
            })
            .expect("a `purge_args: Some(` outside any registrar — this scan cannot place it");
        declared.insert(enclosing.replace('_', "-"));
    }

    // The data rows get the same question. `purge` is not a column today; if it becomes one,
    // this catches the first row that uses it rather than the first user who notices.
    let toml = read("src/backends/builtin_backends.toml");
    for line in toml.lines() {
        assert!(
            !line.trim_start().starts_with("purge"),
            "a row in builtin_backends.toml declares a purge:\n  {line}\nAdd it to \
             AUDITED_PURGERS with the reason its lister cannot report a removed package as \
             installed, or B0 has a second instance."
        );
    }

    let audited: BTreeSet<String> = AUDITED_PURGERS.iter().map(|(n, _)| n.to_string()).collect();
    assert_eq!(
        declared, audited,
        "the managers that declare a purge and the managers audited for B0 disagree. A manager \
         that can remove software without forgetting it can report the leftover as installed; \
         answer that question for it and record the answer in AUDITED_PURGERS."
    );
}

/// The audit's content, for the one member it currently has.
///
/// Asserted on the argv rather than on behaviour, because behaviour needs a Debian box and this
/// is the field a tidying edit would delete for reading badly.
#[test]
fn apts_lister_asks_dpkg_for_the_status_word() {
    let src = crate::harness::registry_source();
    assert!(
        src.contains(r"-f=${db:Status-Status} ${Package} ${Version}\\n"),
        "apt's `list_args` no longer asks for the status word. Without it `dpkg-query -W` \
         reports every package dpkg knows about, including the ones `apt remove` left in \
         `config-files` — which is B0."
    );
}

/// The dpkg states, and which of them mean the software is usable.
///
/// Table-driven over **every** status word dpkg can emit rather than over the one that was
/// reported, because the reported one (`config-files`) is a representative: the same listing
/// carries half-installed and partially-configured packages, and reading either as present is
/// the same defect wearing a different word.
#[test]
fn only_the_dpkg_states_that_mean_installed_are_read_as_installed() {
    // Verbatim shape of `dpkg-query -W -f='${db:Status-Status} ${Package} ${Version}\n'`.
    let listing = "\
installed bash 5.2.21-2ubuntu4
config-files figlet 2.2.5-3
not-installed removed-and-purged \n\
half-installed broken-a 1.0
unpacked broken-b 2.0
half-configured broken-c 3.0
triggers-awaited man-db 2.12.0-4build2
triggers-pending desktop-file-utils 0.27-2build1
installed curl 8.5.0-2ubuntu10.6
";
    let read: BTreeSet<String> = apt::parse_list(listing)
        .expect("this is a well-formed listing")
        .into_iter()
        .map(|p| p.name)
        .collect();

    for present in ["bash", "curl"] {
        assert!(
            read.contains(present),
            "`{present}` is installed and was dropped"
        );
    }
    // The finding itself: `apt remove` leaves this state and the listing must not read it.
    assert!(
        !read.contains("figlet"),
        "a package in `config-files` is what `apt remove` leaves behind — reading it as \
         installed is B0, and it makes the package permanently unreinstallable"
    );
    for absent in ["removed-and-purged", "broken-a", "broken-b", "broken-c"] {
        assert!(
            !read.contains(absent),
            "`{absent}` is not usable software and was read as installed"
        );
    }
    // The other direction, which a narrower fix would have broken: a deferred trigger is an
    // installed package. Calling these absent would make every sync reinstall them.
    for pending in ["man-db", "desktop-file-utils"] {
        assert!(
            read.contains(pending),
            "`{pending}` is installed with a trigger deferred; reading it as absent makes \
             `sync` reinstall a package that is already there"
        );
    }
}

/// A version is still read, and a status word is not mistaken for one.
#[test]
fn the_status_word_is_not_read_as_part_of_the_package() {
    let pkgs = apt::parse_list("installed bash 5.2.21-2ubuntu4\n").expect("one row");
    assert_eq!(pkgs.len(), 1);
    assert_eq!(pkgs[0].name, "bash");
    assert_eq!(pkgs[0].version.as_deref(), Some("5.2.21-2ubuntu4"));
}

/// The other half of the same argv change, and a finding this repo had already written down as
/// unaddressed: apt's error output used to parse as packages.
///
/// `E: Could not open lock file` split on its first space into a package named `E:` at version
/// *"Could not open lock file"*. Requiring a row to open with a status word dpkg can actually
/// emit makes junk unreadable instead of believed — so the listing now refuses rather than
/// reporting a machine with two packages on it.
#[test]
fn apts_own_error_output_is_not_a_listing_of_packages() {
    let e = apt::parse_list("E: Could not open lock file\nE: Are you root?\n")
        .expect_err("apt's error output is not a package listing");
    assert_eq!(e.backend, "apt");
    assert_eq!(e.data_lines, 2);
}

/// The two managers the review read rather than drove, kept honest by construction.
///
/// Neither declares a purge, so neither is in `AUDITED_PURGERS` — but both have a state their
/// listing could have included and does not, and both are one flag away from B0. The flag is
/// what is asserted, with the state it excludes.
#[test]
fn snap_and_flatpak_ask_narrow_questions_and_must_keep_asking_them() {
    let snap = read("src/backends/snap.rs");
    assert!(
        snap.contains(r#"&["list"]"#),
        "snap's installed-listing changed shape"
    );
    assert!(
        !snap.contains(r#"&["list", "--all"]"#),
        "`snap list --all` includes disabled revisions — snaps that are on disk and not the \
         running version. Reading those as installed is B0 on snap."
    );

    let flatpak = read("src/backends/flatpak.rs");
    assert!(
        flatpak.contains(r#""list", "--app""#),
        "`flatpak list` without `--app` includes runtimes, which nobody declared and which \
         would then read as installed packages"
    );
}
