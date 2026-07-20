use super::error::{GrammarError, Origin, Result};
use std::collections::BTreeMap;

/// Option values, keyed in a stable order so error messages and rendered lines do not
/// reshuffle between runs.
///
/// A key may hold more than one value: II.2 makes a key given twice a list (`requires`
/// twice means two requirements), so the storage is a list everywhere and `one()` is the
/// accessor that rejects the plural case.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    inner: BTreeMap<String, Vec<String>>,
}

impl Options {
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn contains(&self, key: &str) -> bool {
        self.inner.contains_key(key)
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.inner.keys().map(String::as_str)
    }

    pub fn all(&self, key: &str) -> &[String] {
        self.inner.get(key).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The single value for `key`, or `None`. A key holding a list yields its first value;
    /// callers that care about the difference use `all`.
    pub fn one(&self, key: &str) -> Option<&str> {
        self.inner.get(key).and_then(|v| v.first()).map(String::as_str)
    }

    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.inner.entry(key.into()).or_default().push(value.into());
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &[String])> {
        self.inner.iter().map(|(k, v)| (k.as_str(), v.as_slice()))
    }

    /// Every value, mutably — for rewriting values in place (variable expansion, IX). Keys
    /// are not offered: a key is grammar, and rewriting one would change what a line means.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut String> {
        self.inner.values_mut().flat_map(|v| v.iter_mut())
    }
}

/// Parse the short form: `@key=value,key2=value2`, given the text AFTER the `@`.
///
/// A bare key is `true` (`@hold`). A comma is always a separator, never data — II.2 makes
/// a comma inside a value an error rather than a guess, because the alternative is
/// deciding on the user's behalf whether `>=1.0,<2.0` is one value or two, and both
/// readings are plausible.
pub fn parse_short(origin: &Origin, text: &str) -> Result<Options> {
    let mut out = Options::default();
    if text.trim().is_empty() {
        return Err(GrammarError::new(origin.clone(), "`@` with no options after it")
            .with_hint("write `@key=value`, or drop the `@`."));
    }

    for part in text.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err(
                GrammarError::new(origin.clone(), "empty option between commas")
                    .with_hint("write `@key=value,key2=value2` with no trailing or doubled comma."),
            );
        }
        match part.split_once('=') {
            Some((k, v)) => {
                let (k, v) = (k.trim(), v.trim());
                if k.is_empty() {
                    return Err(GrammarError::new(
                        origin.clone(),
                        format!("option `{}` has no key before the `=`", part),
                    ));
                }
                if !is_key(k) {
                    return Err(comma_in_value(origin, text));
                }
                out.insert(k, v);
            }
            // No `=`. A bare flag (`@hold`), the `@2.0` mistake, or the tail of a value
            // that contained a comma.
            None => {
                if looks_like_a_version(part) {
                    return Err(GrammarError::new(
                        origin.clone(),
                        format!("`@{}` is not an option", part),
                    )
                    .with_hint(format!("did you mean `@version={}`?", part)));
                }
                if !is_key(part) {
                    return Err(comma_in_value(origin, text));
                }
                out.insert(part, "true");
            }
        }
    }
    Ok(out)
}

/// II.2's specific refusal. Reached when a comma-separated part is not `key=value` and not
/// a flag name, which is what a value containing a comma looks like from here:
/// `version=>=1.0,<2.0` splits into `version=>=1.0` and `<2.0`, and `<2.0` is no key.
fn comma_in_value(origin: &Origin, text: &str) -> GrammarError {
    GrammarError::new(
        origin.clone(),
        format!("`@{}` is not a list of `key=value` options", text),
    )
    .with_hint("commas need the block form.")
}

/// Whether a bare short-form token reads as a version rather than a flag name.
///
/// The test is "starts with a digit", not "parses as a version": `@2` and `@1.6-rc1` are
/// both the same mistake, and no option key begins with a digit.
fn looks_like_a_version(token: &str) -> bool {
    token.chars().next().is_some_and(|c| c.is_ascii_digit())
}

/// An option key: a plain identifier. Not a judgement about which keys exist (II.2's table
/// decides that) — only about what could possibly be one, which is what tells a key apart
/// from the wreckage of a comma-split value.
fn is_key(token: &str) -> bool {
    !token.is_empty()
        && token.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Parse one `key = value` line from inside a `{ }` block.
///
/// Everything after the first `=` is the value, verbatim and trimmed. No escaping exists
/// and none is needed: you reached for the block form precisely because you needed a value
/// the short form could not hold (V.9).
pub fn parse_block_line(origin: &Origin, line: &str) -> Result<(String, String)> {
    let Some((key, value)) = line.split_once('=') else {
        return Err(GrammarError::new(
            origin.clone(),
            format!("`{}` is not `key = value`", line.trim()),
        )
        .with_hint("every line inside a `{ }` block is `key = value`."));
    };
    let key = key.trim();
    let value = value.trim();
    if key.is_empty() {
        return Err(GrammarError::new(
            origin.clone(),
            format!("`{}` has no key before the `=`", line.trim()),
        ));
    }
    if key.split_whitespace().count() > 1 {
        return Err(GrammarError::new(
            origin.clone(),
            format!("`{}` is not a single option key", key),
        )
        .with_hint("keys are one word: `after_install = ./setup.sh`."));
    }
    // `#` does not start a comment inside a block value (V.9): truncating
    // `curl -H "X: #tag"` would silently run the wrong command. But someone who meant a
    // comment gets told, rather than left wondering why their value has a `#` in it.
    if value.contains(" # ") {
        return Err(GrammarError::new(
            origin.clone(),
            format!("block value for `{}` contains ` # `", key),
        )
        .with_hint(
            "block values are verbatim — did you mean a comment? Put it on its own line.",
        ));
    }
    Ok((key.to_string(), value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o() -> Origin {
        Origin::new("modules/dev.txt", 3)
    }

    #[test]
    fn short_form_reads_key_value_pairs() {
        let opts = parse_short(&o(), "version=1.6").unwrap();
        assert_eq!(opts.one("version"), Some("1.6"));
    }

    #[test]
    fn short_form_reads_several_pairs() {
        let opts = parse_short(&o(), "version=1.6,hold").unwrap();
        assert_eq!(opts.one("version"), Some("1.6"));
        assert_eq!(opts.one("hold"), Some("true"));
    }

    #[test]
    fn a_bare_key_is_true() {
        let opts = parse_short(&o(), "hold").unwrap();
        assert_eq!(opts.one("hold"), Some("true"));
    }

    #[test]
    fn a_comma_in_a_value_is_an_error_not_a_guess() {
        // II.2. `>=1.0,<2.0` could be one value or two and both readings are plausible,
        // so the parser refuses rather than picking one.
        let err = parse_short(&o(), "version=>=1.0,<2.0").unwrap_err();
        let hint = err.hint.unwrap();
        assert!(hint.contains("block form"), "{}", hint);
    }

    #[test]
    fn a_bare_version_says_what_was_meant() {
        let err = parse_short(&o(), "2.0").unwrap_err();
        assert_eq!(err.hint.as_deref(), Some("did you mean `@version=2.0`?"));
    }

    #[test]
    fn an_empty_option_list_is_an_error() {
        assert!(parse_short(&o(), "").is_err());
        assert!(parse_short(&o(), "   ").is_err());
    }

    #[test]
    fn a_doubled_comma_is_an_error() {
        assert!(parse_short(&o(), "hold,,version=1").is_err());
        assert!(parse_short(&o(), "hold,").is_err());
    }

    #[test]
    fn every_error_names_the_file_and_line() {
        let err = parse_short(&o(), "2.0").unwrap_err();
        assert_eq!(err.origin.file.to_string_lossy(), "modules/dev.txt");
        assert_eq!(err.origin.line, 3);
        assert!(err.to_string().contains("modules/dev.txt:3"));
    }

    #[test]
    fn a_block_value_runs_verbatim_to_end_of_line() {
        let (k, v) = parse_block_line(&o(), "  after_install = ./setup.sh --flag=a,b  ").unwrap();
        assert_eq!(k, "after_install");
        // Commas and `=` are data here — that is the whole point of the block form.
        assert_eq!(v, "./setup.sh --flag=a,b");
    }

    #[test]
    fn a_block_value_keeps_a_hash_that_is_part_of_the_value() {
        let (_, v) = parse_block_line(&o(), r#"after_install = curl -H "X:#tag""#).unwrap();
        assert_eq!(v, r#"curl -H "X:#tag""#);
    }

    #[test]
    fn a_block_value_that_looks_like_it_has_a_comment_says_so() {
        // V.9: the value is verbatim, so ` # ` is data. Someone who meant a comment would
        // otherwise get a silently wrong value.
        let err = parse_block_line(&o(), "version = 1.6 # my pin").unwrap_err();
        let hint = err.hint.unwrap();
        assert!(hint.contains("verbatim"), "{}", hint);
        assert!(hint.contains("own line"), "{}", hint);
    }

    #[test]
    fn a_block_line_without_an_equals_is_an_error() {
        assert!(parse_block_line(&o(), "after_install ./setup.sh").is_err());
    }

    #[test]
    fn a_repeated_key_makes_a_list() {
        let mut opts = Options::default();
        opts.insert("requires", "apt:libfoo");
        opts.insert("requires", "apt:libbar");
        assert_eq!(opts.all("requires"), ["apt:libfoo", "apt:libbar"]);
    }
}
