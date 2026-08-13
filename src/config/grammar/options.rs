use super::error::{GrammarError, Origin, Result};
use std::collections::BTreeMap;

/// Option values, keyed in a stable order so error messages and rendered lines do not
/// reshuffle between runs.
///
/// A key may hold more than one value: II.2 makes a key given twice a list (`requires`
/// twice means two requirements), so the storage is a list everywhere and `one()` is the
/// accessor that rejects the plural case.
///
/// **Serialised as the map of lists it is.** `PackageSpec` carried
/// `HashMap<String, String>` and `to_spec` joined every list with `;` to fit — so a saved plan
/// wrote `"requires": "a;b"` and something downstream had to split it back on a delimiter
/// nothing validated. A plan file written by an older build does not load, which is correct: it
/// was written in a format that could not represent what the grammar accepts.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
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
        self.inner
            .get(key)
            .and_then(|v| v.first())
            .map(String::as_str)
    }

    /// Append a value to `key`. **A key given twice is a list** (II.2), so this pushes.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.inner.entry(key.into()).or_default().push(value.into());
    }

    /// Make `key` hold exactly `value`, discarding whatever it held.
    ///
    /// Separate from [`insert`](Self::insert) because the two are different operations and the
    /// old `HashMap` spelled both as `insert`. A caller stamping a resolved version onto a spec
    /// means *replace*; a caller reading a second `@requires=` from the line means *append*.
    /// One name for both is how a re-resolved package would have ended up with two versions.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.inner.insert(key.into(), vec![value.into()]);
    }

    /// Make `key` hold exactly these values, in order.
    ///
    /// The plural of [`set`](Self::set), and the reason `to_spec` no longer joins with `;`:
    /// copying a list from one `Options` into another is the operation, and it should not have
    /// to go through a delimiter to do it.
    pub fn set_all(&mut self, key: impl Into<String>, values: Vec<String>) {
        self.inner.insert(key.into(), values);
    }

    /// Drop `key` entirely. `None` when it was not there.
    pub fn remove(&mut self, key: &str) -> Option<Vec<String>> {
        self.inner.remove(key)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
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

/// Built from pairs, appending — so the same key twice is the list II.2 says it is, and a
/// fixture written as a sequence of pairs means what it reads as.
impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for Options {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut out = Self::default();
        for (k, v) in iter {
            out.insert(k, v);
        }
        out
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
        return Err(
            GrammarError::new(origin.clone(), "`@` with no options after it")
                .with_hint("write `@key=value`, or drop the `@`."),
        );
    }

    for part in text.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err(
                GrammarError::new(origin.clone(), "empty option between commas")
                    .with_hint("write `@key=value,key2=value2` with no trailing or doubled comma."),
            );
        }
        if let Some((tail, how)) = spliced_option(part) {
            let hint = match how {
                Splice::AfterSpace => format!(
                    "options are separated by commas, never by spaces. Written with a space, `{}` \
                     is absorbed into the option before it rather than being an option at all.",
                    tail
                ),
                Splice::NamesAnOption => format!(
                    "options are separated by commas: write `,{}`. Written straight after a \
                     value, `{}` is absorbed into that value rather than being an option at all. \
                     If you meant it as part of the value, use the block form.",
                    tail.trim_start_matches('@'),
                    tail
                ),
            };
            return Err(GrammarError::new(
                origin.clone(),
                format!(
                    "`{}` runs two options together — `{}` is not part of a value",
                    part, tail
                ),
            )
            .with_hint(hint));
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
                if v.is_empty() {
                    return Err(empty_value(origin, k));
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

/// `@version=` with nothing after it (AU10).
///
/// A key written with an `=` and no value is a half-typed line, not a request for an empty
/// string: `cargo:ripgrep@version=` parsed, yielded `"version": ""`, and exited 0 — while
/// `npm:` with an empty NAME was correctly refused three lines away. The same shape, refused on
/// one side of the `@` and accepted on the other.
///
/// A flag is how you say "no value": `@hold`, not `@hold=`.
fn empty_value(origin: &Origin, key: &str) -> GrammarError {
    GrammarError::new(
        origin.clone(),
        format!("`@{}=` has nothing after the `=`", key),
    )
    .with_hint(format!(
        "write a value (`@{}=1.2.3`), or drop the `=` if you meant the flag `@{}`.",
        key, key
    ))
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
/// The tail of an option token that has a second option spliced into it with a space.
///
/// **This lexer splits on the first `@` and separates options on commas, so anything written
/// after a space was absorbed into whatever option came before it.** `cargo:ripgrep@nosuchkey`
/// is refused and `cargo:ripgrep@sha256=abc @nosuchkey` was accepted in silence — the same
/// text, admitted or refused by its position alone. What it produced was
/// `sha256 = "abc @nosuchkey"`: a checksum that cannot match, and a second option that never
/// existed. `pkg@requires=jq @hold` made the hold inert the same way (B2).
///
/// **It belongs here rather than in each grammar's validator, because the swallow happens in
/// all ten of them.** Seven accepted it outright; the three that refused — `absent:`, `exec:`,
/// `firewall:` — were saved only by a downstream type check on a date, a count and an enum, and
/// each quoted the swallowed text back as part of the value while doing it. That protection is
/// incidental rather than structural: adding one free-form option to any of the three reopens
/// it tomorrow.
///
/// **An `@` on its own is not the signal.** `@requires=@angular/cli` and
/// `@source=github:owner/repo@v2` are ordinary values and stay legal.
///
/// **Two signals, because the space was never the mechanism.** Requiring whitespace before the
/// `@` made the refusal a property of how the line was typed rather than of what it says:
/// `cargo:ripgrep@version=1.0.0 @hold` was refused and `cargo:ripgrep@version=1.0.0@hold` parsed
/// as the single version `1.0.0@hold`, silently dropping the hold. All ten bare flags went the
/// same way, `@sandbox` and `@system` among them — an option that decides whether a command is
/// confined, and one that decides whether a package is written into the OS's environment.
///
/// So the second signal is the text itself: an `@` whose tail *names an option*. That is the
/// only thing separating `1.0.0@hold` from `owner/repo@v2`, and it is why this function has to
/// consult II.2's table rather than judge by shape alone. A value that genuinely needs an `@`
/// followed by an option's name has the block form, which exists for exactly the values the
/// short form cannot hold.
fn spliced_option(part: &str) -> Option<(&str, Splice)> {
    let bytes = part.as_bytes();
    if let Some(at) =
        (1..bytes.len()).find(|&i| bytes[i] == b'@' && bytes[i - 1].is_ascii_whitespace())
    {
        return Some((part[at..].trim(), Splice::AfterSpace));
    }
    // Every `@`, not just the first: `@version=1.0@hold` and `@version=1.0@x@hold` are one
    // mistake, and stopping at the first `@` would find `x` and call it a value.
    for i in (1..bytes.len()).filter(|&i| bytes[i] == b'@') {
        let tail = &part[i + 1..];
        let key = tail.split_once('=').map_or(tail, |(k, _)| k);
        if crate::config::grammar::statement::is_package_option_key(key) {
            return Some((part[i..].trim(), Splice::NamesAnOption));
        }
    }

    // **And a third signal, for the flag names that are not options at all.**
    // `@version=1.0.0@optional` splices a word II.2's table has never held, so the rule above
    // cannot see it — and it is absorbed exactly like `@hold` was, into a version of
    // `1.0.0@optional` that no manager will ever match.
    //
    // **Two conditions, and both are load-bearing.** The key must be one whose value is a
    // number, a date or a digest rather than a name — `source=` and `requires=` are excluded,
    // because `github:owner/repo@v2` and `@angular/cli` are ordinary values and Q23's ruling
    // depends on a *name* being able to carry an `@`. And the tail must read as an option name
    // rather than as more of the value: `version=1.2.3+build@7` is legal semver build metadata,
    // and `7` is not a key. That is the discriminator [`looks_like_a_version`] already encodes —
    // a leading digit means a version, never a flag — applied on the other side of the `@`.
    if let Some((key, value)) = part.split_once('=') {
        const VALUE_IS_NOT_A_NAME: &[&str] =
            &["version", "sha256", "expires", "until", "size", "quota"];
        if VALUE_IS_NOT_A_NAME.contains(&key.trim()) {
            for (i, _) in value.match_indices('@') {
                let tail = &value[i + 1..];
                let candidate = tail.split_once('=').map_or(tail, |(k, _)| k);
                if is_key(candidate) && !looks_like_a_version(candidate) {
                    let at = key.len() + 1 + i;
                    return Some((part[at..].trim(), Splice::NamesAnOption));
                }
            }
        }
    }
    None
}

/// Which of the two signals found a spliced option — they need different advice, because one is
/// a typing mistake and the other is a value that has to move to the block form.
enum Splice {
    AfterSpace,
    NamesAnOption,
}

fn is_key(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
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
        .with_hint("block values are verbatim — did you mean a comment? Put it on its own line."));
    }
    // The block form's half of AU10. The short form is where the typo was measured, but this
    // is the same line with different punctuation, and a refusal that covers one spelling of a
    // mistake teaches the other one.
    if value.is_empty() {
        return Err(empty_value(origin, key));
    }
    Ok((key.to_string(), value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o() -> Origin {
        Origin::new("modules/dev.txt", 3)
    }

    /// AU10: `@version=` parsed, produced `"version": ""`, and exited 0 — while `npm:` with an
    /// empty name was refused. Both spellings of the mistake, and the flag form that still has
    /// to work, because a refusal that also catches `@hold` would have broken every pin.
    #[test]
    fn an_option_with_nothing_after_the_equals_is_refused() {
        for line in ["version=", "version=  ", "hold=", "arch=x86,extra="] {
            let Err(err) = parse_short(&o(), line) else {
                panic!("`@{}` was accepted", line);
            };
            assert!(
                err.what.contains("nothing after the `=`"),
                "`{}` was refused for the wrong reason: {}",
                line,
                err.what
            );
        }

        let err = parse_block_line(&o(), "after_install =").unwrap_err();
        assert!(err.what.contains("nothing after the `=`"), "{:?}", err);

        // Controls: the flag form and a real value are untouched.
        assert_eq!(parse_short(&o(), "hold").unwrap().one("hold"), Some("true"));
        assert_eq!(
            parse_short(&o(), "version=1.6").unwrap().one("version"),
            Some("1.6")
        );
    }

    /// **B2, in the lexer where it lives.** Options are separated by commas; a space put the
    /// second one inside the first one's value, and nothing said so.
    #[test]
    fn a_second_option_written_with_a_space_is_refused_rather_than_swallowed() {
        for line in [
            "sha256=abc @nosuchkey",
            "version=1.2.3 @sha256=deadbeef",
            "requires=jq @hold",
            "hold @nosuchkey",
            // A tab is a space for this purpose; nobody meant it either.
            "version=1.6\t@hold",
        ] {
            let err = parse_short(&o(), line)
                .expect_err("a space before `@` runs two options together and must be refused");
            let msg = format!("{err}");
            assert!(
                msg.contains("comma") || msg.contains("two options"),
                "the refusal must name the separator, or the user cannot act on it: {msg}"
            );
        }

        // And the specific damage it used to do, asserted as no longer possible: the value was
        // silently wrong and the second option silently absent.
        assert!(parse_short(&o(), "version=1.2.3 @sha256=deadbeef").is_err());
    }

    /// The boundary. An `@` is ordinary inside a value and stays legal — banning it outright
    /// would refuse a scoped npm package and a pinned module source, which are the two things
    /// most likely to be written as an option value.
    #[test]
    fn an_at_sign_inside_a_value_is_not_two_options() {
        for (line, key, value) in [
            ("requires=@angular/cli", "requires", "@angular/cli"),
            (
                "source=github:owner/repo@v2",
                "source",
                "github:owner/repo@v2",
            ),
            ("version=1.2.3+build@7", "version", "1.2.3+build@7"),
        ] {
            let opts = parse_short(&o(), line)
                .unwrap_or_else(|e| panic!("`{line}` is an ordinary value and was refused: {e}"));
            assert_eq!(opts.one(key), Some(value));
        }

        // A value may still carry a space when no `@` follows it — the rule is about the
        // separator that was mistyped, not about spaces.
        assert_eq!(
            parse_short(&o(), "content=hello world")
                .unwrap()
                .one("content"),
            Some("hello world")
        );
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
        opts.insert("requires".to_string(), "apt:libfoo");
        opts.insert("requires".to_string(), "apt:libbar");
        assert_eq!(opts.all("requires"), ["apt:libfoo", "apt:libbar"]);
    }
}
