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

/// A variable's value. The four JSON scalar-and-list types (W2, ruled): a provider that returns
/// JSON has these already, and flattening them to strings at the boundary throws away information
/// the user produced on purpose.
///
/// Comparison is defined in [`Value::equals`] and [`Value::order`]; the load-bearing rule is that
/// there is **no cross-type coercion**, so `"1" == 1` is false rather than a silent true.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum Value {
    Bool(bool),
    /// A JSON number. `f64` because JSON does not distinguish integers, and a config value that
    /// is `3` and one that is `3.0` are the same claim.
    Num(f64),
    Str(String),
    List(Vec<Value>),
}

impl Value {
    /// Parse a value literal the way both a `vars` line and a `when` right-hand side read it, so
    /// `gpu = true` and `when $gpu == true` agree by construction rather than by luck. A `"quoted"`
    /// value is always a string, which is the only way to write the literal text `true` or `5`.
    pub fn parse_literal(text: &str) -> Value {
        let t = text.trim();
        if let Some(inner) = t.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            return Value::Str(inner.to_string());
        }
        if t == "true" {
            return Value::Bool(true);
        }
        if t == "false" {
            return Value::Bool(false);
        }
        if let Some(items) = parse_list_literal(t) {
            return Value::List(items.iter().map(|s| Value::parse_literal(s)).collect());
        }
        if let Some(n) = parse_number(t) {
            return Value::Num(n);
        }
        Value::Str(t.to_string())
    }

    /// The type's name, for an error that says what two things could not be compared.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Bool(_) => "boolean",
            Value::Num(_) => "number",
            Value::Str(_) => "string",
            Value::List(_) => "list",
        }
    }

    /// No cross-type coercion (W2): two values are equal only when they are the same type and the
    /// same value. Strings compare case-insensitively, which is the behaviour detected facts have
    /// always had (`os == LINUX`) and the one place case matters least.
    pub fn equals(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Num(a), Value::Num(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a.eq_ignore_ascii_case(b),
            (Value::List(a), Value::List(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.equals(y))
            }
            _ => false,
        }
    }

    /// Ordering is legal only between two numbers (W2): `"10" > "9"` is false under every string
    /// ordering and true under every intuition, so comparing strings by order is refused rather
    /// than answered wrongly. `None` means the comparison is not defined and the caller errors.
    pub fn order(&self, other: &Value) -> Option<std::cmp::Ordering> {
        match (self, other) {
            (Value::Num(a), Value::Num(b)) => a.partial_cmp(b),
            _ => None,
        }
    }

    /// The text form used when a value is substituted into a string (a `link:` target, a
    /// `@version=`). A list has no single text form, so the caller refuses it by name rather than
    /// inventing a joined string that would parse as one thing and mean another.
    pub fn as_interpolated(&self) -> std::result::Result<String, &'static str> {
        match self {
            Value::Str(s) => Ok(s.clone()),
            Value::Bool(b) => Ok(b.to_string()),
            Value::Num(n) => Ok(format_number(*n)),
            Value::List(_) => Err("list"),
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Str(s) => write!(f, "{}", s),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Num(n) => write!(f, "{}", format_number(*n)),
            Value::List(items) => {
                let parts: Vec<String> = items.iter().map(|v| v.to_string()).collect();
                write!(f, "[{}]", parts.join(", "))
            }
        }
    }
}

/// Print `3` not `3.0`, but `1.5` as itself — the number a user wrote is the number they see.
fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.is_finite() && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{}", n)
    }
}

/// A number literal, and only a literal: a finite decimal with at most one point and an optional
/// leading sign. `1e5`, `inf`, `nan` and `1.2.3` are not numbers, so a version string stays a
/// string and shell text is never mistaken for arithmetic.
fn parse_number(t: &str) -> Option<f64> {
    if t.is_empty() {
        return None;
    }
    let body = t.strip_prefix(['+', '-']).unwrap_or(t);
    if body.is_empty() {
        return None;
    }
    let mut seen_dot = false;
    for c in body.chars() {
        match c {
            '0'..='9' => {}
            '.' if !seen_dot => seen_dot = true,
            _ => return None,
        }
    }
    t.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Split `[a, b, [c, d]]` into its top-level element texts, tracking bracket depth so a nested
/// list is one element. `None` when the text is not a bracketed list at all.
fn parse_list_literal(t: &str) -> Option<Vec<String>> {
    let inner = t.strip_prefix('[')?.strip_suffix(']')?;
    if inner.trim().is_empty() {
        return Some(Vec::new());
    }
    let mut items = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let bytes = inner.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'[' => depth += 1,
            b']' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                items.push(inner[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    items.push(inner[start..].trim().to_string());
    Some(items)
}

/// The resolved set. A `BTreeMap` so `plan` and `sync` print variables in the same order and a
/// diff of two machines' resolved vars is readable.
pub type Vars = BTreeMap<String, Value>;

/// Where each resolved variable's value came from — the winning definition's line for a line
/// file, the provider file for a script or program. Kept beside [`Vars`] rather than inside it
/// so the value path (gating, the plan, the diff) never carries provenance it does not read;
/// only the tooling that explains a variable (`linix vars`, `why`) asks for this (W11/W12).
pub type VarOrigins = BTreeMap<String, Origin>;

/// One `NAME = VALUE` line. `conditional` is whether it came from inside a `when` block, which
/// is what IX.3 turns on: a top-level line defines a name, a conditional one may only override.
#[derive(Debug, Clone)]
pub struct Definition {
    pub name: String,
    pub value: String,
    pub origin: Origin,
    pub conditional: bool,
}

/// Resolve definitions that `when` has already gated down to the ones that apply here.
///
/// Order of business, and each step is a rule from IX.3:
/// 1. Every name needs a top-level definition; a `when` block may not introduce one.
/// 2. Two matching `when` blocks setting one name differently is a contradiction, not a
///    last-wins — the same rule II.7.5 applies to package declarations.
/// 3. Values may reference other variables, so they resolve in dependency order, and a cycle
///    is an error naming the loop.
pub fn resolve(defs: &[Definition]) -> Result<Vars> {
    let raw = winning_defs(defs)?;
    interpolate_all(&raw)
}

/// [`resolve`], plus the origin of the winning definition for each name — the line that set it,
/// or the top-level default when no block overrode it. For the tooling that has to say *where* a
/// value came from (W11/W12); the value path uses [`resolve`] and never pays for this.
pub fn resolve_with_origins(defs: &[Definition]) -> Result<(Vars, VarOrigins)> {
    let raw = winning_defs(defs)?;
    let origins: VarOrigins = raw.iter().map(|(k, d)| (k.clone(), d.origin.clone())).collect();
    let values = interpolate_all(&raw)?;
    Ok((values, origins))
}

/// The one definition that wins for each name: its top-level default, replaced by the single
/// `when` override that applies here. The IX.3 rules (a name needs a default, two matching
/// blocks that disagree is a contradiction) are enforced here, so both [`resolve`] and
/// [`resolve_with_origins`] share exactly one implementation of them.
fn winning_defs(defs: &[Definition]) -> Result<BTreeMap<String, &Definition>> {
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
    Ok(raw)
}

/// Resolve every value into a typed [`Value`], substituting `$other` references, in dependency
/// order.
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

    let value = resolve_value(def, raw, done, visiting, seen)?;

    visiting.pop();
    done.insert(name.to_string(), value);
    Ok(())
}

/// Turn one definition's raw text into a typed value. A value that is exactly one reference
/// (`alias = $other`) inherits that variable's type, so a list can be aliased; any other value
/// containing `$` is string interpolation and its result is a string; a value with no reference
/// is a typed literal.
fn resolve_value(
    def: &Definition,
    raw: &BTreeMap<String, &Definition>,
    done: &mut Vars,
    visiting: &mut Vec<String>,
    seen: &mut HashSet<String>,
) -> Result<Value> {
    let text = def.value.trim();
    if let Some(referenced) = sole_reference(text) {
        require_defined(referenced, raw, &def.origin, None)?;
        resolve_one(referenced, raw, done, visiting, seen)?;
        return Ok(done.get(referenced).cloned().unwrap_or(Value::Str(String::new())));
    }
    if text.contains('$') {
        let s = interpolate_string(&def.value, Some(&def.name), &def.origin, raw, done, visiting, seen)?;
        return Ok(Value::Str(s));
    }
    Ok(Value::parse_literal(&def.value))
}

/// The single `$name`/`${name}` a value consists of entirely, or `None` if it is anything else.
fn sole_reference(text: &str) -> Option<&str> {
    let after = text.strip_prefix('$')?;
    let (name, rest) = split_reference(after);
    match name {
        Some(n) if rest.is_empty() => Some(n),
        _ => None,
    }
}

fn require_defined(
    referenced: &str,
    raw: &BTreeMap<String, &Definition>,
    origin: &Origin,
    referrer: Option<&str>,
) -> Result<()> {
    if raw.contains_key(referenced) {
        return Ok(());
    }
    let what = match referrer {
        Some(name) if name != VALUE_PLACEHOLDER => {
            format!("`{}` refers to `${}`, which is not defined", name, referenced)
        }
        _ => format!("`${}` is not defined", referenced),
    };
    Err(GrammarError::new(origin.clone(), what)
        .with_hint("every variable needs a top-level default in `vars` before it can be used."))
}

/// Substitute `$name` references into `text`, resolving each referenced variable first. The
/// result is always a string — this is text composition, and a referenced list has no text form.
#[allow(clippy::too_many_arguments)]
fn interpolate_string(
    text: &str,
    referrer: Option<&str>,
    origin: &Origin,
    raw: &BTreeMap<String, &Definition>,
    done: &mut Vars,
    visiting: &mut Vec<String>,
    seen: &mut HashSet<String>,
) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        // `$$` is a literal `$`, the one escape. Without it there is no way to write a dollar
        // sign in a value at all.
        if let Some(tail) = after.strip_prefix('$') {
            out.push('$');
            rest = tail;
            continue;
        }
        let (referenced, remainder) = split_reference(after);
        match referenced {
            None => {
                out.push('$');
                rest = after;
            }
            Some(referenced) => {
                require_defined(referenced, raw, origin, referrer)?;
                resolve_one(referenced, raw, done, visiting, seen)?;
                let value = done.get(referenced).cloned().unwrap_or(Value::Str(String::new()));
                match value.as_interpolated() {
                    Ok(s) => out.push_str(&s),
                    Err(_) => {
                        return Err(GrammarError::new(
                            origin.clone(),
                            format!("`${}` is a list and cannot go inside a value", referenced),
                        )
                        .with_hint(
                            "a list has no single text form; reference it in a `when` condition \
                             instead, or name a scalar variable here.",
                        ));
                    }
                }
                rest = remainder;
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// The variables that differ between two resolved sets, for W13's "what changed since the last
/// sync" note. Each entry is `(name, before, after)`; a `None` side means the variable was added
/// or has gone. Equality follows [`Value::equals`], so a change that is only letter-case is not
/// reported as a change.
pub fn diff(before: &Vars, after: &Vars) -> Vec<(String, Option<Value>, Option<Value>)> {
    let names: std::collections::BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    let mut out = Vec::new();
    for name in names {
        match (before.get(name), after.get(name)) {
            (Some(a), Some(b)) if !a.equals(b) => {
                out.push((name.clone(), Some(a.clone()), Some(b.clone())))
            }
            (None, Some(b)) => out.push((name.clone(), None, Some(b.clone()))),
            (Some(a), None) => out.push((name.clone(), Some(a.clone()), None)),
            _ => {}
        }
    }
    out
}

/// Every `$name`/`${name}` a text references, skipping the `$$` escape and shell positionals
/// (`$1`). For `check` to find a variable defined but referenced nowhere (W5): an unused name is
/// a note, not an error, because on a fleet it usually means the block that used it was deleted
/// on this branch. This reads references statically from a file's text — the fleet's whole set,
/// not just the arms this host reached — so a variable used only in another machine's `when`
/// block is correctly seen as used.
pub fn referenced_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find('$') {
        let after = &rest[at + 1..];
        if let Some(tail) = after.strip_prefix('$') {
            rest = tail;
            continue;
        }
        let (referenced, remainder) = split_reference(after);
        match referenced {
            Some(name) => {
                names.push(name.to_string());
                rest = remainder;
            }
            None => rest = after,
        }
    }
    names
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
/// the typo. A referenced list is refused by name for the same reason as inside `vars`.
pub fn expand(value: &str, vars: &Vars, origin: &Origin) -> Result<String> {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        if let Some(tail) = after.strip_prefix('$') {
            out.push('$');
            rest = tail;
            continue;
        }
        let (referenced, remainder) = split_reference(after);
        match referenced {
            None => {
                out.push('$');
                rest = after;
            }
            Some(referenced) => {
                let Some(v) = vars.get(referenced) else {
                    return Err(GrammarError::new(
                        origin.clone(),
                        format!("`${}` is not defined", referenced),
                    )
                    .with_hint(
                        "every variable needs a top-level default in `vars` before it can be used.",
                    ));
                };
                match v.as_interpolated() {
                    Ok(s) => out.push_str(&s),
                    Err(_) => {
                        return Err(GrammarError::new(
                            origin.clone(),
                            format!("`${}` is a list and cannot go inside a value", referenced),
                        )
                        .with_hint(
                            "a list has no single text form; reference it in a `when` condition \
                             instead, or name a scalar variable here.",
                        ));
                    }
                }
                rest = remainder;
            }
        }
    }
    out.push_str(rest);
    Ok(out)
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

    fn str_val(s: &str) -> Value {
        Value::Str(s.into())
    }

    #[test]
    fn a_default_survives_when_nothing_overrides_it() {
        let v = resolve(&[top("role", "desktop", 1)]).unwrap();
        assert_eq!(v["role"], str_val("desktop"));
    }

    #[test]
    fn a_matching_block_overrides_the_default() {
        let v = resolve(&[top("role", "desktop", 1), when("role", "travel", 5)]).unwrap();
        assert_eq!(v["role"], str_val("travel"));
    }

    #[test]
    fn the_origin_is_the_winning_definition_not_the_default() {
        // W11/W12: `why` and `linix vars` must point at the line that actually set the value —
        // the override when one applies, the default otherwise.
        let (v, o) = resolve_with_origins(&[
            top("role", "desktop", 1),
            when("role", "travel", 5),
            top("gpu", "none", 2),
        ])
        .unwrap();
        assert_eq!(v["role"], str_val("travel"));
        assert_eq!(o["role"].line, 5, "the override line, not the default");
        assert_eq!(o["gpu"].line, 2, "an unoverridden default names its own line");
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
        assert_eq!(v["role"], str_val("travel"));
    }

    #[test]
    fn one_name_cannot_have_two_defaults() {
        let err = resolve(&[top("role", "a", 1), top("role", "b", 2)]).unwrap_err();
        assert!(err.what.contains("defined twice"), "{}", err);
    }

    #[test]
    fn a_value_may_be_built_from_another_variable() {
        let v = resolve(&[top("role", "render", 1), top("tier", "${role}-heavy", 2)]).unwrap();
        assert_eq!(v["tier"], str_val("render-heavy"));
    }

    #[test]
    fn a_reference_ends_at_a_non_name_character_without_braces() {
        let v = resolve(&[top("role", "render", 1), top("path", "/etc/$role/conf", 2)]).unwrap();
        assert_eq!(v["path"], str_val("/etc/render/conf"));
    }

    #[test]
    fn braces_are_what_let_a_reference_touch_a_name_character() {
        let v = resolve(&[top("role", "render", 1), top("tier", "$role_x", 2)]);
        assert!(v.is_err(), "`$role_x` must not silently resolve to `render_x`");
    }

    #[test]
    fn derived_values_resolve_in_dependency_order_not_file_order() {
        let v = resolve(&[
            top("tier", "${role}-heavy", 1),
            top("role", "render", 2),
            top("label", "${tier}!", 3),
        ])
        .unwrap();
        assert_eq!(v["label"], str_val("render-heavy!"));
    }

    #[test]
    fn an_override_is_visible_to_everything_derived_from_it() {
        let v = resolve(&[
            top("role", "desktop", 1),
            top("tier", "${role}-tier", 2),
            when("role", "travel", 5),
        ])
        .unwrap();
        assert_eq!(v["tier"], str_val("travel-tier"), "derived values must see the override");
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
        assert_eq!(v["price"], str_val("$5"));
    }

    #[test]
    fn a_shell_positional_is_not_a_variable_reference() {
        let v = resolve(&[top("cmd", "awk '{print $1}'", 1)]).unwrap();
        assert_eq!(v["cmd"], str_val("awk '{print $1}'"));
    }

    // --- W2: types ------------------------------------------------------------------------

    #[test]
    fn a_bare_true_or_false_is_a_boolean_not_a_string() {
        let v = resolve(&[top("gpu", "true", 1), top("headless", "false", 2)]).unwrap();
        assert_eq!(v["gpu"], Value::Bool(true));
        assert_eq!(v["headless"], Value::Bool(false));
    }

    #[test]
    fn a_plain_number_is_a_number_and_a_version_is_not() {
        let v = resolve(&[top("count", "3", 1), top("ver", "1.6.0", 2), top("ratio", "1.5", 3)])
            .unwrap();
        assert_eq!(v["count"], Value::Num(3.0));
        assert_eq!(v["ver"], str_val("1.6.0"));
        assert_eq!(v["ratio"], Value::Num(1.5));
    }

    #[test]
    fn quotes_force_a_string_even_when_it_looks_like_another_type() {
        let v = resolve(&[top("flag", "\"true\"", 1), top("n", "\"5\"", 2)]).unwrap();
        assert_eq!(v["flag"], str_val("true"));
        assert_eq!(v["n"], str_val("5"));
    }

    #[test]
    fn a_bracketed_value_is_a_list_of_typed_elements() {
        let v = resolve(&[top("tags", "[travel, work]", 1), top("ports", "[22, 80]", 2)]).unwrap();
        assert_eq!(v["tags"], Value::List(vec![str_val("travel"), str_val("work")]));
        assert_eq!(v["ports"], Value::List(vec![Value::Num(22.0), Value::Num(80.0)]));
    }

    #[test]
    fn a_value_that_is_exactly_one_reference_inherits_its_type() {
        let v = resolve(&[
            top("tags", "[a, b]", 1),
            top("alias", "$tags", 2),
            top("count", "3", 3),
            top("n", "${count}", 4),
        ])
        .unwrap();
        assert_eq!(v["alias"], Value::List(vec![str_val("a"), str_val("b")]));
        assert_eq!(v["n"], Value::Num(3.0), "a sole reference keeps the number type");
    }

    #[test]
    fn a_list_cannot_be_interpolated_into_a_string_value() {
        let err = resolve(&[top("tags", "[a, b]", 1), top("label", "x-${tags}", 2)]).unwrap_err();
        assert!(err.what.contains("list"), "{}", err);
    }

    #[test]
    fn value_equality_has_no_cross_type_coercion() {
        assert!(Value::Num(1.0).equals(&Value::Num(1.0)));
        assert!(!Value::Str("1".into()).equals(&Value::Num(1.0)));
        assert!(!Value::Bool(true).equals(&Value::Str("true".into())));
        assert!(Value::Str("Linux".into()).equals(&Value::Str("linux".into())));
    }

    #[test]
    fn ordering_is_defined_only_between_numbers() {
        use std::cmp::Ordering;
        assert_eq!(Value::Num(9.0).order(&Value::Num(10.0)), Some(Ordering::Less));
        assert_eq!(Value::Str("9".into()).order(&Value::Str("10".into())), None);
        assert_eq!(Value::Num(1.0).order(&Value::Str("1".into())), None);
    }

    #[test]
    fn a_number_displays_without_a_trailing_zero() {
        assert_eq!(Value::Num(3.0).to_string(), "3");
        assert_eq!(Value::Num(1.5).to_string(), "1.5");
    }

    // --- expand ---------------------------------------------------------------------------

    #[test]
    fn expand_substitutes_into_a_value_written_outside_vars() {
        let mut vars = Vars::new();
        vars.insert("role".into(), str_val("travel"));
        let out = expand("~/.config/$role/init.lua", &vars, &origin(3)).unwrap();
        assert_eq!(out, "~/.config/travel/init.lua");
    }

    #[test]
    fn expand_stringifies_a_number_or_boolean() {
        let mut vars = Vars::new();
        vars.insert("n".into(), Value::Num(5.0));
        assert_eq!(expand("v$n", &vars, &origin(1)).unwrap(), "v5");
    }

    #[test]
    fn expand_refuses_a_list_rather_than_joining_it() {
        let mut vars = Vars::new();
        vars.insert("tags".into(), Value::List(vec![str_val("a")]));
        let err = expand("x-$tags", &vars, &origin(1)).unwrap_err();
        assert!(err.what.contains("list"), "{}", err);
    }

    #[test]
    fn expand_refuses_an_unknown_name_rather_than_leaving_it_literal() {
        let vars = Vars::new();
        let err = expand("~/.config/$rle/init.lua", &vars, &origin(3)).unwrap_err();
        assert!(err.what.contains("rle"), "{}", err);
    }

    #[test]
    fn expand_leaves_a_value_with_no_references_alone() {
        let vars = Vars::new();
        assert_eq!(expand("plain/path", &vars, &origin(1)).unwrap(), "plain/path");
    }

    #[test]
    fn diff_reports_changed_added_and_gone_but_not_case_only() {
        let mut before = Vars::new();
        before.insert("role".into(), str_val("travel"));
        before.insert("gpu".into(), str_val("nvidia"));
        before.insert("os".into(), str_val("Linux"));
        let mut after = Vars::new();
        after.insert("role".into(), str_val("desktop")); // changed
        after.insert("os".into(), str_val("linux")); // case only — not a change
        after.insert("cores".into(), Value::Num(8.0)); // added
        // gpu is gone
        let d = diff(&before, &after);
        assert_eq!(d.len(), 3, "role changed, cores added, gpu gone — os is case-only: {:?}", d);
        assert!(d.iter().any(|(n, a, b)| n == "role"
            && a.as_ref().unwrap().equals(&str_val("travel"))
            && b.as_ref().unwrap().equals(&str_val("desktop"))));
        assert!(d.iter().any(|(n, a, b)| n == "cores" && a.is_none() && b.is_some()));
        assert!(d.iter().any(|(n, a, b)| n == "gpu" && a.is_some() && b.is_none()));
    }

    #[test]
    fn referenced_names_finds_every_reference_and_skips_escapes() {
        assert_eq!(referenced_names("when $role == travel"), vec!["role"]);
        assert_eq!(referenced_names("${a}/${b}/$c"), vec!["a", "b", "c"]);
        // `$$` is an escape and `$1` a shell positional — neither is a variable reference.
        assert!(referenced_names("cost $$5 and awk '{print $1}'").is_empty());
        assert!(referenced_names("no refs here").is_empty());
    }

    #[test]
    fn parse_number_rejects_infinity_and_multiple_dots() {
        assert_eq!(parse_number("inf"), None);
        assert_eq!(parse_number("1.2.3"), None);
        assert_eq!(parse_number("1e5"), None);
        assert_eq!(parse_number("-4"), Some(-4.0));
    }
}
