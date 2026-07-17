use super::error::{GrammarError, Origin, Result};
use super::options::{parse_short, Options};

/// `re` is reserved: `apt:re:^fonts-` must always mean a regex, so a custom backend named
/// `re` (which the onboarder would otherwise happily accept) would make `re:foo`
/// ambiguous forever.
pub const RESERVED_BACKEND_NAMES: &[&str] = &["re"];

/// What a package line selects inside its backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    Name(String),
    /// `BACKEND:re:PATTERN` — matches names in that backend. Live by default; frozen only
    /// when `locks/` holds an entry for it (II.15).
    Regex(String),
}

impl Selector {
    pub fn as_str(&self) -> &str {
        match self {
            Selector::Name(n) | Selector::Regex(n) => n,
        }
    }
}

/// A package declaration: the backend (or none, meaning "resolve via `priority`"), what it
/// selects, and its options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageDecl {
    /// `None` for a bare name. II.7 resolves it via `priority`, then locks the answer —
    /// the bare name is the question, the lock is the answer (V.16).
    pub backend: Option<String>,
    pub selector: Selector,
    pub options: Options,
}

/// A reference to a module (lowercase) or a profile (Capitalized). Case is what tells them
/// apart, so `(Work | gaming) & security` reads without extra syntax (II.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reference {
    Module(String),
    Profile(String),
}

impl Reference {
    /// Classify by the first character's case. A name starting with neither (a digit,
    /// `_`) is rejected by the caller rather than guessed at.
    pub fn classify(name: &str) -> Option<Self> {
        let first = name.chars().next()?;
        if first.is_uppercase() {
            Some(Reference::Profile(name.to_string()))
        } else if first.is_lowercase() {
            Some(Reference::Module(name.to_string()))
        } else {
            None
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Reference::Module(n) | Reference::Profile(n) => n,
        }
    }
}

/// One statement. II.2's full list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement {
    Package(PackageDecl),
    /// `absent:BACKEND:NAME` — declare it must not exist. The one thing LiNix may remove
    /// that it does not manage, because you named it (V.7).
    Absent(PackageDecl),
    Repo(String),
    Shim(String, Options),
    Schedule(String, Options),
    Service(String, Options),
    Link(String, Options),
    Use(Reference),
    /// `exclude heavy` — subtract that module's or profile's packages (II.4).
    Exclude(Reference),
    /// `intersect security` — keep only packages that are also in it (II.4).
    Intersect(Reference),
    /// `-vim` — subtract one package (II.4).
    ///
    /// Not an infix operator: real package names contain `-` (`g++` aside, `fonts-noto`
    /// does), so `a - b` cannot be told from a package called `a - b` without quoting, and
    /// there are no quotes (V.10).
    Subtract(String),
    /// `(Work | gaming) & security` — a set expression over modules and profiles (II.4).
    Expr(String),
}

/// Decides whether a `prefix:` names a real backend. Injected rather than hardcoded: the
/// answer is host-dependent (there is no `winget` on Linux) and the onboarder can add
/// backends at runtime, so a static list would be a second copy of a fact the registry
/// already owns (P4).
pub trait BackendNames {
    fn is_backend(&self, name: &str) -> bool;
}

impl<F: Fn(&str) -> bool> BackendNames for F {
    fn is_backend(&self, name: &str) -> bool {
        self(name)
    }
}

/// Every keyword that introduces a statement, for the "unrecognised line" error.
const KNOWN_PREFIXES: &[&str] = &[
    "absent:",
    "repo:",
    "shim:",
    "schedule:",
    "service:",
    "link:",
];

/// Parse one statement. `line` must already have had comments stripped and be non-blank.
///
/// This is the only `backend:name` parser. Eight existed before, six of which never
/// checked that the prefix named a real backend — so every new prefix (`absent:`, `re:`,
/// `repo:`) was a thing they silently read as a backend name (C13).
pub fn parse(origin: &Origin, line: &str, backends: &dyn BackendNames) -> Result<Statement> {
    let line = line.trim();

    if let Some(rest) = line.strip_prefix("use ") {
        return parse_use(origin, rest.trim());
    }
    if line == "use" || line.starts_with("use\t") {
        return parse_use(origin, line[3..].trim());
    }

    // II.4's set directives. Checked before the package parser, which would otherwise read
    // `exclude heavy` as a package named `exclude heavy`.
    for word in ["exclude ", "intersect "] {
        if let Some(rest) = line.strip_prefix(word) {
            return parse_set_directive(origin, word.trim(), rest.trim());
        }
    }

    // V.44: `use` already means union, so a second word for it is two ways to do one thing.
    if let Some(rest) = line.strip_prefix("include ") {
        return Err(GrammarError::new(
            origin.clone(),
            format!("`include {}` — there is no `include`", rest.trim()),
        )
        .with_hint(format!(
            "write `use {}`. One word brings something in, everywhere: modules use it too.",
            rest.trim()
        )));
    }

    // A set expression, before the package parser reads `(Work` as a package name.
    if crate::app::profile_expr::looks_like_expression(line) {
        return Ok(Statement::Expr(line.to_string()));
    }

    // `-vim`. Checked after expressions so `a \ b` is a difference, not a subtraction.
    if let Some(rest) = line.strip_prefix('-') {
        let target = rest.trim();
        if target.is_empty() {
            return Err(GrammarError::new(origin.clone(), "`-` subtracts nothing")
                .with_hint("write `-vim` to take one package out."));
        }
        return Ok(Statement::Subtract(target.to_string()));
    }

    if let Some(rest) = line.strip_prefix("absent:") {
        let decl = parse_package(origin, rest.trim(), backends)?;
        if decl.backend.is_none() {
            return Err(GrammarError::new(
                origin.clone(),
                format!("`absent:{}` does not name a backend", decl.selector.as_str()),
            )
            .with_hint(
                "an `absent:` line reaches outside what LiNix manages, so it must say which \
                 backend: `absent:apt:libreoffice`.",
            ));
        }
        return Ok(Statement::Absent(decl));
    }

    if let Some(rest) = line.strip_prefix("repo:") {
        let spec = rest.trim();
        if spec.is_empty() {
            return Err(GrammarError::new(origin.clone(), "`repo:` names no repository"));
        }
        return Ok(Statement::Repo(spec.to_string()));
    }

    for (prefix, build) in [
        ("shim:", Statement::Shim as fn(String, Options) -> Statement),
        ("schedule:", Statement::Schedule),
        ("service:", Statement::Service),
        ("link:", Statement::Link),
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let (name, options) = split_options(origin, rest.trim())?;
            if name.is_empty() {
                return Err(GrammarError::new(
                    origin.clone(),
                    format!("`{}` names nothing", prefix),
                ));
            }
            return Ok(build(name, options));
        }
    }

    let decl = parse_package(origin, line, backends)?;
    Ok(Statement::Package(decl))
}

fn parse_use(origin: &Origin, target: &str) -> Result<Statement> {
    if target.is_empty() {
        return Err(GrammarError::new(origin.clone(), "`use` names nothing")
            .with_hint("write `use editors` (a module) or `use Work` (a profile)."));
    }
    // `use` takes a name, never a path and never a URL (II.2). A file from the internet is
    // a fetch step that puts a module on disk; then you `use` it by name like everything
    // else.
    if target.contains('/') || target.contains('\\') || target.contains("://") {
        return Err(GrammarError::new(
            origin.clone(),
            format!("`use {}` looks like a path or a URL", target),
        )
        .with_hint(
            "`use` takes a name. Fetch the file into `modules/` first, then `use` it by name.",
        ));
    }
    if target.split_whitespace().count() > 1 {
        return Err(GrammarError::new(
            origin.clone(),
            format!("`use {}` names more than one thing", target),
        )
        .with_hint("one `use` per line."));
    }
    Reference::classify(target).map(Statement::Use).ok_or_else(|| {
        GrammarError::new(
            origin.clone(),
            format!("`{}` is neither a module nor a profile name", target),
        )
        .with_hint("profiles are Capitalized, modules are lowercase.")
    })
}

/// `exclude heavy` / `intersect security` — both take one module or profile name, and case
/// says which, exactly as `use` does.
fn parse_set_directive(origin: &Origin, word: &str, target: &str) -> Result<Statement> {
    if target.is_empty() {
        return Err(GrammarError::new(
            origin.clone(),
            format!("`{}` names nothing", word),
        )
        .with_hint(format!("write `{} heavy` (a module) or `{} Work` (a profile).", word, word)));
    }
    if target.split_whitespace().count() > 1 {
        return Err(GrammarError::new(
            origin.clone(),
            format!("`{} {}` names more than one thing", word, target),
        )
        .with_hint(format!("one `{}` per line.", word)));
    }
    let reference = Reference::classify(target).ok_or_else(|| {
        GrammarError::new(
            origin.clone(),
            format!("`{}` is neither a module nor a profile name", target),
        )
        .with_hint("profiles are Capitalized, modules are lowercase.")
    })?;
    Ok(match word {
        "exclude" => Statement::Exclude(reference),
        _ => Statement::Intersect(reference),
    })
}

/// Split `NAME@opts` into its name and options. Used by the non-package statements, whose
/// names are not `backend:name` (`shim:jq@source=cargo:jq`).
fn split_options(origin: &Origin, text: &str) -> Result<(String, Options)> {
    match text.split_once('@') {
        Some((name, opts)) => Ok((name.trim().to_string(), parse_short(origin, opts)?)),
        None => Ok((text.to_string(), Options::default())),
    }
}

fn parse_package(origin: &Origin, text: &str, backends: &dyn BackendNames) -> Result<PackageDecl> {
    let (head, options) = match text.split_once('@') {
        Some((head, opts)) => (head.trim(), parse_short(origin, opts)?),
        None => (text, Options::default()),
    };

    if head.is_empty() {
        return Err(GrammarError::new(origin.clone(), "no package name"));
    }

    // Checked before the backend split so `re:^fonts-` gets the error that says what is
    // missing, rather than "`re` is not a backend" — which is true but useless.
    if let Some(pattern) = head.strip_prefix("re:") {
        return Err(GrammarError::new(
            origin.clone(),
            format!("`re:{}` does not say which backend to match in", pattern.trim()),
        )
        .with_hint("write `apt:re:^fonts-`. A pattern has to be matched somewhere."));
    }

    let (backend, rest) = match head.split_once(':') {
        Some((prefix, rest)) if backends.is_backend(prefix) => {
            (Some(prefix.to_string()), rest.trim())
        }
        // A colon that is not a known backend. Guessing it is a backend is C13; guessing it
        // is part of the name silently accepts `snap:foo` on a host with no snap. Refuse.
        Some((prefix, _)) => {
            return Err(GrammarError::new(
                origin.clone(),
                format!("`{}` is not a backend LiNix uses", prefix),
            )
            .with_hint(format!(
                "add `{}` to your `priority` file, or check the spelling. Not listed means \
                 LiNix does not use it at all.",
                prefix
            )));
        }
        None => (None, head),
    };

    let selector = match rest.strip_prefix("re:") {
        Some(pattern) => {
            let pattern = pattern.trim();
            if pattern.is_empty() {
                return Err(GrammarError::new(origin.clone(), "`re:` has no pattern"));
            }
            Selector::Regex(pattern.to_string())
        }
        None => {
            if rest.is_empty() {
                return Err(GrammarError::new(origin.clone(), "no package name after the backend"));
            }
            // A package name is one word. Without this, any unrecognised prose becomes a
            // package literally named after itself — VI.1's "any typo becomes a package
            // name", which is what II.2's "an unrecognised line is an error" forbids.
            if rest.split_whitespace().count() > 1 {
                return Err(GrammarError::new(
                    origin.clone(),
                    format!("`{}` is not a package name", rest),
                ));
            }
            Selector::Name(rest.to_string())
        }
    };

    let decl = PackageDecl {
        backend,
        selector,
        options,
    };
    validate_options(origin, &decl)?;
    Ok(decl)
}

/// Option rules from II.2's table that are about the options themselves rather than any
/// one backend.
fn validate_options(origin: &Origin, decl: &PackageDecl) -> Result<()> {
    let o = &decl.options;

    // `@hold` says "never upgrade this"; `@version=` says "this exact version". Together
    // they are a contradiction, not a refinement: hold means whatever is installed, and
    // version means something specific that may not be it.
    if o.contains("hold") && o.contains("version") {
        return Err(GrammarError::new(
            origin.clone(),
            "`@hold` and `@version=` contradict each other",
        )
        .with_hint(
            "`@hold` keeps whatever is installed; `@version=` pins a specific one. Pick one.",
        ));
    }

    // `requires` is install ORDERING for things outside a package manager's own dependency
    // graph (V.29). A bare name would have to be resolved via `priority`, and the whole
    // point is that these are things with no one to ask.
    for req in o.all("requires") {
        if !req.contains(':') {
            return Err(GrammarError::new(
                origin.clone(),
                format!("`requires = {}` is a bare name", req),
            )
            .with_hint("`requires` needs a backend: `requires = apt:libfoo`."));
        }
    }

    // II.2: `expires` and `until` are absolute datetimes. A duration cannot work in a file
    // — the machine reading it next week has no idea when you wrote it (V.38), which is
    // exactly why `@lease=2h` was inert.
    for key in ["expires", "until"] {
        if let Some(v) = o.one(key) {
            if !is_absolute_datetime(v) {
                return Err(GrammarError::new(
                    origin.clone(),
                    format!("`@{}={}` is not an absolute date and time", key, v),
                )
                .with_hint(
                    "write it out in full: `@expires=2026-07-17T14:00`. A duration cannot \
                     work in a file — whoever reads it later has no idea when you wrote it.",
                ));
            }
        }
    }

    // `until` is the mirror of `expires` and only makes sense on `absent:` (absent now,
    // present after). On a present line it would mean "install this later", which the
    // grammar has no way to act on.
    Ok(())
}

/// Accepts RFC3339 and the `YYYY-MM-DDTHH:MM` form II.2 uses in its example. Rejects
/// anything that reads as a duration.
fn is_absolute_datetime(v: &str) -> bool {
    if chrono::DateTime::parse_from_rfc3339(v).is_ok() {
        return true;
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d"] {
        if chrono::NaiveDateTime::parse_from_str(v, fmt).is_ok()
            || chrono::NaiveDate::parse_from_str(v, fmt).is_ok()
        {
            return true;
        }
    }
    false
}

/// The `absent:`-style prefixes, for building an "unrecognised line" message that lists
/// what was expected.
pub fn known_prefixes() -> &'static [&'static str] {
    KNOWN_PREFIXES
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o() -> Origin {
        Origin::new("modules/dev.txt", 7)
    }

    /// Stands in for the live BackendRegistry.
    fn known(name: &str) -> bool {
        matches!(name, "apt" | "cargo" | "snap" | "npm")
    }

    fn p(line: &str) -> Result<Statement> {
        parse(&o(), line, &known)
    }

    #[test]
    fn a_bare_name_has_no_backend() {
        let Statement::Package(d) = p("ripgrep").unwrap() else {
            panic!()
        };
        assert_eq!(d.backend, None);
        assert_eq!(d.selector, Selector::Name("ripgrep".into()));
    }

    #[test]
    fn an_explicit_backend_is_read() {
        let Statement::Package(d) = p("apt:curl").unwrap() else {
            panic!()
        };
        assert_eq!(d.backend.as_deref(), Some("apt"));
        assert_eq!(d.selector, Selector::Name("curl".into()));
    }

    #[test]
    fn an_unknown_backend_prefix_is_an_error_not_a_package_name() {
        // C13. Six of the eight old parsers did `split_once(':')` and trusted the prefix,
        // so a typo became a backend and every new prefix was read as one.
        let err = p("aptt:curl").unwrap_err();
        assert!(err.what.contains("not a backend"), "{}", err.what);
        assert!(err.hint.unwrap().contains("priority"));
    }

    #[test]
    fn a_backend_not_in_priority_says_so() {
        // V.15: not listed means LiNix does not use it at all, and saying so catches typos.
        let err = parse(&o(), "flatpak:gimp", &known).unwrap_err();
        assert!(err.what.contains("flatpak"), "{}", err.what);
    }

    #[test]
    fn a_regex_selects_by_pattern() {
        let Statement::Package(d) = p("apt:re:^fonts-").unwrap() else {
            panic!()
        };
        assert_eq!(d.backend.as_deref(), Some("apt"));
        assert_eq!(d.selector, Selector::Regex("^fonts-".into()));
    }

    #[test]
    fn a_regex_must_say_which_backend() {
        let err = p("re:^fonts-").unwrap_err();
        assert!(err.what.contains("does not say which backend"), "{}", err.what);
    }

    #[test]
    fn re_is_reserved_against_a_custom_backend() {
        assert!(RESERVED_BACKEND_NAMES.contains(&"re"));
    }

    #[test]
    fn absent_declares_a_package_must_not_exist() {
        let Statement::Absent(d) = p("absent:apt:libreoffice").unwrap() else {
            panic!()
        };
        assert_eq!(d.backend.as_deref(), Some("apt"));
        assert_eq!(d.selector.as_str(), "libreoffice");
    }

    #[test]
    fn absent_must_name_a_backend() {
        // `absent:` reaches outside what LiNix manages, so it cannot be left to `priority`.
        let err = p("absent:libreoffice").unwrap_err();
        assert!(err.what.contains("does not name a backend"), "{}", err.what);
    }

    #[test]
    fn hold_and_version_together_are_a_contradiction() {
        let err = p("apt:jq@hold,version=1.6").unwrap_err();
        assert!(err.what.contains("contradict"), "{}", err.what);
    }

    #[test]
    fn a_bare_requires_is_an_error() {
        let err = p("apt:nginx@requires=libfoo").unwrap_err();
        assert!(err.what.contains("bare name"), "{}", err.what);
        assert!(err.hint.unwrap().contains("apt:libfoo"));
    }

    #[test]
    fn a_qualified_requires_is_accepted() {
        assert!(p("apt:nginx@requires=apt:libfoo").is_ok());
    }

    #[test]
    fn a_relative_expiry_is_an_error() {
        // V.38: "2 hours" cannot work in a file — this is why `@lease=2h` was inert.
        let err = p("apt:jq@expires=2h").unwrap_err();
        assert!(err.what.contains("not an absolute date"), "{}", err.what);
        assert!(err.hint.unwrap().contains("2026-07-17T14:00"));
    }

    #[test]
    fn an_absolute_expiry_is_accepted() {
        assert!(p("apt:jq@expires=2026-07-17T14:00").is_ok());
        assert!(p("apt:jq@expires=2026-07-17T14:00:00Z").is_ok());
    }

    #[test]
    fn use_takes_a_module_by_lowercase_name() {
        assert_eq!(
            p("use editors").unwrap(),
            Statement::Use(Reference::Module("editors".into()))
        );
    }

    #[test]
    fn use_takes_a_profile_by_capitalized_name() {
        assert_eq!(
            p("use Work").unwrap(),
            Statement::Use(Reference::Profile("Work".into()))
        );
    }

    #[test]
    fn use_never_takes_a_path_or_a_url() {
        // II.2. A file from the internet is a fetch step that puts a module on disk; then
        // you `use` it by name like everything else.
        for bad in [
            "use ./base.txt",
            "use /etc/linix/base.txt",
            "use https://x/y.txt",
        ] {
            let err = p(bad).unwrap_err();
            assert!(err.hint.unwrap().contains("takes a name"), "{}", bad);
        }
    }

    #[test]
    fn repo_and_the_package_needing_it_are_both_statements() {
        assert_eq!(
            p("repo:ppa:deadsnakes/ppa").unwrap(),
            Statement::Repo("ppa:deadsnakes/ppa".into())
        );
    }

    #[test]
    fn shim_carries_its_source() {
        let Statement::Shim(name, opts) = p("shim:jq@source=cargo:jq").unwrap() else {
            panic!()
        };
        assert_eq!(name, "jq");
        assert_eq!(opts.one("source"), Some("cargo:jq"));
    }

    #[test]
    fn every_error_names_the_file_and_line() {
        let err = p("aptt:curl").unwrap_err();
        assert!(err.to_string().contains("modules/dev.txt:7"), "{}", err);
    }
}
