//! Reading the artifact options off a resolved `PackageSpec`.
//!
//! One place turns option strings into the selector's types, so a backend never parses
//! `formats` itself and the grammar's validation and the backend's reading cannot disagree.

use super::format::FormatOrder;
use super::pattern::AssetPattern;
use std::collections::HashMap;

/// `to_spec` flattens a repeated key by joining the values with `;`, which is what preserves
/// `formats`' order across the seam.
const LIST_SEPARATOR: char = ';';

#[derive(Debug, Default)]
pub struct ArtifactOptions {
    /// `None` means the user did not say, so the detected default applies.
    pub formats: Option<FormatOrder>,
    pub asset: Option<AssetPattern>,
    pub bin: Option<String>,
}

impl ArtifactOptions {
    pub fn read(options: &HashMap<String, String>) -> Result<Self, String> {
        let formats = match options.get("formats") {
            None => None,
            Some(raw) => Some(
                FormatOrder::parse_all(raw.split(LIST_SEPARATOR).filter(|s| !s.trim().is_empty()))
                    .map_err(|e| e.to_string())?,
            ),
        };

        let asset = match options.get("asset") {
            None => None,
            Some(raw) => Some(AssetPattern::parse(raw).map_err(|e| e.to_string())?),
        };

        let bin = options
            .get("bin")
            .map(|b| b.trim().to_string())
            .filter(|b| !b.is_empty());

        Ok(ArtifactOptions {
            formats,
            asset,
            bin,
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

/// The built-in default for this machine.
pub fn default_formats() -> FormatOrder {
    FormatOrder::detected_default(
        std::env::consts::OS,
        crate::config::parser::distro_family().as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::artifact::Format;

    fn opts(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn a_joined_list_keeps_the_order_it_was_written_in() {
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
}
