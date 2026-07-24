//! Choosing which file of a release to install.
//!
//! Nothing here touches the network. The selector is given the assets a release offers and
//! returns the choice plus what it passed over, so the plan can show the decision and the lock
//! can record it. A selection that cannot be explained is a selection that drifts.

use super::format::{Format, FormatOrder};
use super::pattern::AssetPattern;
use super::platform::Platform;
use std::fmt;

/// One file a release offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

impl Asset {
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Asset {
            name: name.into(),
            url: url.into(),
        }
    }
}

/// An asset the selector chose, with the format it was chosen as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    pub asset: Asset,
    pub format: Format,
}

/// An asset that could have been installed and was not. Only assets that survive the platform
/// and format filters appear here — the ones this machine cannot run are noise, not a choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassedOver {
    pub name: String,
    pub format: Format,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub picks: Vec<Pick>,
    pub passed_over: Vec<PassedOver>,
}

impl Selection {
    /// Whether the choice came down to a tie-break rather than being forced. The plan says so
    /// out loud when it did: a guess that is printed is not the silent guess.
    pub fn was_ambiguous(&self) -> bool {
        !self.passed_over.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rejection {
    ForeignPlatform,
    UnwantedFormat,
    UnmatchedPattern,
    Unrecognised,
}

impl Rejection {
    fn describe(self) -> &'static str {
        match self {
            Rejection::ForeignPlatform => "not for this machine",
            Rejection::UnwantedFormat => "format not in your list",
            Rejection::UnmatchedPattern => "does not match @asset",
            Rejection::Unrecognised => "unrecognised file type",
        }
    }
}

#[derive(Debug, Clone)]
struct Rejected {
    name: String,
    format: Option<Format>,
    why: Rejection,
}

/// Everything the selector needs, and nothing it does not. Constructed by the caller so this
/// module never reaches for global state or the environment.
pub struct Request<'a> {
    pub package: &'a str,
    pub release: &'a str,
    pub platform: &'a Platform,
    pub formats: &'a FormatOrder,
    pub pattern: Option<&'a AssetPattern>,
}

#[derive(Debug, Clone)]
pub struct NoMatch {
    package: String,
    release: String,
    formats: String,
    pattern: Option<String>,
    rejected: Vec<(String, Option<Format>, &'static str)>,
}

impl fmt::Display for NoMatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{} — no asset matches your formats.", self.package)?;
        writeln!(f, "  wanted:  {}", self.formats)?;
        if let Some(p) = &self.pattern {
            writeln!(f, "  @asset:  {}", p)?;
        }
        if self.rejected.is_empty() {
            writeln!(f, "  release {} offers no assets at all.", self.release)?;
        } else {
            writeln!(f, "  release {} offers:", self.release)?;
            let width = self
                .rejected
                .iter()
                .map(|(n, _, _)| n.len())
                .max()
                .unwrap_or(0);
            for (name, format, why) in &self.rejected {
                let shape = format.map(|f| f.to_string()).unwrap_or_else(|| "-".into());
                writeln!(f, "    {:width$}  {:<9} ({})", name, shape, why, width = width)?;
            }
        }
        write!(
            f,
            "  add the format you want to `priority`, or pin one with @formats= or @asset=."
        )
    }
}

impl std::error::Error for NoMatch {}

/// A candidate that survived every filter, carried with the facts the tie-break needs.
struct Candidate {
    asset: Asset,
    format: Format,
    rank: usize,
    specificity: u8,
}

pub fn select(request: &Request<'_>, assets: &[Asset]) -> Result<Selection, NoMatch> {
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut rejected: Vec<Rejected> = Vec::new();

    for asset in assets {
        if Format::is_metadata_filename(&asset.name) {
            continue;
        }
        let runs_here = request.platform.accepts(&asset.name);
        let format = classify_format(&asset.name, request.platform, request.pattern);

        if !runs_here {
            rejected.push(Rejected {
                name: asset.name.clone(),
                format,
                why: Rejection::ForeignPlatform,
            });
            continue;
        }
        let Some(format) = format else {
            rejected.push(Rejected {
                name: asset.name.clone(),
                format: None,
                why: Rejection::Unrecognised,
            });
            continue;
        };
        if let Some(pattern) = request.pattern {
            if !pattern.matches(&asset.name) {
                rejected.push(Rejected {
                    name: asset.name.clone(),
                    format: Some(format),
                    why: Rejection::UnmatchedPattern,
                });
                continue;
            }
        }
        let Some(rank) = request.formats.rank(format) else {
            rejected.push(Rejected {
                name: asset.name.clone(),
                format: Some(format),
                why: Rejection::UnwantedFormat,
            });
            continue;
        };
        candidates.push(Candidate {
            asset: asset.clone(),
            format,
            rank,
            specificity: request.platform.specificity(&asset.name),
        });
    }

    if candidates.is_empty() {
        return Err(NoMatch {
            package: request.package.to_string(),
            release: request.release.to_string(),
            formats: request.formats.to_string(),
            pattern: request.pattern.map(|p| p.to_string()),
            rejected: rejected
                .into_iter()
                .map(|r| (r.name, r.format, r.why.describe()))
                .collect(),
        });
    }

    sort_by_preference(&mut candidates);

    if request
        .pattern
        .is_some_and(AssetPattern::installs_every_match)
    {
        return Ok(Selection {
            picks: candidates
                .into_iter()
                .map(|c| Pick {
                    asset: c.asset,
                    format: c.format,
                })
                .collect(),
            passed_over: Vec::new(),
        });
    }

    let mut candidates = candidates.into_iter();
    let winner = candidates.next().expect("candidates is non-empty");
    let passed_over = candidates
        .map(|c| PassedOver {
            name: c.asset.name,
            format: c.format,
        })
        .collect();

    Ok(Selection {
        picks: vec![Pick {
            asset: winner.asset,
            format: winner.format,
        }],
        passed_over,
    })
}

/// The tie-break, in one place so there is exactly one answer to "two assets, both legal":
/// the format you asked for first, then the asset that names this machine most explicitly,
/// then the shortest filename, then alphabetical so the result never depends on the order
/// GitHub happened to return.
fn sort_by_preference(candidates: &mut [Candidate]) {
    candidates.sort_by(|a, b| {
        a.rank
            .cmp(&b.rank)
            .then(b.specificity.cmp(&a.specificity))
            .then(a.asset.name.len().cmp(&b.asset.name.len()))
            .then(a.asset.name.cmp(&b.asset.name))
    });
}

/// An asset with no recognised extension is a `Binary` only if the filename **names this
/// machine** and its trailing dot-segment is not a file extension.
///
/// D2's rule is *"matched this machine's os/arch and has no recognised extension"*, and
/// "matched" is stronger than `accepts`, which only says the name does not contradict this
/// machine. Checked against real releases, the weaker reading made `MD5SUMS` — an actual asset
/// of every rclone release — an executable candidate on every platform, and `jq-linux64` one
/// on Windows. This is the one place in artifact selection that fails quietly rather than
/// loudly, so it takes the strict reading and a file that claims nothing is not an executable.
///
/// **`@asset=` overrides it**, because naming the file *is* the claim: a project shipping one
/// bare `mytool` for one platform is installed by naming it, not by LiNix guessing.
pub(super) fn classify_format(
    name: &str,
    platform: &Platform,
    pattern: Option<&AssetPattern>,
) -> Option<Format> {
    if let Some(format) = Format::of_filename(name) {
        return Some(format);
    }
    if has_extension(name) {
        return None;
    }
    if platform.specificity(name) > 0 {
        return Some(Format::Binary);
    }
    if pattern.is_some_and(|p| p.names_exactly(name)) {
        return Some(Format::Binary);
    }
    None
}

fn has_extension(name: &str) -> bool {
    let tail = name.rsplit('/').next().unwrap_or(name);
    match tail.rsplit_once('.') {
        None => false,
        Some((stem, ext)) => {
            !stem.is_empty()
                && (1..=5).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
                && ext.chars().any(|c| c.is_ascii_alphabetic())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linux64() -> Platform {
        Platform::new("linux", "x86_64")
    }

    fn assets(names: &[&str]) -> Vec<Asset> {
        names
            .iter()
            .map(|n| Asset::new(*n, format!("https://example.test/{}", n)))
            .collect()
    }

    fn request<'a>(
        formats: &'a FormatOrder,
        platform: &'a Platform,
        pattern: Option<&'a AssetPattern>,
    ) -> Request<'a> {
        Request {
            package: "sharkdp/fd",
            release: "v10.2.0",
            platform,
            formats,
            pattern,
        }
    }

    #[test]
    fn the_first_format_in_the_order_wins_over_a_later_one() {
        let formats = FormatOrder::new(vec![Format::Deb, Format::Tarball]);
        let platform = linux64();
        let found = select(
            &request(&formats, &platform, None),
            &assets(&["fd-linux-x86_64.tar.gz", "fd_10.2.0_amd64.deb"]),
        )
        .unwrap();
        assert_eq!(found.picks[0].format, Format::Deb);
        assert_eq!(found.picks[0].asset.name, "fd_10.2.0_amd64.deb");
    }

    #[test]
    fn a_later_format_is_a_fallback_not_an_addition() {
        let formats = FormatOrder::new(vec![Format::Deb, Format::Tarball]);
        let platform = linux64();
        let found = select(
            &request(&formats, &platform, None),
            &assets(&["fd-linux-x86_64.tar.gz"]),
        )
        .unwrap();
        assert_eq!(found.picks.len(), 1);
        assert_eq!(found.picks[0].format, Format::Tarball);
    }

    #[test]
    fn the_d3_case_shortest_filename_wins_and_says_so() {
        let formats = FormatOrder::new(vec![Format::Deb]);
        let platform = linux64();
        let found = select(
            &request(&formats, &platform, None),
            &assets(&["fd-musl_10.2.0_amd64.deb", "fd_10.2.0_amd64.deb"]),
        )
        .unwrap();
        assert_eq!(found.picks[0].asset.name, "fd_10.2.0_amd64.deb");
        assert!(found.was_ambiguous());
        assert_eq!(found.passed_over[0].name, "fd-musl_10.2.0_amd64.deb");
    }

    #[test]
    fn an_asset_pattern_narrows_to_the_variant() {
        let formats = FormatOrder::new(vec![Format::Deb]);
        let platform = linux64();
        let pattern = AssetPattern::parse("*musl*").unwrap();
        let found = select(
            &request(&formats, &platform, Some(&pattern)),
            &assets(&["fd-musl_10.2.0_amd64.deb", "fd_10.2.0_amd64.deb"]),
        )
        .unwrap();
        assert_eq!(found.picks[0].asset.name, "fd-musl_10.2.0_amd64.deb");
        assert!(!found.was_ambiguous());
    }

    #[test]
    fn a_pattern_narrows_but_does_not_select() {
        let formats = FormatOrder::new(vec![Format::Deb]);
        let platform = linux64();
        let pattern = AssetPattern::parse("*amd64*").unwrap();
        let found = select(
            &request(&formats, &platform, Some(&pattern)),
            &assets(&["fd-musl_10.2.0_amd64.deb", "fd_10.2.0_amd64.deb"]),
        )
        .unwrap();
        assert_eq!(found.picks.len(), 1, "still one pick, tie-broken as usual");
        assert_eq!(found.picks[0].asset.name, "fd_10.2.0_amd64.deb");
    }

    #[test]
    fn asset_all_installs_every_match_instead_of_choosing() {
        let formats = FormatOrder::new(vec![Format::Deb, Format::Tarball]);
        let platform = linux64();
        let pattern = AssetPattern::parse("all").unwrap();
        let found = select(
            &request(&formats, &platform, Some(&pattern)),
            &assets(&["fd_10.2.0_amd64.deb", "fd-linux-x86_64.tar.gz"]),
        )
        .unwrap();
        assert_eq!(found.picks.len(), 2);
        assert!(!found.was_ambiguous(), "nothing was passed over");
    }

    #[test]
    fn a_foreign_asset_is_never_chosen_even_as_a_last_resort() {
        let formats = FormatOrder::new(vec![Format::Zip]);
        let platform = linux64();
        let err = select(
            &request(&formats, &platform, None),
            &assets(&["fd-x86_64-pc-windows-msvc.zip"]),
        )
        .unwrap_err();
        assert!(err.to_string().contains("not for this machine"));
    }

    #[test]
    fn no_match_reports_what_the_release_actually_offered() {
        let formats = FormatOrder::new(vec![Format::Deb, Format::AppImage]);
        let platform = linux64();
        let err = select(
            &request(&formats, &platform, None),
            &assets(&[
                "fd-v10.2.0-x86_64-unknown-linux-gnu.tar.gz",
                "fd_10.2.0_arm64.deb",
            ]),
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("sharkdp/fd"));
        assert!(msg.contains("deb, appimage"));
        assert!(msg.contains("fd-v10.2.0-x86_64-unknown-linux-gnu.tar.gz"));
        assert!(msg.contains("format not in your list"));
        assert!(msg.contains("fd_10.2.0_arm64.deb"));
        assert!(msg.contains("not for this machine"));
    }

    #[test]
    fn an_extensionless_asset_is_a_binary() {
        let formats = FormatOrder::new(vec![Format::Binary]);
        let platform = linux64();
        let found = select(&request(&formats, &platform, None), &assets(&["fd-linux"])).unwrap();
        assert_eq!(found.picks[0].format, Format::Binary);
    }

    #[test]
    fn a_version_number_in_the_name_is_not_an_extension() {
        let formats = FormatOrder::new(vec![Format::Binary]);
        let platform = linux64();
        let found = select(
            &request(&formats, &platform, None),
            &assets(&["fd-v10.2.0-x86_64-linux"]),
        )
        .unwrap();
        assert_eq!(found.picks[0].asset.name, "fd-v10.2.0-x86_64-linux");
    }

    #[test]
    fn release_notes_are_not_mistaken_for_an_executable() {
        let formats = FormatOrder::new(vec![Format::Binary]);
        let platform = linux64();
        let err = select(
            &request(&formats, &platform, None),
            &assets(&["release-notes.txt"]),
        )
        .unwrap_err();
        assert!(err.to_string().contains("unrecognised file type"));
    }

    #[test]
    fn checksums_and_signatures_are_not_offered_as_choices() {
        let formats = FormatOrder::new(vec![Format::Deb]);
        let platform = linux64();
        let found = select(
            &request(&formats, &platform, None),
            &assets(&[
                "fd_10.2.0_amd64.deb",
                "fd_10.2.0_amd64.deb.sha256",
                "checksums.txt",
            ]),
        )
        .unwrap();
        assert_eq!(found.picks[0].asset.name, "fd_10.2.0_amd64.deb");
        assert!(!found.was_ambiguous());
    }

    #[test]
    fn an_explicit_target_beats_a_shorter_silent_one() {
        let formats = FormatOrder::new(vec![Format::Tarball]);
        let platform = linux64();
        let found = select(
            &request(&formats, &platform, None),
            &assets(&["fd.tar.gz", "fd-linux-x86_64.tar.gz"]),
        )
        .unwrap();
        assert_eq!(found.picks[0].asset.name, "fd-linux-x86_64.tar.gz");
    }

    #[test]
    fn selection_does_not_depend_on_the_order_the_api_returned() {
        let formats = FormatOrder::new(vec![Format::Deb]);
        let platform = linux64();
        let forwards = select(
            &request(&formats, &platform, None),
            &assets(&["fd-a_amd64.deb", "fd-b_amd64.deb"]),
        )
        .unwrap();
        let backwards = select(
            &request(&formats, &platform, None),
            &assets(&["fd-b_amd64.deb", "fd-a_amd64.deb"]),
        )
        .unwrap();
        assert_eq!(forwards.picks, backwards.picks);
    }

    #[test]
    fn a_release_with_no_assets_says_so_rather_than_listing_nothing() {
        let formats = FormatOrder::new(vec![Format::Deb]);
        let platform = linux64();
        let err = select(&request(&formats, &platform, None), &[]).unwrap_err();
        assert!(err.to_string().contains("no assets at all"));
    }
}

