//! Reading the artifact options off a resolved `PackageSpec`.
//!
//! One place turns option strings into the selector's types, so a backend never parses
//! `formats` itself and the grammar's validation and the backend's reading cannot disagree.

use super::format::FormatOrder;
use super::pattern::AssetPattern;
use crate::config::grammar::Options;

#[derive(Debug, Default)]
pub struct ArtifactOptions {
    /// `None` means the user did not say, so the detected default applies.
    pub formats: Option<FormatOrder>,
    pub asset: Option<AssetPattern>,
    pub bin: Option<String>,
    /// `@download_only` (D3b): fetch the artifact and stop — never unpack it, shim it, or put it
    /// on `PATH`. It is still declared, so it is still removed when the line goes. This is what a
    /// download backend does *by default* when it has no way to install the thing it fetched (no
    /// executable inside, no installer), rather than failing.
    pub download_only: bool,
}

impl ArtifactOptions {
    /// **The list arrives as a list.** `formats` used to be one string that this function split
    /// on a `;` `to_spec` had joined it with — a delimiter nothing validated, standing in for a
    /// `Vec` the grammar had produced and then thrown away. A `@formats=` value containing a
    /// semicolon was unrepresentable and no layer said so.
    pub fn read(options: &Options) -> Result<Self, String> {
        let raw = options.all("formats");
        let formats = if raw.is_empty() {
            None
        } else {
            Some(
                FormatOrder::parse_all(
                    raw.iter()
                        .map(String::as_str)
                        .filter(|s| !s.trim().is_empty()),
                )
                .map_err(|e| e.to_string())?
                // A `@formats=` the user wrote is an instruction, and the tie-break honours
                // it over an asset that merely names the machine well (D2).
                .as_user_specified(),
            )
        };

        let asset = match options.one("asset") {
            None => None,
            Some(raw) => Some(AssetPattern::parse(raw).map_err(|e| e.to_string())?),
        };

        let bin = options
            .one("bin")
            .map(|b| b.trim().to_string())
            .filter(|b| !b.is_empty());

        let download_only = options
            .one("download_only")
            .map(|v| v != "false" && v != "no")
            .unwrap_or(false);

        Ok(ArtifactOptions {
            formats,
            asset,
            bin,
            download_only,
        })
    }

    /// The line beats `priority`, and `priority` beats the built-in default. A line's list
    /// replaces rather than extends: half-overriding an ordered list produces an order nobody
    /// wrote.
    pub fn resolved_formats(&self, fallback: &FormatOrder) -> FormatOrder {
        match &self.formats {
            Some(f) if !f.is_empty() => f.clone(),
            _ => fallback.clone(),
        }
    }
}

/// The built-in default for this machine, from the same detected facts `when family` reads.
pub fn default_formats() -> FormatOrder {
    let facts = crate::config::parser::HostFacts::current();
    FormatOrder::detected_default(&facts.os, Some(facts.family.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::artifact::Format;

    /// A `;` in a value here is not a delimiter any more — it is written as the list it was
    /// standing in for, which is the whole point of the change these tests cover.
    fn opts(pairs: &[(&str, &str)]) -> Options {
        let mut o = Options::default();
        for (k, v) in pairs {
            for part in v.split(';') {
                o.insert(*k, part);
            }
        }
        o
    }

    #[test]
    fn a_repeated_key_keeps_the_order_it_was_written_in() {
        let read = ArtifactOptions::read(&opts(&[("formats", "deb;tarball;binary")])).unwrap();
        assert_eq!(
            read.formats.unwrap().as_slice(),
            &[Format::Deb, Format::Tarball, Format::Binary]
        );
    }

    #[test]
    fn a_single_format_needs_no_separator() {
        let read = ArtifactOptions::read(&opts(&[("formats", "deb")])).unwrap();
        assert_eq!(read.formats.unwrap().as_slice(), &[Format::Deb]);
    }

    #[test]
    fn an_unknown_format_is_refused_here_too() {
        let err = ArtifactOptions::read(&opts(&[("formats", "deb;nonsense")])).unwrap_err();
        assert!(err.contains("nonsense"));
    }

    #[test]
    fn no_formats_option_means_the_default_applies() {
        let read = ArtifactOptions::read(&opts(&[])).unwrap();
        let fallback = FormatOrder::new(vec![Format::Tarball]);
        assert_eq!(read.resolved_formats(&fallback), fallback);
    }

    #[test]
    fn a_line_replaces_the_fallback_rather_than_extending_it() {
        let read = ArtifactOptions::read(&opts(&[("formats", "rpm")])).unwrap();
        let fallback = FormatOrder::new(vec![Format::Deb, Format::Tarball]);
        assert_eq!(read.resolved_formats(&fallback).as_slice(), &[Format::Rpm]);
    }

    #[test]
    fn an_empty_bin_is_the_same_as_no_bin() {
        let read = ArtifactOptions::read(&opts(&[("bin", "   ")])).unwrap();
        assert!(read.bin.is_none());
    }

    #[test]
    fn an_asset_pattern_is_compiled_once_here() {
        let read = ArtifactOptions::read(&opts(&[("asset", "*musl*")])).unwrap();
        assert!(read.asset.unwrap().matches("tool-musl.deb"));
    }

    #[test]
    fn download_only_defaults_off_and_reads_a_bare_flag() {
        assert!(!ArtifactOptions::read(&opts(&[])).unwrap().download_only);
        // The grammar turns a bare `@download_only` into "true".
        assert!(
            ArtifactOptions::read(&opts(&[("download_only", "true")]))
                .unwrap()
                .download_only
        );
        assert!(
            !ArtifactOptions::read(&opts(&[("download_only", "false")]))
                .unwrap()
                .download_only
        );
        assert!(
            !ArtifactOptions::read(&opts(&[("download_only", "no")]))
                .unwrap()
                .download_only
        );
    }
}
