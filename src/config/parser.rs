use crate::core::{Error, Result};
use crate::model::vars::Value;

/// Facts about the host used to evaluate `when` conditionals, so a single shared repo can
/// serve a heterogeneous fleet (Linux + macOS + Windows). The model reads these when it
/// resolves `when` gates in `active`, profiles and modules (II.2).
#[derive(Debug, Clone)]
pub struct HostFacts {
    pub os: String,
    pub arch: String,
    pub host: String,
    /// The distribution family: `debian`, `fedora`, `arch`, `suse`, … On a system with no
    /// distributions this is the OS name, so `family` always answers something.
    ///
    /// `os` already answers linux-or-windows, which is why this does not.
    pub family: String,
    /// The resolved `vars` (Part IX), reached as `$name`. Empty until a caller supplies them,
    /// so a `when $role == …` in a repo with no `vars` file is an unknown key and says so.
    pub vars: crate::model::vars::Vars,
}

impl HostFacts {
    /// Gather this machine's facts.
    pub fn current() -> Self {
        Self {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            host: crate::config::Config::get_hostname(),
            family: distro_family().unwrap_or_else(|| std::env::consts::OS.to_string()),
            vars: Default::default(),
        }
    }

    fn value_for(&self, key: &str) -> Option<Value> {
        // IX.4: `$name` is a variable you decided, `name` is a fact the machine reported. The
        // sigil is what lets LiNix add a detected fact without changing the meaning of a file
        // where someone happened to use that word as a variable.
        if let Some(var) = key.strip_prefix('$') {
            return self.vars.get(var).cloned();
        }
        // A detected fact is always a string (W2), so `$var == family` follows the same
        // no-coercion equality as any other comparison.
        match key {
            "os" => Some(Value::Str(self.os.clone())),
            "arch" => Some(Value::Str(self.arch.clone())),
            "host" | "hostname" => Some(Value::Str(self.host.clone())),
            "family" => Some(Value::Str(self.family.clone())),
            _ => None,
        }
    }

    /// The resolved variables this run evaluates `$name` against.
    ///
    /// Resolved once per invocation and carried, never recomputed: a provider may read the
    /// clock or shell out, so a second resolution can disagree with the first, and a `plan`
    /// that disagrees with the `sync` executing it is not a plan (IX.6).
    pub fn with_vars(mut self, vars: crate::model::vars::Vars) -> Self {
        self.vars = vars;
        self
    }
}

/// The distribution family — `debian`, `fedora`, `arch`, `suse`, … — read from
/// `/etc/os-release`, which is the only place that answers it. `None` where there are no
/// distributions to tell apart.
pub fn distro_family() -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let text = std::fs::read_to_string("/etc/os-release").ok()?;
    parse_os_release_family(&text)
}

/// `ID_LIKE` names the family and `ID` names the distribution, so a derivative
/// (`linuxmint`, whose `ID_LIKE` is `ubuntu debian`) resolves to the family that decides
/// which artifact installs. `ID_LIKE` is checked first for exactly that reason.
fn parse_os_release_family(text: &str) -> Option<String> {
    let field = |key: &str| -> Option<String> {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(key)?.strip_prefix('='))
            .map(|v| v.trim().trim_matches('"').to_lowercase())
    };

    const FAMILIES: [&str; 6] = ["debian", "fedora", "rhel", "suse", "arch", "alpine"];
    let id_like = field("ID_LIKE").unwrap_or_default();
    for family in FAMILIES {
        if id_like.split_whitespace().any(|w| w == family) {
            return Some(family.to_string());
        }
    }

    let id = field("ID")?;
    // `ubuntu` has no `ID_LIKE` on some releases and is not itself in the family list.
    match id.as_str() {
        "ubuntu" | "debian" | "raspbian" => Some("debian".into()),
        "fedora" => Some("fedora".into()),
        "centos" | "rhel" | "rocky" | "almalinux" => Some("rhel".into()),
        "opensuse" | "sles" => Some("suse".into()),
        "arch" | "manjaro" | "endeavouros" => Some("arch".into()),
        "alpine" => Some("alpine".into()),
        other => Some(other.to_string()),
    }
}

/// Evaluate a `when` predicate against host facts. Forms: `os == linux`, `arch != x86_64`,
/// `$count > 3`, `os in [linux, macos]`. Values are typed (W2): comparison is between values of
/// the same type, ordering is numbers only, and there is no truthiness — a bare `$flag` is an
/// error, not a non-empty test. Pure — unit tested.
pub fn eval_when(pred: &str, facts: &HostFacts) -> Result<bool> {
    let pred = pred.trim();

    // Membership form: `key in [a, b, c]`  (brackets optional), or `key in $listvar`.
    if let Some((key, rest)) = pred.split_once(" in ") {
        let actual = lhs_value(key.trim(), facts)?;
        let elements = list_operand(rest.trim(), facts)?;
        return Ok(elements.iter().any(|el| actual.equals(el)));
    }

    // Comparison form. Two-char operators are matched before one-char ones so `<=` is not read
    // as `<`; every operator is distinct so order among the two-char set does not matter.
    for op in ["==", "!=", "<=", ">="] {
        if let Some((l, r)) = pred.split_once(op) {
            return compare(l.trim(), op, r.trim(), facts);
        }
    }
    for op in ["<", ">"] {
        if let Some((l, r)) = pred.split_once(op) {
            return compare(l.trim(), op, r.trim(), facts);
        }
    }

    // No operator. A bare `$flag` is the truthiness trap W3 forbids: `false` is a non-empty
    // string, so `when $gpu` would fire on `gpu = false`. Refuse it, and say what to write.
    if let Some(var) = pred.strip_prefix('$') {
        return Err(Error::Config(format!(
            "`{}` is not a condition on its own — there is no truthiness; write `${} == true` or another comparison",
            pred, var
        )));
    }
    Err(Error::Config(format!(
        "invalid `when` predicate '{}' (use `key == value`, `key != value`, `key < number`, or `key in [..]`)",
        pred
    )))
}

/// The left side of a comparison is always a key — a detected fact or a `$variable` — and an
/// unknown one is an error, never a silent false (a typo'd `$rle` that read as false is a block
/// that never fires and never complains, IX.3).
fn lhs_value(key: &str, facts: &HostFacts) -> Result<Value> {
    facts
        .value_for(key)
        .ok_or_else(|| Error::Config(format!("unknown `when` key '{}'", key)))
}

/// The right side is a value: another `$variable`, or a literal read with the same rules a `vars`
/// line uses, so `$gpu == true` and `gpu = true` mean the same thing.
fn rhs_value(token: &str, facts: &HostFacts) -> Result<Value> {
    if token.starts_with('$') {
        return lhs_value(token, facts);
    }
    Ok(Value::parse_literal(token))
}

fn compare(left: &str, op: &str, right: &str, facts: &HostFacts) -> Result<bool> {
    let a = lhs_value(left, facts)?;
    let b = rhs_value(right, facts)?;
    match op {
        "==" => Ok(a.equals(&b)),
        "!=" => Ok(!a.equals(&b)),
        _ => {
            let ord = a.order(&b).ok_or_else(|| {
                Error::Config(format!(
                    "`{} {} {}` compares a {} with a {}; ordering is only defined between numbers",
                    left,
                    op,
                    right,
                    a.type_name(),
                    b.type_name()
                ))
            })?;
            Ok(match op {
                "<" => ord.is_lt(),
                ">" => ord.is_gt(),
                "<=" => ord.is_le(),
                ">=" => ord.is_ge(),
                _ => unreachable!("operator set is closed"),
            })
        }
    }
}

/// The right side of `in`: a bracketed literal list, or a `$variable` that must itself be a list.
fn list_operand(rest: &str, facts: &HostFacts) -> Result<Vec<Value>> {
    if rest.starts_with('$') {
        return match lhs_value(rest, facts)? {
            Value::List(items) => Ok(items),
            other => Err(Error::Config(format!(
                "`{}` is a {}, not a list; `in` needs a list on the right",
                rest,
                other.type_name()
            ))),
        };
    }
    let inner = rest.trim_start_matches('[').trim_end_matches(']');
    Ok(inner
        .split(',')
        .map(|s| Value::parse_literal(s.trim()))
        .collect())
}

/// Split a removal target like `backend:name[@opts]` into `(Some(backend), bare_name)`
/// when the prefix names a real backend, or `(None, name)` otherwise. `@options` are
/// stripped from the name. `is_known_backend` decides whether a `prefix:` is a backend
/// (so package names that legitimately contain a colon aren't misread as `backend:name`).
///
/// This is the parsing `remove` must use to match how `install` reads its arguments —
/// passing the whole `backend:name` string to a backend's `info()`/`remove()` (which
/// expect the *bare* name) silently makes `remove backend:pkg` a no-op. It consults the
/// registry (unlike a blind `split_once(':')`), which is why it is not one of the parsers
/// C13 retired.
pub fn split_removal_target(
    input: &str,
    is_known_backend: impl Fn(&str) -> bool,
) -> (Option<String>, String) {
    let (backend, name_part) = match input.split_once(':') {
        Some((b, n)) if is_known_backend(b) => (Some(b.to_string()), n),
        _ => (None, input),
    };
    let bare = name_part.split('@').next().unwrap_or(name_part).to_string();
    (backend, bare)
}

#[cfg(test)]
mod conditional_tests {
    use super::*;

    fn facts() -> HostFacts {
        HostFacts {
            os: "linux".into(),
            arch: "x86_64".into(),
            host: "laptop".into(),
            family: "debian".into(),
            vars: Default::default(),
        }
    }

    fn with_role(role: &str) -> HostFacts {
        let mut vars = crate::model::vars::Vars::new();
        vars.insert("role".into(), Value::Str(role.into()));
        facts().with_vars(vars)
    }

    /// U26: a family that cannot be shown to be X makes `family == X` false, not an error.
    /// On a BSD, `HostFacts::current` falls back to the OS name (`freebsd`), so `family`
    /// answers something real and `== debian` is correctly false — no silent else-branch from
    /// an empty family, and no hard error the owner ruled against.
    #[test]
    fn a_family_that_is_not_debian_makes_the_comparison_false() {
        let bsd = HostFacts {
            os: "freebsd".into(),
            family: "freebsd".into(),
            ..facts()
        };
        assert!(
            !eval_when("family == debian", &bsd).unwrap(),
            "freebsd is not debian"
        );
        assert!(
            eval_when("family == freebsd", &bsd).unwrap(),
            "but it IS freebsd"
        );
        assert!(eval_when("family != debian", &bsd).unwrap());
    }

    /// The OS-name fallback is what guarantees `family` is never empty — an empty family is
    /// what would make every `when family ==` silently false, which is the failure U26 exists
    /// to close. `current()` uses `distro_family().unwrap_or(OS)`, so a host with no
    /// `/etc/os-release` still answers the OS name rather than "".
    #[test]
    fn family_is_never_empty() {
        let f = HostFacts::current();
        assert!(!f.family.is_empty(), "family must always answer something");
    }

    #[test]
    fn a_variable_is_reached_with_the_sigil() {
        let f = with_role("travel");
        assert!(eval_when("$role == travel", &f).unwrap());
        assert!(!eval_when("$role == desktop", &f).unwrap());
        assert!(eval_when("$role in [travel, workstation]", &f).unwrap());
    }

    #[test]
    fn a_variable_can_never_shadow_a_detected_fact() {
        // IX.4: the sigil exists so LiNix can add a detected fact forever without changing the
        // meaning of a file where someone used that word as a variable name.
        let mut vars = crate::model::vars::Vars::new();
        vars.insert("os".into(), Value::Str("definitely-not-linux".into()));
        let f = facts().with_vars(vars);
        assert!(
            eval_when("os == linux", &f).unwrap(),
            "`os` must stay the detected fact"
        );
        assert!(eval_when("$os == definitely-not-linux", &f).unwrap());
    }

    #[test]
    fn typed_comparisons_have_no_cross_type_coercion() {
        let mut vars = crate::model::vars::Vars::new();
        vars.insert("gpu".into(), Value::Bool(true));
        vars.insert("cores".into(), Value::Num(8.0));
        vars.insert("role".into(), Value::Str("travel".into()));
        vars.insert(
            "tags".into(),
            Value::List(vec![Value::Str("travel".into())]),
        );
        let f = facts().with_vars(vars);

        assert!(eval_when("$gpu == true", &f).unwrap());
        assert!(!eval_when("$gpu == false", &f).unwrap());
        // A boolean is not the string "true": no coercion (W2).
        assert!(!eval_when("$gpu == \"true\"", &f).unwrap());

        assert!(eval_when("$cores > 4", &f).unwrap());
        assert!(eval_when("$cores <= 8", &f).unwrap());
        assert!(!eval_when("$cores < 8", &f).unwrap());

        // `in` tests whether the left value is a member of the right list.
        assert!(eval_when("$role in [travel, work]", &f).unwrap());
        assert!(!eval_when("$role in [home, office]", &f).unwrap());
        // The right side may be a list-typed variable.
        assert!(eval_when("$role in $tags", &f).unwrap());
        // A non-list on the right of `in` is refused.
        assert!(eval_when("$role in $gpu", &f).is_err());
    }

    #[test]
    fn ordering_a_string_is_refused_not_answered_wrongly() {
        let mut vars = crate::model::vars::Vars::new();
        vars.insert("ver".into(), Value::Str("10".into()));
        let f = facts().with_vars(vars);
        // `"10" > "9"` is false under string ordering and true under intuition; refuse it (W2).
        assert!(eval_when("$ver > 9", &f).is_err());
    }

    #[test]
    fn a_bare_variable_is_not_a_condition() {
        // W3: `false` is a non-empty string, so a truthiness test would fire on `gpu = false`.
        let mut vars = crate::model::vars::Vars::new();
        vars.insert("gpu".into(), Value::Bool(false));
        let f = facts().with_vars(vars);
        let err = eval_when("$gpu", &f).unwrap_err();
        assert!(err.to_string().contains("== true"), "{}", err);
    }

    #[test]
    fn an_undefined_variable_is_an_error_not_a_silent_false() {
        // A typo'd `$rle` that read as false would be a block that never fires and never
        // complains, which is the failure IX.3 exists to delete.
        let f = with_role("travel");
        assert!(eval_when("$rle == travel", &f).is_err());
    }

    #[test]
    fn eval_equality_and_inequality() {
        let f = facts();
        assert!(eval_when("os == linux", &f).unwrap());
        assert!(!eval_when("os == macos", &f).unwrap());
        assert!(eval_when("os != windows", &f).unwrap());
        assert!(!eval_when("arch != x86_64", &f).unwrap());
        // case-insensitive value match
        assert!(eval_when("os == LINUX", &f).unwrap());
    }

    #[test]
    fn eval_membership() {
        let f = facts();
        assert!(eval_when("os in [linux, macos]", &f).unwrap());
        assert!(!eval_when("os in [windows, macos]", &f).unwrap());
        assert!(eval_when("host in [laptop, desktop]", &f).unwrap());
    }

    #[test]
    fn eval_rejects_unknown_key_and_bad_syntax() {
        let f = facts();
        assert!(eval_when("kernel == 6.1", &f).is_err());
        assert!(eval_when("os linux", &f).is_err());
    }

    #[test]
    fn family_answers_the_distribution_and_os_answers_the_kernel() {
        let f = facts();
        assert!(eval_when("family == debian", &f).unwrap());
        assert!(eval_when("os == linux", &f).unwrap());
        assert!(
            !eval_when("family == linux", &f).unwrap(),
            "`family` must not still be answering the os question"
        );
    }

    #[test]
    fn a_derivative_resolves_to_the_family_that_decides_the_artifact() {
        let mint = "NAME=\"Linux Mint\"\nID=linuxmint\nID_LIKE=\"ubuntu debian\"\n";
        assert_eq!(parse_os_release_family(mint).as_deref(), Some("debian"));
    }

    #[test]
    fn ubuntu_is_debian_even_without_an_id_like() {
        assert_eq!(
            parse_os_release_family("ID=ubuntu\nVERSION_ID=\"24.04\"\n").as_deref(),
            Some("debian")
        );
    }

    #[test]
    fn quotes_around_a_value_are_not_part_of_it() {
        assert_eq!(
            parse_os_release_family("ID=\"fedora\"\n").as_deref(),
            Some("fedora")
        );
    }

    #[test]
    fn the_rhel_derivatives_share_one_family() {
        for id in ["centos", "rocky", "almalinux"] {
            assert_eq!(
                parse_os_release_family(&format!("ID={}\n", id)).as_deref(),
                Some("rhel"),
                "{} should be rhel",
                id
            );
        }
    }

    #[test]
    fn an_unknown_distribution_reports_its_own_name_rather_than_nothing() {
        assert_eq!(
            parse_os_release_family("ID=voidlinux\n").as_deref(),
            Some("voidlinux")
        );
    }

    #[test]
    fn an_os_release_with_no_id_at_all_answers_nothing() {
        assert_eq!(parse_os_release_family("PRETTY_NAME=\"mystery\"\n"), None);
    }
}

#[cfg(test)]
mod removal_target_tests {
    use super::split_removal_target;

    // A tiny fixed backend set for the tests.
    fn known(b: &str) -> bool {
        matches!(b, "apt" | "uv" | "npm" | "web" | "cargo")
    }

    #[test]
    fn backend_prefix_scopes_and_strips_to_bare_name() {
        assert_eq!(
            split_removal_target("uv:ruff", known),
            (Some("uv".to_string()), "ruff".to_string())
        );
        assert_eq!(
            split_removal_target("apt:tree", known),
            (Some("apt".to_string()), "tree".to_string())
        );
    }

    #[test]
    fn bare_name_has_no_backend() {
        assert_eq!(
            split_removal_target("ripgrep", known),
            (None, "ripgrep".to_string())
        );
    }

    #[test]
    fn options_are_stripped_from_the_name() {
        assert_eq!(
            split_removal_target("npm:typescript@version=5", known),
            (Some("npm".to_string()), "typescript".to_string())
        );
    }

    #[test]
    fn unknown_prefix_is_not_treated_as_a_backend() {
        // A colon in a name whose prefix isn't a real backend stays part of the name.
        assert_eq!(
            split_removal_target("some:weird-name", known),
            (None, "some:weird-name".to_string())
        );
    }

    #[test]
    fn web_url_name_keeps_its_scheme_colon() {
        // web:https://x -> backend web, name "https://x" (only the first colon is the split).
        assert_eq!(
            split_removal_target("web:https://example.com/a.tar.gz", known),
            (
                Some("web".to_string()),
                "https://example.com/a.tar.gz".to_string()
            )
        );
    }
}
