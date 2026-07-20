//! Resolving the `vars` file into `name → value` pairs (Part IX).
//!
//! One contract: a provider produces `name → value`. This is the file provider — the same
//! contract with a trivial implementation — and the resolution rules below are the contract's,
//! not the file's, so a script or an external executable resolves through the same code.
//!
//! Pure: no I/O, no clock, no shell. The caller hands over definitions that `when` has already
//! gated and gets back the resolved set or an error naming the file and line.

use crate::config::grammar::{GrammarError, Origin, Result};
use std::collections::{BTreeMap, HashSet};

/// The name [`expand`] resolves a standalone value under. Never a real variable name — a
/// variable is an identifier, and this is not one — so it cannot collide with a user's.
const VALUE_PLACEHOLDER: &str = "<value>";

/// One `NAME = VALUE` line. `conditional` is whether it came from inside a `when` block, which
/// is what IX.3 turns on: a top-level line defines a name, a conditional one may only override.
#[derive(Debug, Clone)]
pub struct Definition {
    pub name: String,
    pub value: String,
    pub origin: Origin,
    pub conditional: bool,
}

/// The resolved set. A `BTreeMap` so `plan` and `sync` print variables in the same order and a
/// diff of two machines' resolved vars is readable.
pub type Vars = BTreeMap<String, String>;

/// Resolve definitions that `when` has already gated down to the ones that apply here.
///
/// Order of business, and each step is a rule from IX.3:
/// 1. Every name needs a top-level definition; a `when` block may not introduce one.
/// 2. Two matching `when` blocks setting one name differently is a contradiction, not a
///    last-wins — the same rule II.7.5 applies to package declarations.
/// 3. Values may reference other variables, so they resolve in dependency order, and a cycle
///    is an error naming the loop.
pub fn resolve(defs: &[Definition]) -> Result<Vars> {
    let mut defaults: BTreeMap<String, &Definition> = BTreeMap::new();
    for def in defs.iter().filter(|d| !d.conditional) {
        if let Some(prev) = defaults.insert(def.name.clone(), def) {
            return Err(GrammarError::new(
                def.origin.clone(),
                format!(
                    "`{}` is defined twice at the top level (also at {})",
                    def.name, prev.origin
                ),
            )
            .with_hint("a name has one default; use a `when` block to override it."));
        }
    }

    // The overrides that actually apply. Two blocks that both matched and disagree is the
    // contradiction; two that agree is redundant but not wrong, so it is not an error.
    let mut applied: BTreeMap<String, &Definition> = BTreeMap::new();
    for def in defs.iter().filter(|d| d.conditional) {
        if !defaults.contains_key(&def.name) {
            return Err(GrammarError::new(
                def.origin.clone(),
                format!("`{}` is only defined inside a `when` block", def.name),
            )
            .with_hint(
                "give it a default at the top level. Every variable is defined on every \
                 machine, so a typo is always an error and never a block that quietly never \
                 fires.",
            ));
        }
        match applied.get(&def.name) {
            Some(prev) if prev.value != def.value => {
                return Err(GrammarError::new(
                    def.origin.clone(),
                    format!(
                        "`{}` is set to `{}` here and `{}` at {} — both conditions match this machine",
                        def.name, def.value, prev.value, prev.origin
                    ),
                )
                .with_hint("narrow one of the `when` conditions so only one applies."));
            }
            _ => {
                applied.insert(def.name.clone(), def);
            }
        }
    }

    let mut raw: BTreeMap<String, &Definition> = defaults;
    raw.extend(applied);

    interpolate_all(&raw)
}

/// Resolve every value, substituting `$other` references, in dependency order.
fn interpolate_all(raw: &BTreeMap<String, &Definition>) -> Result<Vars> {
    let mut done: Vars = BTreeMap::new();
    let mut visiting: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let names: Vec<String> = raw.keys().cloned().collect();
    for name in names {
        resolve_one(&name, raw, &mut done, &mut visiting, &mut seen)?;
    }
    Ok(done)
}

fn resolve_one(
    name: &str,
    raw: &BTreeMap<String, &Definition>,
    done: &mut Vars,
    visiting: &mut Vec<String>,
    seen: &mut HashSet<String>,
) -> Result<()> {
    if done.contains_key(name) {
        return Ok(());
    }
    let def = match raw.get(name) {
        Some(d) => *d,
        None => return Ok(()),
    };

    if seen.contains(name) {
        // Name the whole loop rather than the one edge that closed it: "a -> b -> a" is
        // actionable, "a cycle exists" is not (V.45).
        let start = visiting.iter().position(|v| v == name).unwrap_or(0);
        let mut loop_names: Vec<String> = visiting[start..].to_vec();
        loop_names.push(name.to_string());
        return Err(GrammarError::new(
            def.origin.clone(),
            format!("`{}` is defined in terms of itself: {}", name, loop_names.join(" -> ")),
        )
        .with_hint("break the loop — a variable cannot be its own input."));
    }
    seen.insert(name.to_string());
    visiting.push(name.to_string());

    let mut out = String::with_capacity(def.value.len());
    let mut rest = def.value.as_str();
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let (referenced, remainder) = split_reference(after);
        match referenced {
            // `$$` is a literal `$`, the one escape. Without it there is no way to write a
            // dollar sign in a value at all.
            None if after.starts_with('$') => {
                out.push('$');
                rest = &after[1..];
                continue;
            }
            None => {
                out.push('$');
                rest = after;
                continue;
            }
            Some(referenced) => {
                if !raw.contains_key(referenced) {
                    let what = if name == VALUE_PLACEHOLDER {
                        format!("`${}` is not defined", referenced)
                    } else {
                        format!("`{}` refers to `${}`, which is not defined", name, referenced)
                    };
                    return Err(GrammarError::new(def.origin.clone(), what).with_hint(
                        "every variable needs a top-level default in `vars` before it can be used.",
                    ));
                }
                resolve_one(referenced, raw, done, visiting, seen)?;
                out.push_str(done.get(referenced).map(String::as_str).unwrap_or(""));
                rest = remainder;
            }
        }
    }
    out.push_str(rest);

    visiting.pop();
    done.insert(name.to_string(), out);
    Ok(())
}

/// Read a variable reference off the front of `text`, returning it and what follows.
///
/// `${name}` exists so a reference can end where a name character would otherwise continue:
/// `$role_x` would read `role_x` as the name, and `${role}_x` says otherwise.
///
/// A name starts with a letter or `_`, never a digit, so `awk '{print $1}'` in a value is the
/// shell text it looks like and not a reference to a variable nobody could have declared.
fn split_reference(text: &str) -> (Option<&str>, &str) {
    if let Some(braced) = text.strip_prefix('{') {
        return match braced.find('}') {
            Some(end) if end > 0 => (Some(&braced[..end]), &braced[end + 1..]),
            _ => (None, text),
        };
    }
    if !text.starts_with(|c: char| c.is_alphabetic() || c == '_') {
        return (None, text);
    }
    let end = text
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(text.len());
    (Some(&text[..end]), &text[end..])
}

/// Substitute `$name` references in a value written outside `vars` — a `link:` target, a
/// `@version=`. Unknown names are an error, never left as literal text: a silently unexpanded
/// `$rle` would become a path with a dollar sign in it and fail somewhere with no mention of
/// the typo.
pub fn expand(value: &str, vars: &Vars, origin: &Origin) -> Result<String> {
    let as_defs: BTreeMap<String, Definition> = vars
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                Definition {
                    name: k.clone(),
                    value: v.clone(),
                    origin: origin.clone(),
                    conditional: false,
                },
            )
        })
        .collect();
    let refs: BTreeMap<String, &Definition> = as_defs.iter().map(|(k, v)| (k.clone(), v)).collect();

    let holder = Definition {
        name: VALUE_PLACEHOLDER.to_string(),
        value: value.to_string(),
        origin: origin.clone(),
        conditional: false,
    };
    let mut one = refs.clone();
    one.insert(VALUE_PLACEHOLDER.to_string(), &holder);

    let mut done: Vars = vars.clone();
    done.remove(VALUE_PLACEHOLDER);
    let mut visiting = Vec::new();
    let mut seen = HashSet::new();
    resolve_one(VALUE_PLACEHOLDER, &one, &mut done, &mut visiting, &mut seen)?;
    Ok(done.remove(VALUE_PLACEHOLDER).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(line: usize) -> Origin {
        Origin::new("vars", line)
    }

    fn top(name: &str, value: &str, line: usize) -> Definition {
        Definition {
            name: name.into(),
            value: value.into(),
            origin: origin(line),
            conditional: false,
        }
    }

    fn when(name: &str, value: &str, line: usize) -> Definition {
        Definition {
            name: name.into(),
            value: value.into(),
            origin: origin(line),
            conditional: true,
        }
    }

    #[test]
    fn a_default_survives_when_nothing_overrides_it() {
        let v = resolve(&[top("role", "desktop", 1)]).unwrap();
        assert_eq!(v["role"], "desktop");
    }

    #[test]
    fn a_matching_block_overrides_the_default() {
        let v = resolve(&[top("role", "desktop", 1), when("role", "travel", 5)]).unwrap();
        assert_eq!(v["role"], "travel");
    }

    #[test]
    fn a_variable_defined_only_inside_a_block_is_an_error() {
        // IX.3: otherwise `role` is undefined on every machine that is not the laptop, and
        // `when $role == travel` there has no answer.
        let err = resolve(&[when("role", "travel", 5)]).unwrap_err();
        assert!(err.what.contains("only defined inside a `when` block"), "{}", err);
        assert!(err.to_string().contains("vars:5"), "{}", err);
    }

    #[test]
    fn two_matching_blocks_that_disagree_name_both_lines() {
        let err = resolve(&[
            top("role", "desktop", 1),
            when("role", "travel", 5),
            when("role", "workstation", 9),
        ])
        .unwrap_err();
        assert!(err.what.contains("travel"), "{}", err);
        assert!(err.what.contains("workstation"), "{}", err);
        assert!(err.what.contains("vars:5"), "names the other line: {}", err);
    }

    #[test]
    fn two_matching_blocks_that_agree_are_redundant_not_wrong() {
        let v = resolve(&[
            top("role", "desktop", 1),
            when("role", "travel", 5),
            when("role", "travel", 9),
        ])
        .unwrap();
        assert_eq!(v["role"], "travel");
    }

    #[test]
    fn one_name_cannot_have_two_defaults() {
        let err = resolve(&[top("role", "a", 1), top("role", "b", 2)]).unwrap_err();
        assert!(err.what.contains("defined twice"), "{}", err);
    }

    #[test]
    fn a_value_may_be_built_from_another_variable() {
        let v = resolve(&[top("role", "render", 1), top("tier", "${role}-heavy", 2)]).unwrap();
        assert_eq!(v["tier"], "render-heavy");
    }

    #[test]
    fn a_reference_ends_at_a_non_name_character_without_braces() {
        let v = resolve(&[top("role", "render", 1), top("path", "/etc/$role/conf", 2)]).unwrap();
        assert_eq!(v["path"], "/etc/render/conf");
    }

    #[test]
    fn braces_are_what_let_a_reference_touch_a_name_character() {
        // `$role-heavy` reads `role-heavy` as the name; `-` ends a name but a reader would
        // not guess that, which is exactly why `${}` exists.
        let v = resolve(&[top("role", "render", 1), top("tier", "$role_x", 2)]);
        assert!(v.is_err(), "`$role_x` must not silently resolve to `render_x`");
    }

    #[test]
    fn derived_values_resolve_in_dependency_order_not_file_order() {
        // `tier` is defined before what it depends on.
        let v = resolve(&[
            top("tier", "${role}-heavy", 1),
            top("role", "render", 2),
            top("label", "${tier}!", 3),
        ])
        .unwrap();
        assert_eq!(v["label"], "render-heavy!");
    }

    #[test]
    fn an_override_is_visible_to_everything_derived_from_it() {
        let v = resolve(&[
            top("role", "desktop", 1),
            top("tier", "${role}-tier", 2),
            when("role", "travel", 5),
        ])
        .unwrap();
        assert_eq!(v["tier"], "travel-tier", "derived values must see the override");
    }

    #[test]
    fn a_cycle_names_the_whole_loop() {
        let err = resolve(&[
            top("a", "${b}", 1),
            top("b", "${c}", 2),
            top("c", "${a}", 3),
        ])
        .unwrap_err();
        assert!(err.what.contains("->"), "{}", err);
        assert!(err.what.contains('a') && err.what.contains('b') && err.what.contains('c'), "{}", err);
    }

    #[test]
    fn a_variable_that_references_itself_is_a_cycle() {
        let err = resolve(&[top("a", "${a}", 1)]).unwrap_err();
        assert!(err.what.contains("defined in terms of itself"), "{}", err);
    }

    #[test]
    fn referring_to_a_name_that_does_not_exist_is_an_error() {
        let err = resolve(&[top("tier", "${nosuch}-heavy", 1)]).unwrap_err();
        assert!(err.what.contains("nosuch"), "{}", err);
    }

    #[test]
    fn a_doubled_dollar_is_a_literal_one() {
        let v = resolve(&[top("price", "$$5", 1)]).unwrap();
        assert_eq!(v["price"], "$5");
    }

    #[test]
    fn a_shell_positional_is_not_a_variable_reference() {
        // A name cannot start with a digit, so `$1` is the shell text it looks like. Without
        // this rule every value carrying a shell snippet is an error about an undefined `1`.
        let v = resolve(&[top("cmd", "awk '{print $1}'", 1)]).unwrap();
        assert_eq!(v["cmd"], "awk '{print $1}'");
    }

    #[test]
    fn expand_substitutes_into_a_value_written_outside_vars() {
        let mut vars = Vars::new();
        vars.insert("role".into(), "travel".into());
        let out = expand("~/.config/$role/init.lua", &vars, &origin(3)).unwrap();
        assert_eq!(out, "~/.config/travel/init.lua");
    }

    #[test]
    fn expand_refuses_an_unknown_name_rather_than_leaving_it_literal() {
        // A silently unexpanded `$rle` becomes a path with a dollar in it and fails later,
        // somewhere else, with no mention of the typo.
        let vars = Vars::new();
        let err = expand("~/.config/$rle/init.lua", &vars, &origin(3)).unwrap_err();
        assert!(err.what.contains("rle"), "{}", err);
    }

    #[test]
    fn expand_leaves_a_value_with_no_references_alone() {
        let vars = Vars::new();
        assert_eq!(expand("plain/path", &vars, &origin(1)).unwrap(), "plain/path");
    }
}
