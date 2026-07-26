//! Files that are a list of bare names plus `when` blocks — `active` and `priority` (II.6).
//!
//! Both answer one question with one word per line, and both gate those lines with `when`.
//! That was two hand-rolled block walkers with the same brace handling, the same nesting
//! limit and the same unclosed/stray-brace errors, written twice — so "one rule, everywhere"
//! (II.2) was one rule and several implementations, which is how they drift.
//!
//! Modules and profiles are NOT read here, and that is deliberate rather than an omission:
//! they hold statements, not names, and a `when` may nest inside them. A list you open to
//! see what is on at a glance nests one level and no further.

use super::error::{GrammarError, Origin, Result};
use super::options::{self, Options};
use crate::config::parser::{eval_when, HostFacts};
use std::path::Path;

/// One name in such a file, and the `when` block that gates it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatedLine {
    pub text: String,
    /// 1-based, as an editor counts.
    pub line: usize,
    /// The predicate of the enclosing block, if any.
    pub gate: Option<String>,
    /// Whether it applies to this host. A name inside a `when` that does not match is in
    /// the file and not in force.
    pub on: bool,
    /// The name's options body, empty unless the file allows one and the name carried one.
    pub options: Options,
}

/// What the file holds, so one reader can write both files' errors in their own words.
pub struct Vocabulary<'a> {
    /// What one line names: "profile name", "backend name".
    pub noun: &'a str,
    /// The hint for a line that is neither a name nor a block.
    pub holds: &'a str,
    /// The hint for a nested `when`.
    pub nesting: &'a str,
    /// `Some(hint)` if a name here may carry a `{ key = value }` body, `None` if it may not.
    ///
    /// `priority` may — a backend line takes `formats`/`channel` (VIII.2). `active` may not:
    /// a profile name answers one question and has nothing to configure, so a body there is
    /// a mistake and reads better as one.
    pub body: Option<&'a str>,
}

/// Read the body into its names, keeping every one — gated or not — so the caller can tell
/// "not listed" from "listed but not for this host". The caller applies whatever rule makes
/// a name legal in its file; this owns the block structure and nothing else.
pub fn read(
    file: &Path,
    body: &str,
    facts: &HostFacts,
    vocab: &Vocabulary,
) -> Result<Vec<GatedLine>> {
    read_inner(file, body, Some(facts), vocab)
}

/// Every name the file holds, `when` blocks included whether or not they match, with no
/// predicate evaluated at all.
///
/// For the one question that has to be answered before variables exist: `priority` decides
/// which backends there are, resolving variables needs that vocabulary, and evaluating a
/// `when $role` here is the unknown-key refusal this exists to remove. A superset is safe for
/// that use and wrong for every other, so this must never decide what to install — the same
/// warning [`Document::every_statement`] carries.
pub fn read_every(file: &Path, body: &str, vocab: &Vocabulary) -> Result<Vec<GatedLine>> {
    read_inner(file, body, None, vocab)
}

fn read_inner(
    file: &Path,
    body: &str,
    facts: Option<&HostFacts>,
    vocab: &Vocabulary,
) -> Result<Vec<GatedLine>> {
    let mut out: Vec<GatedLine> = Vec::new();
    let mut gate: Option<(String, bool)> = None;
    // The entry whose `{ }` body is open, and the header it was opened with.
    let mut open_body: Option<(usize, String)> = None;

    for (idx, raw) in body.lines().enumerate() {
        let origin = Origin::new(file, idx + 1);

        // Inside an options body a value is verbatim to end of line, so `#` is data and
        // must not be stripped (V.9). A whole-line comment is still a comment.
        if let Some((entry, header)) = open_body.clone() {
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if trimmed == "}" {
                open_body = None;
                continue;
            }
            if trimmed.ends_with('{') {
                return Err(GrammarError::new(
                    origin,
                    format!("a `{{` block inside the `{}` block", header),
                )
                .with_hint(vocab.nesting.to_string()));
            }
            let (k, v) = options::parse_block_line(&origin, trimmed)?;
            out[entry].options.insert(k, v);
            continue;
        }

        let line = super::strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }

        if line == "}" {
            if gate.is_none() {
                return Err(GrammarError::new(
                    origin,
                    "`}` closes a `when` that was never opened",
                ));
            }
            gate = None;
            continue;
        }

        // A name carrying an options body, where the file allows one.
        if let (Some(hint), Some(header)) = (vocab.body, line.strip_suffix('{')) {
            let name = header.trim();
            if !name.is_empty() && !name.starts_with("when ") {
                if name.split_whitespace().count() > 1 {
                    return Err(GrammarError::new(
                        origin,
                        format!("`{}` is not a {}", name, vocab.noun),
                    )
                    .with_hint(hint.to_string()));
                }
                out.push(GatedLine {
                    text: name.to_string(),
                    line: idx + 1,
                    gate: gate.as_ref().map(|(p, _)| p.clone()),
                    on: gate.as_ref().map(|(_, hit)| *hit).unwrap_or(true),
                    options: Options::default(),
                });
                open_body = Some((out.len() - 1, name.to_string()));
                continue;
            }
        }

        if let Some(header) = line.strip_suffix('{') {
            let header = header.trim();
            let Some(pred) = header.strip_prefix("when ") else {
                return Err(GrammarError::new(
                    origin,
                    format!("`{}` is not a `when` block", header),
                )
                .with_hint(vocab.holds.to_string()));
            };
            if gate.is_some() {
                return Err(
                    GrammarError::new(origin, "a `when` block inside a `when` block")
                        .with_hint(vocab.nesting.to_string()),
                );
            }
            let pred = pred.trim();
            let hit = match facts {
                Some(facts) => eval_when(pred, facts)
                    .map_err(|e| GrammarError::new(Origin::new(file, idx + 1), e.to_string()))?,
                None => true,
            };
            gate = Some((pred.to_string(), hit));
            continue;
        }

        if line.split_whitespace().count() > 1 {
            return Err(
                GrammarError::new(origin, format!("`{}` is not a {}", line, vocab.noun))
                    .with_hint(vocab.holds.to_string()),
            );
        }

        out.push(GatedLine {
            text: line.to_string(),
            line: idx + 1,
            gate: gate.as_ref().map(|(p, _)| p.clone()),
            on: gate.as_ref().map(|(_, hit)| *hit).unwrap_or(true),
            options: Options::default(),
        });
    }

    if let Some((entry, header)) = open_body {
        return Err(GrammarError::new(
            Origin::new(file, out[entry].line),
            format!("the `{}` block is never closed", header),
        )
        .with_hint("add the matching `}`."));
    }
    if gate.is_some() {
        return Err(
            GrammarError::new(Origin::new(file, 0), "a `when` block is never closed")
                .with_hint("add the matching `}`."),
        );
    }
    Ok(out)
}
