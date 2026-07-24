//! D2: the classifier, run against the asset lists of real releases and checked by hand.
//!
//! The rule — an extension decides the format, and a name with no recognised extension that
//! matches this machine is `binary` — was ruled on reasoning and never tested against what
//! projects actually publish. It is the one part of artifact selection that fails quietly, so
//! the check is a fixture of real filenames rather than filenames someone imagined.

use super::format::{Format, FormatOrder};
use super::pattern::AssetPattern;
use super::platform::Platform;
use super::select::{classify_format, select, Asset, Request};

struct Block {
    repo: String,
    expected: Vec<(String, String, String)>,
    assets: Vec<Asset>,
}

fn blocks() -> Vec<Block> {
    let text = include_str!("real_releases.txt");
    let mut out: Vec<Block> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || (line.starts_with('#') && !line.starts_with("## ")) {
            continue;
        }
        if let Some(head) = line.strip_prefix("## ") {
            out.push(Block {
                repo: head.to_string(),
                expected: Vec::new(),
                assets: Vec::new(),
            });
        } else if let Some(exp) = line.strip_prefix("-> ") {
            let (target, want) = exp.split_once('=').expect("`-> os/arch = expected`");
            let (os, arch) = target.trim().split_once('/').expect("os/arch");
            let b = out.last_mut().expect("an expectation before any block");
            b.expected
                .push((os.to_string(), arch.to_string(), want.trim().to_string()));
        } else {
            let b = out.last_mut().expect("an asset before any block");
            b.assets
                .push(Asset::new(line, format!("https://example.test/{}", line)));
        }
    }
    assert!(out.len() >= 6, "the fixture lost its blocks");
    out
}

/// `linux` is checked as a debian machine because that is the default order that puts a system
/// package first, which is the one most likely to pick something surprising.
fn family_of(os: &str) -> Option<&'static str> {
    match os {
        "linux" => Some("debian"),
        _ => None,
    }
}

#[test]
fn every_real_release_resolves_to_the_asset_a_human_checked() {
    for b in blocks() {
        for (os, arch, want) in &b.expected {
            let platform = Platform::new(os.clone(), arch.clone());
            let formats = FormatOrder::detected_default(os, family_of(os));
            let request = Request {
                package: &b.repo,
                release: "fixture",
                platform: &platform,
                formats: &formats,
                pattern: None,
            };
            let got = match select(&request, &b.assets) {
                Ok(sel) => sel.picks[0].asset.name.clone(),
                Err(_) => "none".to_string(),
            };
            assert_eq!(&got, want, "{} on {}/{}", b.repo, os, arch);
        }
    }
}

/// The quiet one, stated separately because the test above would still pass if `binary` were
/// never chosen at all: across every real asset here, the only files LiNix would run as
/// executables are the ones whose names say which machine they are for.
#[test]
fn nothing_becomes_an_executable_without_naming_the_machine() {
    for b in blocks() {
        for (os, arch, _) in &b.expected {
            let platform = Platform::new(os.clone(), arch.clone());
            for asset in &b.assets {
                if Format::is_metadata_filename(&asset.name) {
                    continue;
                }
                if classify_format(&asset.name, &platform, None) == Some(Format::Binary) {
                    assert!(
                        platform.specificity(&asset.name) > 0,
                        "{} would be run as an executable on {}/{} while naming neither",
                        asset.name,
                        os,
                        arch
                    );
                }
            }
        }
    }
}

/// The two real files that made the rule stricter, named so the reason outlives the fixture:
/// `MD5SUMS` is in every rclone release and `jq-linux64` is in jq's.
#[test]
fn the_two_real_files_that_used_to_be_executables_are_not() {
    for (os, arch) in [
        ("linux", "x86_64"),
        ("windows", "x86_64"),
        ("macos", "aarch64"),
    ] {
        let p = Platform::new(os, arch);
        assert_ne!(
            classify_format("MD5SUMS", &p, None),
            Some(Format::Binary),
            "MD5SUMS on {}/{}",
            os,
            arch
        );
    }
    // `linux64` is a real spelling of "linux", so on Windows this is foreign now rather than
    // a nameless file that could run anywhere.
    assert!(!Platform::new("windows", "x86_64").accepts("jq-linux64"));
    assert!(Platform::new("linux", "x86_64").accepts("jq-linux64"));
    // ...and on Linux it is an executable, because there it does name the machine.
    assert_eq!(
        classify_format("jq-linux64", &Platform::new("linux", "x86_64"), None),
        Some(Format::Binary)
    );
}

/// A release whose only assets are signatures has nothing to install, and must say so rather
/// than pick one. helm publishes exactly this.
#[test]
fn a_release_of_nothing_but_signatures_is_an_error() {
    let b = blocks()
        .into_iter()
        .find(|b| b.repo.starts_with("helm/"))
        .expect("the helm block");
    let platform = Platform::new("linux", "x86_64");
    let formats = FormatOrder::detected_default("linux", Some("debian"));
    let err = select(
        &Request {
            package: "helm/helm",
            release: "fixture",
            platform: &platform,
            formats: &formats,
            pattern: None,
        },
        &b.assets,
    )
    .expect_err("signatures are not installable");
    assert!(err.to_string().contains("no asset matches"), "{}", err);
}

/// The escape hatch the stricter rule needs: a project shipping one bare executable for one
/// platform is installed by naming it, because naming it is the claim.
#[test]
fn naming_an_asset_exactly_installs_it_even_when_the_name_says_nothing() {
    let assets = vec![Asset::new("mytool", "https://example.test/mytool")];
    let platform = Platform::new("linux", "x86_64");
    let formats = FormatOrder::detected_default("linux", None);
    let pattern = AssetPattern::parse("mytool").unwrap();
    let picked = select(
        &Request {
            package: "acme/mytool",
            release: "fixture",
            platform: &platform,
            formats: &formats,
            pattern: Some(&pattern),
        },
        &assets,
    )
    .expect("an exactly-named asset installs");
    assert_eq!(picked.picks[0].format, Format::Binary);

    // A glob is not a claim about the platform, so it does not open the same door.
    let glob = AssetPattern::parse("my*").unwrap();
    assert!(select(
        &Request {
            package: "acme/mytool",
            release: "fixture",
            platform: &platform,
            formats: &formats,
            pattern: Some(&glob),
        },
        &assets,
    )
    .is_err());
}

