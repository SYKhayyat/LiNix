//! A repeated option key reaches the backend as a list, and a `;` in a value is data.
//!
//! `PackageSpec.options` was `HashMap<String, String>`, so `to_spec` joined every value the
//! grammar had kept as a `Vec` with `;` — under a comment reading *"`requires` is a list; the
//! rest are single values. Joined with `;` because that is what the planner already splits on."*
//!
//! **That comment was wrong twice.** II.2 makes *any* repeated key a list, and
//! `validate_setting` refuses two values by counting them — so the grammar knew. And three lines
//! below the join, `requires: options.all("requires").to_vec()` gave the one list somebody
//! remembered a real `Vec<String>`. Every other list became a delimiter that nothing validated
//! and that `ArtifactOptions::read`, `ChangePlanner::in_scope` and `insight::gating_of` each
//! split back apart in their own way.
//!
//! Two properties, and the second is the one a delimiter can never have:
//!
//! 1. **A key given twice arrives twice**, in the order it was written.
//! 2. **A value containing the delimiter is one value.** Under the old model `@bin=a;b` was two
//!    formats to the artifact reader, one scope to the planner, and a single string to whoever
//!    printed it — three answers to one question, decided by which layer split last.

use linix::config::grammar::options::parse_short;
use linix::config::grammar::{Origin, Selector};
use linix::core::PackageSpec;
use linix::model::resolve::{to_spec, Provenance};

fn origin() -> Origin {
    Origin::new(std::path::Path::new("modules/dev.txt"), 1)
}

/// Build the spec the resolver would build for one declaration.
fn spec_for(tag: &str) -> PackageSpec {
    let at = origin();
    let options = parse_short(&at, tag).expect("the tag parses");
    to_spec(
        "github",
        &Selector::Name("ripgrep".into()),
        &options,
        true,
        None,
        Provenance {
            origin: &at,
            scopes: &["module:dev".to_string(), "profile:Work".to_string()],
            gates: &[],
        },
    )
}

#[test]
fn a_key_given_twice_arrives_as_two_values_in_order() {
    let spec = spec_for("formats=deb");
    assert_eq!(spec.options.all("formats"), &["deb".to_string()]);

    let at = origin();
    let mut options = parse_short(&at, "formats=deb").expect("parses");
    options.insert("formats", "tarball");
    let spec = to_spec(
        "github",
        &Selector::Name("ripgrep".into()),
        &options,
        true,
        None,
        Provenance {
            origin: &at,
            scopes: &[],
            gates: &[],
        },
    );
    assert_eq!(
        spec.options.all("formats"),
        &["deb".to_string(), "tarball".to_string()],
        "the second value was lost or merged — this is the `;`-join, back"
    );
}

/// The property a delimiter cannot have. Under the old model this value was two formats to
/// `ArtifactOptions::read` and one string to everyone else.
#[test]
fn a_semicolon_inside_a_value_is_data_and_not_a_separator() {
    let spec = spec_for("bin=weird;name");
    assert_eq!(spec.options.all("bin"), &["weird;name".to_string()]);
    assert_eq!(spec.options.one("bin"), Some("weird;name"));

    let read = linix::backends::artifact::ArtifactOptions::read(&spec.options)
        .expect("the artifact reader accepts it");
    assert_eq!(
        read.bin.as_deref(),
        Some("weird;name"),
        "the artifact reader split a value that was never a list"
    );
}

/// `__scopes` is genuinely plural — a package a module holds and a profile reaches belongs to
/// both — and it was a `;`-joined string every reader split for itself.
#[test]
fn the_scopes_tag_is_a_list_and_not_a_joined_string() {
    let spec = spec_for("hold");
    assert_eq!(
        spec.options.all("__scopes"),
        &["module:dev".to_string(), "profile:Work".to_string()]
    );
    assert_eq!(
        spec.options.one("__scopes"),
        Some("module:dev"),
        "`one` on a plural key yields the first, and callers that care use `all`"
    );
}

/// `requires` was the one list that already had a real `Vec<String>`, because somebody
/// remembered. It still does, and now it is not the exception.
#[test]
fn requires_still_lands_in_its_own_vec() {
    let at = origin();
    let mut options = parse_short(&at, "requires=curl").expect("parses");
    options.insert("requires", "jq");
    let spec = to_spec(
        "github",
        &Selector::Name("ripgrep".into()),
        &options,
        true,
        None,
        Provenance {
            origin: &at,
            scopes: &[],
            gates: &[],
        },
    );
    assert_eq!(spec.requires, vec!["curl".to_string(), "jq".to_string()]);
    assert_eq!(
        spec.options.all("requires"),
        &["curl".to_string(), "jq".to_string()],
        "the options map should carry it too — the `Vec` field is a convenience, not the source"
    );
}

/// A spec round-trips through JSON with its lists intact. Saved plans are written and read as
/// JSON, and the old shape wrote `\"formats\": \"deb;tarball\"` — a string a reader had to know to
/// split, in a file whose whole purpose is to be read later by something that was not there.
#[test]
fn a_saved_spec_round_trips_with_its_lists() {
    let at = origin();
    let mut options = parse_short(&at, "formats=deb").expect("parses");
    options.insert("formats", "tarball");
    let spec = to_spec(
        "github",
        &Selector::Name("ripgrep".into()),
        &options,
        true,
        None,
        Provenance {
            origin: &at,
            scopes: &[],
            gates: &[],
        },
    );
    let json = serde_json::to_string(&spec).expect("serialises");
    assert!(
        json.contains(r#"["deb","tarball"]"#),
        "the list was flattened on the way to disk: {json}"
    );
    let back: PackageSpec = serde_json::from_str(&json).expect("deserialises");
    assert_eq!(back.options.all("formats"), spec.options.all("formats"));
}
