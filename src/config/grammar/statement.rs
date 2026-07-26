use super::error::{GrammarError, Origin, Result};
use super::options::{parse_short, Options};

/// `re` is reserved: `apt:re:^fonts-` must always mean a regex, so a custom backend named
/// `re` (which the onboarder would otherwise happily accept) would make `re:foo`
/// ambiguous forever. `list` is reserved for the same reason — it names the `priority` file
/// inside a backend chain (`apt,list:rg`), so a backend called `list` would make that
/// unreadable.
pub const RESERVED_BACKEND_NAMES: &[&str] = &["re", "list"];

/// The word that means "the `priority` file" where a backend name is expected.
pub const PRIORITY_KEYWORD: &str = "list";

/// Which managers a line will accept, when it has not pinned exactly one.
///
/// A pin (`apt:rg`) says apt or nothing — carried in `PackageDecl::backend`, so this only
/// describes the unpinned case. Separating the two is what lets `apt:rg` keep meaning apt on
/// a machine that also has dnf, while `apt,dnf:rg` and a bare `rg` stay installable on a
/// machine that has neither apt nor the manager some other machine froze the name to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Candidates {
    /// A bare name (`rg`), spelled explicitly as `list:rg`: every manager in `priority`, in
    /// that order.
    #[default]
    Priority,
    /// `apt,dnf:rg` — these, in this order, and nothing else.
    Named(Vec<String>),
    /// `apt,list:rg` — these first, then the rest of `priority` in its own order.
    NamedThenPriority(Vec<String>),
}

impl Candidates {
    /// The managers to ask, in order. `priority` supplies the tail for the two variants that
    /// end in `list`; a name already asked for is not asked twice.
    pub fn order(&self, priority: &[String]) -> Vec<String> {
        let (head, tail): (&[String], &[String]) = match self {
            Candidates::Priority => (&[], priority),
            Candidates::Named(names) => (names, &[]),
            Candidates::NamedThenPriority(names) => (names, priority),
        };
        let mut out: Vec<String> = head.to_vec();
        for name in tail {
            if !out.contains(name) {
                out.push(name.clone());
            }
        }
        out
    }

    /// Whether this line would accept `backend`. A lock naming a manager the line no longer
    /// lists is not an answer to the question the line is now asking.
    pub fn accepts(&self, backend: &str, priority: &[String]) -> bool {
        self.order(priority).iter().any(|b| b == backend)
    }
}

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
    /// `Some` only when the line pinned exactly one manager (`apt:rg`). II.7 resolves the
    /// rest through `candidates`, then locks the answer — the unpinned name is the question,
    /// the lock is the answer (V.16).
    pub backend: Option<String>,
    /// Which managers may answer, when `backend` is `None`. Ignored when it is `Some`.
    pub candidates: Candidates,
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

/// One parsed line. Covers every statement kind the grammar accepts: II.2's declarations
/// and typed lines (`Package`, `Absent`, `Repo`, `Shim`, `Schedule`, `Service`, `Link`,
/// `Use`) **and** II.4's set operations (`Exclude`, `Intersect`, `Subtract`, `Expr`) — the
/// latter belong to the set-math grammar, not II.2's statement table, so this is not "II.2's
/// full list" but the union of the two grammars a module line can be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement {
    Package(PackageDecl),
    /// `absent:BACKEND:NAME` — declare it must not exist. The one thing LiNix may remove
    /// that it does not manage, because you named it (V.7).
    Absent(PackageDecl),
    /// `repo:BACKEND:SPEC` — a repository, for a named backend (V.47). A PPA is apt's, a
    /// COPR dnf's; guessing the backend runs the wrong system command, so it is named.
    Repo { backend: String, spec: String },
    Shim(String, Options),
    Schedule(String, Options),
    Service(String, Options),
    Link(String, Options),
    /// `setting:SCHEMA/KEY @value=…` — a desktop setting whose home is a settings store
    /// rather than a file (X.4). GNOME and KDE keep configuration in dconf and kconfig, so
    /// `link:` cannot reach it; the adapter is chosen by what is running, not by what was
    /// typed, which is why this is a statement and not a backend.
    Setting(String, Options),
    /// `exec:PATH @runs=N` — run a script the config carries (XIII.3). A *verb*, not a noun:
    /// its `when` decides whether the machine wants it, and `locks/exec.toml` (keyed by the
    /// script's content hash) decides whether it already happened. Unlike every other
    /// statement, a false `when` does not mean "undo" — a script that succeeds makes its own
    /// condition false, so treating false as removal would flap. See the three-state table in
    /// XIII.3. The script goes through II.12's approval ledger like any other code the repo
    /// runs ("hash everything, no exceptions").
    Exec(String, Options),
    /// `dotfiles:PATH` — a folder mirrored into place, one file at a time (XIII.21).
    ///
    /// Every other statement names one thing; this names a tree and stands for as many
    /// declarations as it holds. It links **files**, never directories (U22): a symlinked
    /// directory takes everything the application later writes there into the git-tracked
    /// repo, and `bundle` then hands it to whoever the backup goes to.
    Dotfiles(String, Options),
    /// `firewall:22/tcp`, `firewall:default/incoming @value=deny` — a declared perimeter
    /// (Part XI). One spelling across ufw, firewalld and Windows Defender, which is the whole
    /// argument for a built-in backend rather than a per-machine `[[backend]]` naming `ufw`.
    Firewall(String, Options),
    /// `use editors` / `use workstation(user=shaul, gpu=nvidia)` — bring in a module or profile
    /// (II.2), optionally with **arguments** binding that module's `param`s (U32). The args are
    /// empty for the ordinary form; a profile referenced with args is refused at parse time,
    /// because a profile has no parameters to bind.
    Use(Reference, Vec<(String, String)>),
    /// `param NAME` / `param NAME = DEFAULT` — a module parameter (U32). A `param` with no
    /// default is required: a `use` that omits it is a loud error naming the module and the
    /// parameter, never an empty string that makes a `when` silently false (V.78). Legal only in
    /// a module; parsed here so there is one parser, and rejected by file context like `schedule:`.
    Param { name: String, default: Option<String> },
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
    /// `NAME = VALUE` — a variable (IX.2). Legal only in the `vars` file; parsed here so
    /// there is one parser, and rejected by file context the way `schedule:` is.
    Var { name: String, value: String },
}

impl Statement {
    /// How this statement is named: `service:nginx`, `apt:jq`, `use work`, `-vim`.
    ///
    /// **One spelling, because it had three.** Set math keyed statements one way, `edit`'s
    /// line matcher a second, the teardown ledger a third — three lists of the same twelve
    /// variants, each of which had to be extended whenever a statement kind was added, and
    /// none of which the compiler could check against the others. A statement's identity is a
    /// property of the statement, so it lives on the statement.
    ///
    /// Written form, not resolved form: a bare `jq` keys as `jq`, because set math runs while
    /// the files are being read and nothing has probed a backend yet.
    pub fn key(&self) -> String {
        match self {
            Statement::Package(d) | Statement::Absent(d) => match &d.backend {
                Some(b) => format!("{}:{}", b, d.selector.as_str()),
                None => d.selector.as_str().to_string(),
            },
            Statement::Repo { backend, spec } => format!("repo:{}:{}", backend, spec),
            Statement::Shim(n, _) => format!("shim:{}", n),
            Statement::Schedule(n, _) => format!("schedule:{}", n),
            Statement::Service(n, _) => format!("service:{}", n),
            Statement::Link(n, _) => format!("link:{}", n),
            Statement::Setting(n, _) => format!("setting:{}", n),
            Statement::Exec(n, _) => format!("exec:{}", n),
            Statement::Dotfiles(n, _) => format!("dotfiles:{}", n),
            Statement::Firewall(n, _) => format!("firewall:{}", n),
            Statement::Use(r, _) => format!("use {}", r.name()),
            Statement::Param { name, .. } => format!("param {}", name),
            Statement::Exclude(r) => format!("exclude {}", r.name()),
            Statement::Intersect(r) => format!("intersect {}", r.name()),
            Statement::Subtract(p) => format!("-{}", p),
            Statement::Var { name, .. } => format!("{} =", name),
            Statement::Expr(e) => e.clone(),
        }
    }

    /// The keyword that introduces this statement — `service`, `link`, `firewall` — for the
    /// kinds that have one.
    ///
    /// `None` for a package line (whose prefix is a *backend*, not a keyword) and for set math
    /// (an operation, not a thing). A caller that wants to group or filter by kind asks here
    /// rather than re-splitting [`key`](Self::key) on `:`, which would read `apt:jq` as the
    /// kind `apt`.
    pub fn kind(&self) -> Option<&'static str> {
        Some(match self {
            Statement::Repo { .. } => "repo",
            Statement::Shim(..) => "shim",
            Statement::Schedule(..) => "schedule",
            Statement::Service(..) => "service",
            Statement::Link(..) => "link",
            Statement::Setting(..) => "setting",
            Statement::Exec(..) => "exec",
            Statement::Dotfiles(..) => "dotfiles",
            Statement::Firewall(..) => "firewall",
            Statement::Package(_)
            | Statement::Absent(_)
            | Statement::Use(..)
            | Statement::Param { .. }
            | Statement::Exclude(_)
            | Statement::Intersect(_)
            | Statement::Subtract(_)
            | Statement::Expr(_)
            | Statement::Var { .. } => return None,
        })
    }

    /// What this statement names, without its keyword: `nginx` for `service:nginx`.
    ///
    /// The `key` minus the `kind`, so the two can never disagree about where the boundary is.
    pub fn subject(&self) -> Option<String> {
        let kind = self.kind()?;
        let key = self.key();
        Some(key.strip_prefix(kind)?.trim_start_matches(':').to_string())
    }
}

/// Decides whether a `prefix:` names a real backend. Injected rather than hardcoded: the
/// answer is host-dependent (there is no `winget` on Linux) and the onboarder can add
/// backends at runtime, so a static list would be a second copy of a fact the registry
/// already owns (P4).
pub trait BackendNames {
    fn is_backend(&self, name: &str) -> bool;

    /// The backends a group name expands to (U18), or `None` when it is not a group. A group is
    /// a shorthand for a comma-chain, so expansion happens here, in the one parser, and the
    /// expanded members go through the same backend check every chain part does.
    ///
    /// Default: nothing is a group. The paths with no `groups` file — and every test that only
    /// cares about backends — keep working unchanged.
    fn expand_group(&self, _name: &str) -> Option<Vec<String>> {
        None
    }
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
    let stmt = parse_inner(origin, line, backends)?;
    validate(origin, &stmt)?;
    Ok(stmt)
}

fn parse_inner(origin: &Origin, line: &str, backends: &dyn BackendNames) -> Result<Statement> {
    let line = line.trim();

    if let Some(rest) = line.strip_prefix("use ") {
        return parse_use(origin, rest.trim());
    }
    if line == "use" || line.starts_with("use\t") {
        return parse_use(origin, line[3..].trim());
    }

    // `param NAME` / `param NAME = DEFAULT` (U32). Checked before the package parser so a bare
    // `param gpu` is a parameter declaration, not a package named `param gpu`.
    if let Some(rest) = line.strip_prefix("param ") {
        return parse_param(origin, rest.trim());
    }
    if line == "param" || line.starts_with("param\t") {
        return parse_param(origin, line[5..].trim());
    }

    // II.4's set directives. Checked before the package parser, which would otherwise read
    // `exclude heavy` as a package named `exclude heavy`.
    for word in ["exclude ", "intersect "] {
        if let Some(rest) = line.strip_prefix(word) {
            return parse_set_directive(origin, word.trim(), rest.trim());
        }
    }

    // V.46: `use` already means union, so a second word for it is two ways to do one thing.
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

    // A set expression, before the package parser reads `(Work` as a package name — but NOT
    // before the typed statements. `looks_like_expression` fires on `\ | & (`, and a
    // `link:C:\Users\me\.vimrc` is full of `\`: without this guard II.4's set math eats
    // II.2's statements, and `link:` silently parses as `Statement::Expr`. A line that opens
    // with a known statement prefix is that statement, never an expression.
    if !starts_with_statement_prefix(line)
        && crate::app::profile_expr::looks_like_expression(line)
    {
        return Ok(Statement::Expr(line.to_string()));
    }

    // `-vim`. Checked after expressions so `a \ b` is a difference, not a subtraction.
    if let Some(rest) = line.strip_prefix('-') {
        let target = rest.trim();
        if target.is_empty() {
            return Err(GrammarError::new(origin.clone(), "`-` subtracts nothing")
                .with_hint("write `-vim` to take one package out."));
        }
        reject_leading_dash(origin, target)?;
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
        let rest = rest.trim();
        // `repo:apt:ppa:deadsnakes/ppa` — backend, then the spec (which has its own colons).
        let Some((backend, spec)) = rest.split_once(':') else {
            return Err(GrammarError::new(
                origin.clone(),
                format!("`repo:{}` does not name a backend", rest),
            )
            .with_hint(
                "a repository belongs to one package manager, so name it: \
                 `repo:apt:ppa:deadsnakes/ppa`. A PPA is apt's, a COPR is dnf's.",
            ));
        };
        let (backend, spec) = (backend.trim(), spec.trim());
        if backend.is_empty() || spec.is_empty() {
            return Err(GrammarError::new(origin.clone(), "`repo:` needs `backend:spec`")
                .with_hint("for example `repo:apt:ppa:deadsnakes/ppa`."));
        }
        if !backends.is_backend(backend) {
            return Err(GrammarError::new(
                origin.clone(),
                format!("`{}` is not a backend", backend),
            )
            .with_hint("name the package manager that owns this repository, e.g. `apt`."));
        }
        reject_leading_dash(origin, spec)?;
        return Ok(Statement::Repo {
            backend: backend.to_string(),
            spec: spec.to_string(),
        });
    }

    for (prefix, build) in [
        ("shim:", Statement::Shim as fn(String, Options) -> Statement),
        ("schedule:", Statement::Schedule),
        ("service:", Statement::Service),
        ("link:", Statement::Link),
        ("setting:", Statement::Setting),
        ("exec:", Statement::Exec),
        ("dotfiles:", Statement::Dotfiles),
        ("firewall:", Statement::Firewall),
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let (name, options) = split_options(origin, rest.trim())?;
            if name.is_empty() {
                return Err(GrammarError::new(
                    origin.clone(),
                    format!("`{}` names nothing", prefix),
                ));
            }
            reject_leading_dash(origin, &name)?;
            return Ok(build(name, options));
        }
    }

    if let Some(var) = parse_var(line) {
        return Ok(var);
    }

    let decl = parse_package(origin, line, backends)?;
    Ok(Statement::Package(decl))
}

/// `NAME = VALUE` (IX.2), where NAME is an identifier.
///
/// Checked last, and only for a bare identifier before the `=`, so nothing that is already a
/// package line can be read as a variable: `apt:foo@version=1.2` has a `:` and an `@` in its
/// head, and `-vim` does not start with a name character.
fn parse_var(line: &str) -> Option<Statement> {
    let (head, value) = line.split_once('=')?;
    let name = head.trim();
    if name.is_empty() || !name.starts_with(|c: char| c.is_alphabetic() || c == '_') {
        return None;
    }
    if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    Some(Statement::Var {
        name: name.to_string(),
        // Verbatim to end of line, trimmed — the same rule as a block-form option value.
        value: value.trim().to_string(),
    })
}

/// Whether a line opens with one of II.2's typed-statement prefixes. Such a line is that
/// statement and must not be mistaken for a set expression (II.4), whatever punctuation its
/// payload carries — a `link:` target is a path, not a difference.
fn starts_with_statement_prefix(line: &str) -> bool {
    [
        "absent:", "repo:", "shim:", "schedule:", "service:", "link:", "exec:", "dotfiles:",
        "firewall:",
    ]
        .iter()
        .any(|p| line.starts_with(p))
}

fn parse_use(origin: &Origin, target: &str) -> Result<Statement> {
    if target.is_empty() {
        return Err(GrammarError::new(origin.clone(), "`use` names nothing")
            .with_hint("write `use editors` (a module) or `use Work` (a profile)."));
    }

    // Split off an optional `(args)` before validating the name, so a `/` inside an argument
    // value (`use m(path=/etc/foo)`) is not mistaken for the path in a `use` target (U32).
    let (name, args) = match target.split_once('(') {
        Some((name, rest)) => {
            let inner = rest.strip_suffix(')').ok_or_else(|| {
                GrammarError::new(
                    origin.clone(),
                    format!("`use {}` opens `(` but never closes it", target),
                )
                .with_hint("write `use workstation(user=shaul, gpu=nvidia)`.")
            })?;
            (name.trim(), parse_use_args(origin, inner)?)
        }
        None => (target, Vec::new()),
    };

    // `use` takes a name, never a path and never a URL (II.2). A file from the internet is
    // a fetch step that puts a module on disk; then you `use` it by name like everything
    // else.
    if name.contains('/') || name.contains('\\') || name.contains("://") {
        return Err(GrammarError::new(
            origin.clone(),
            format!("`use {}` looks like a path or a URL", name),
        )
        .with_hint(
            "`use` takes a name. Fetch the file into `modules/` first, then `use` it by name.",
        ));
    }
    if name.split_whitespace().count() > 1 {
        return Err(GrammarError::new(
            origin.clone(),
            format!("`use {}` names more than one thing", name),
        )
        .with_hint("one `use` per line."));
    }
    let reference = Reference::classify(name).ok_or_else(|| {
        GrammarError::new(
            origin.clone(),
            format!("`{}` is neither a module nor a profile name", name),
        )
        .with_hint("profiles are Capitalized, modules are lowercase.")
    })?;
    // A profile has no parameters to bind (U32): only modules declare `param`.
    if !args.is_empty() && matches!(reference, Reference::Profile(_)) {
        return Err(GrammarError::new(
            origin.clone(),
            format!("`use {}` passes arguments to a profile", name),
        )
        .with_hint("only a module takes parameters (`param`); a profile has none to bind."));
    }
    Ok(Statement::Use(reference, args))
}

/// Parse `k=v, k2=v2` from inside a `use name(...)`. Values are verbatim to the next comma,
/// trimmed. An empty argument list (`use m()`) is allowed and binds nothing.
fn parse_use_args(origin: &Origin, inner: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for piece in inner.split(',') {
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        let Some((k, v)) = piece.split_once('=') else {
            return Err(GrammarError::new(
                origin.clone(),
                format!("`{}` is not a `name=value` argument", piece),
            )
            .with_hint("write each argument as `name=value`, comma-separated."));
        };
        let key = k.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(GrammarError::new(
                origin.clone(),
                format!("`{}` is not a parameter name", key),
            )
            .with_hint("parameter names are letters, digits and `_`."));
        }
        out.push((key.to_string(), v.trim().to_string()));
    }
    Ok(out)
}

/// `param NAME` / `param NAME = DEFAULT` (U32). The name is an identifier; a default is
/// verbatim to end of line, trimmed, exactly like a `Var` value.
fn parse_param(origin: &Origin, rest: &str) -> Result<Statement> {
    if rest.is_empty() {
        return Err(GrammarError::new(origin.clone(), "`param` names nothing")
            .with_hint("write `param user` (required) or `param gpu = none` (with a default)."));
    }
    let (name, default) = match rest.split_once('=') {
        Some((n, d)) => (n.trim(), Some(d.trim().to_string())),
        None => (rest.trim(), None),
    };
    if name.is_empty()
        || !name.starts_with(|c: char| c.is_alphabetic() || c == '_')
        || !name.chars().all(|c| c.is_alphanumeric() || c == '_')
    {
        return Err(GrammarError::new(
            origin.clone(),
            format!("`{}` is not a parameter name", name),
        )
        .with_hint("parameter names start with a letter or `_` and hold letters, digits and `_`."));
    }
    Ok(Statement::Param {
        name: name.to_string(),
        default,
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

/// Read the `backend:` prefix of a package line, which may name a chain.
///
/// `apt` pins. `apt,dnf` and `apt,list` do not — they say what the line will accept, in
/// order, and leave the choosing to the machine. A comma rather than a hyphen because
/// package managers have hyphens in their names (`nix-env`, `apt-get`), and a separator a
/// backend name can contain is a separator that stops working the day such a backend is
/// added.
fn parse_prefix(
    origin: &Origin,
    prefix: &str,
    backends: &dyn BackendNames,
) -> Result<(Option<String>, Candidates)> {
    // A group is a shortcut for a comma-chain (U18), so expand it into that chain BEFORE any
    // validation — every member then goes through the same backend check and `list`-only rules a
    // hand-written chain does. `tools:rg` becomes `apt,dnf,cargo:rg`, and `tools,brew:rg` splices
    // the group's members in front of `brew`. Nested groups are already flattened to terminal
    // backends by `Groups` (a cycle was refused at load), so a single expansion here is complete.
    let mut expanded: Vec<String> = Vec::new();
    for raw in prefix.split(',').map(str::trim) {
        match backends.expand_group(raw) {
            Some(members) => expanded.extend(members),
            None => expanded.push(raw.to_string()),
        }
    }
    let parts: Vec<&str> = expanded.iter().map(String::as_str).collect();

    let unknown = |name: &str| {
        GrammarError::new(
            origin.clone(),
            format!("`{}` is not a backend LiNix uses", name),
        )
        .with_hint(format!(
            "add `{}` to your `priority` file, or check the spelling. Not listed means \
             LiNix does not use it at all.",
            name
        ))
    };

    // `list` is only a tail: everything after it would never be reached, and writing
    // something unreachable means the line does not say what its author thinks it says.
    if let Some(pos) = parts.iter().position(|p| *p == PRIORITY_KEYWORD) {
        if pos != parts.len() - 1 {
            return Err(GrammarError::new(
                origin.clone(),
                format!("`{}` must come last in `{}`", PRIORITY_KEYWORD, prefix),
            )
            .with_hint(format!(
                "`{}` already means every manager in `priority`, so nothing written after \
                 it can ever be reached.",
                PRIORITY_KEYWORD
            )));
        }
    }

    let mut named: Vec<String> = Vec::new();
    let mut ends_in_priority = false;
    for part in &parts {
        if part.is_empty() {
            return Err(GrammarError::new(
                origin.clone(),
                format!("`{}` has an empty backend in it", prefix),
            )
            .with_hint("write `apt,dnf:rg` — one manager between each comma."));
        }
        if *part == PRIORITY_KEYWORD {
            ends_in_priority = true;
            continue;
        }
        if !backends.is_backend(part) {
            return Err(unknown(part));
        }
        if named.iter().any(|n| n == part) {
            return Err(GrammarError::new(
                origin.clone(),
                format!("`{}` is named twice in `{}`", part, prefix),
            )
            .with_hint("the first one already decides; the second can never be reached."));
        }
        named.push(part.to_string());
    }

    Ok(match (named.len(), ends_in_priority) {
        // `list:rg`, which is what a bare `rg` means spelled out.
        (0, true) => (None, Candidates::Priority),
        (0, false) => return Err(unknown(prefix)),
        // One manager and no tail is the pin: apt or nothing.
        (1, false) => (Some(named.remove(0)), Candidates::Priority),
        (_, false) => (None, Candidates::Named(named)),
        (_, true) => (None, Candidates::NamedThenPriority(named)),
    })
}

/// A name reaches a manager's command line, where a leading `-` is an option and not a name.
/// The `--` every invocation emits (II.12b) holds for managers that honour it; this holds for
/// the rest, and it is the layer that can say *which line* is wrong.
fn reject_leading_dash(origin: &Origin, name: &str) -> Result<()> {
    if name.starts_with('-') {
        return Err(GrammarError::new(
            origin.clone(),
            format!("`{}` starts with `-`, so it is an option and not a package name", name),
        )
        .with_hint(
            "package names reach the manager's command line. If you meant to take a package \
             out of the set, a subtraction is `-name` at the start of its own line.",
        ));
    }
    Ok(())
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

    let (backend, candidates, rest) = match head.split_once(':') {
        Some((prefix, rest)) => {
            let (backend, candidates) = parse_prefix(origin, prefix, backends)?;
            (backend, candidates, rest.trim())
        }
        None => (None, Candidates::Priority, head),
    };

    let selector = match rest.strip_prefix("re:") {
        Some(pattern) => {
            let pattern = pattern.trim();
            if pattern.is_empty() {
                return Err(GrammarError::new(origin.clone(), "`re:` has no pattern"));
            }
            // A pattern is matched against one manager's catalogue and frozen in that
            // manager's regex lock. Spread over a chain there is no single catalogue to
            // match and no single lock to write, so the line has to pin.
            if backend.is_none() {
                return Err(GrammarError::new(
                    origin.clone(),
                    format!("`{}` must match in exactly one backend", head),
                )
                .with_hint(
                    "write `apt:re:^fonts-`. A pattern is matched against one manager's \
                     catalogue, so a chain has nothing to match against.",
                ));
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
            reject_leading_dash(origin, rest)?;
            Selector::Name(rest.to_string())
        }
    };

    Ok(PackageDecl {
        backend,
        candidates,
        selector,
        options,
    })
}

/// Every option rule in II.2, for every statement that carries options.
///
/// This runs on the finished statement rather than inside the header parse, because a block
/// body's keys are merged in after the header is parsed. Validating at the header let
/// `apt:jq@hold { version = 1.6 }` through — the same contradiction the short form refuses,
/// silent — and II.2 closes with the reason that cannot stand: silently ignoring an option
/// the user wrote is how a config grows lines that do nothing.
pub fn validate(origin: &Origin, stmt: &Statement) -> Result<()> {
    match stmt {
        Statement::Package(decl) => validate_options(origin, decl, false),
        Statement::Absent(decl) => validate_options(origin, decl, true),
        Statement::Shim(name, o) => validate_extra_options(origin, "shim", name, o),
        Statement::Service(name, o) => validate_extra_options(origin, "service", name, o),
        Statement::Link(name, o) => validate_extra_options(origin, "link", name, o),
        Statement::Schedule(name, o) => validate_extra_options(origin, "schedule", name, o),
        Statement::Setting(name, o) => validate_setting(origin, name, o),
        Statement::Exec(name, o) => validate_exec(origin, name, o),
        Statement::Dotfiles(name, o) => validate_extra_options(origin, "dotfiles", name, o),
        Statement::Firewall(name, o) => validate_firewall(origin, name, o),
        Statement::Repo { .. }
        | Statement::Use(..)
        | Statement::Param { .. }
        | Statement::Exclude(_)
        | Statement::Intersect(_)
        | Statement::Subtract(_)
        | Statement::Var { .. }
        | Statement::Expr(_) => Ok(()),
    }
}

/// The options each non-package statement understands (II.2's table).
///
/// A `schedule:` also needs `cron` and `run` to be *present*, which `model::schedule` checks
/// when it builds the job — that is a question about one line's meaning, not about which
/// words are legal, and it has an error that can name what is missing.
/// `scope` is on exactly the three statements where "for me" and "for the machine" can differ
/// (U19). A `service:` is the init system's business and a `repo:` is the manager's, so
/// neither takes it — a key that means nothing on a statement is a key that will be written
/// there and silently ignored.
pub const SHIM_OPTION_KEYS: &[&str] = &["source", "scope"];
pub const SERVICE_OPTION_KEYS: &[&str] = &["enabled", "status"];
pub const LINK_OPTION_KEYS: &[&str] =
    &["target", "content", "template", "decrypt", "identity", "scope", "backup"];
pub const SCHEDULE_OPTION_KEYS: &[&str] = &["cron", "run", "notify"];
pub const SETTING_OPTION_KEYS: &[&str] = &["value", "scope"];
/// `runs` caps how many times a distinct script content may run — `1` (the default) is
/// run-once-per-content; `always` opts out (see `model::exec`). `undo` is deliberately absent:
/// what a removal means is U3, still open, so no key promises it.
/// `undo` is what removing the line runs (U3). Optional, because a script has no inverse and
/// inventing one would be LiNix claiming to undo something it cannot: without it, removing an
/// `exec:` drops the record and nothing else, and `plan` says so in those words.
pub const EXEC_OPTION_KEYS: &[&str] = &["runs", "undo"];
/// `target` is where the tree is mirrored to; absent means the home directory, which is what a
/// dotfiles tree mirrors by definition. There is deliberately no per-file option: the tree has
/// no place to write one, which is why it never decrypts (U24).
pub const DOTFILES_OPTION_KEYS: &[&str] = &["target"];
/// `value` is the policy a `default/...` rule sets (`allow` or `deny`). A port rule takes no
/// options: `firewall:22/tcp` is the whole declaration.
pub const FIREWALL_OPTION_KEYS: &[&str] = &["value"];

/// A firewall line names a rule the grammar can read, and a default policy says which one.
fn validate_firewall(origin: &Origin, name: &str, options: &Options) -> Result<()> {
    validate_extra_options(origin, "firewall", name, options)?;
    let rule = crate::model::firewall::Rule::parse(name)
        .map_err(|e| GrammarError::new(origin.clone(), e))?;
    match rule {
        crate::model::firewall::Rule::Default { .. } => {
            match options.one("value").map(str::trim) {
                Some("allow") | Some("deny") => Ok(()),
                _ => Err(GrammarError::new(
                    origin.clone(),
                    format!("`firewall:{}` needs a policy", name),
                )
                .with_hint(
                    "say which way it goes: `@value=deny` or `@value=allow`. A default policy                      with no value declares nothing, and it is the most consequential line in                      a firewall.",
                )),
            }
        }
        // A port rule is its own declaration; `@value=` on one would be a second way to say
        // the same thing, and a confusing one (`firewall:22/tcp @value=deny` reads as both).
        crate::model::firewall::Rule::Port { .. } => match options.one("value") {
            None => Ok(()),
            Some(_) => Err(GrammarError::new(
                origin.clone(),
                format!("`firewall:{}` takes no `value`", name),
            )
            .with_hint(
                "a declared port is open — that is what declaring it means. To close one,                  delete the line; `@value=` belongs on `default/incoming` only.",
            )),
        },
    }
}

fn keys_for(prefix: &str) -> &'static [&'static str] {
    match prefix {
        "shim" => SHIM_OPTION_KEYS,
        "service" => SERVICE_OPTION_KEYS,
        "link" => LINK_OPTION_KEYS,
        "setting" => SETTING_OPTION_KEYS,
        "exec" => EXEC_OPTION_KEYS,
        "dotfiles" => DOTFILES_OPTION_KEYS,
        "firewall" => FIREWALL_OPTION_KEYS,
        _ => SCHEDULE_OPTION_KEYS,
    }
}

/// An `exec:` names a script and, optionally, how many times its content may run. The name
/// must be non-empty; `runs`, if present, is a positive count or the word `always`.
/// `@scope=` must name one of the two things it can mean. A misspelling that parsed as
/// "default" would be a line that reads as a decision and behaves as if nobody made one.
fn validate_scope(origin: &Origin, prefix: &str, name: &str, options: &Options) -> Result<()> {
    let Some(written) = options.one("scope") else {
        return Ok(());
    };
    if crate::model::scope::Scope::parse(written).is_none() {
        return Err(GrammarError::new(
            origin.clone(),
            format!("`{}:{}` has an invalid `scope={}`", prefix, name, written),
        )
        .with_hint(format!(
            "scope is {}. Omitting it means whatever this store does by default.",
            crate::model::scope::Scope::vocabulary()
        )));
    }
    Ok(())
}

fn validate_exec(origin: &Origin, name: &str, options: &Options) -> Result<()> {
    if name.trim().is_empty() {
        return Err(GrammarError::new(origin.clone(), "`exec:` names no script")
            .with_hint("write `exec:./bin/setup.sh` — a path to a script the config carries."));
    }
    for key in options.keys() {
        if !EXEC_OPTION_KEYS.contains(&key) {
            return Err(GrammarError::new(
                origin.clone(),
                format!("`exec:{}` has an unknown option `{}`", name, key),
            )
            .with_hint(
                "an exec takes `runs` (a positive number, or `always`) and `undo` (a command \
                 to run when the line is removed).",
            ));
        }
    }
    if let Some(runs) = options.one("runs") {
        let runs = runs.trim();
        if runs != "always" && runs.parse::<u32>().map(|n| n == 0).unwrap_or(true) {
            return Err(GrammarError::new(
                origin.clone(),
                format!("`exec:{}` has an invalid `runs={}`", name, runs),
            )
            .with_hint("`runs` is a positive number (the ceiling on how many times this \
                        content runs) or `always` to run every sync."));
        }
    }
    Ok(())
}

/// Split `SCHEMA/KEY` into its halves. The one place the shape is decided, so the parser's
/// refusal and the adapter's lookup cannot disagree about what a setting names.
pub fn split_setting(name: &str) -> Option<(&str, &str)> {
    let (schema, key) = name.split_once('/')?;
    let (schema, key) = (schema.trim(), key.trim());
    if schema.is_empty() || key.is_empty() || key.contains('/') {
        return None;
    }
    Some((schema, key))
}

/// A setting names a schema, a key inside it, and the value it must hold. A line missing any
/// of the three describes no state, and applying it would mean choosing on the user's behalf
/// which key they meant.
fn validate_setting(origin: &Origin, name: &str, options: &Options) -> Result<()> {
    validate_extra_options(origin, "setting", name, options)?;

    if split_setting(name).is_none() {
        return Err(GrammarError::new(
            origin.clone(),
            format!("`{}` is not `SCHEMA/KEY`", name),
        )
        .with_hint(
            "a setting names the schema and the key inside it, separated by one `/`: \
             `setting:org.gnome.desktop.interface/color-scheme @value=prefer-dark`.",
        ));
    }

    if options.one("value").is_none_or(str::is_empty) {
        return Err(
            GrammarError::new(origin.clone(), format!("`setting:{}` has no value", name))
                .with_hint(
                    "say what the key must hold: `@value=prefer-dark`. A setting with no value \
                     declares nothing.",
                ),
        );
    }
    if options.all("value").len() > 1 {
        return Err(
            GrammarError::new(origin.clone(), format!("`setting:{}` has two values", name))
                .with_hint("a key holds one value. Name the one you want."),
        );
    }
    Ok(())
}

fn validate_extra_options(
    origin: &Origin,
    prefix: &str,
    name: &str,
    options: &Options,
) -> Result<()> {
    let legal = keys_for(prefix);
    for key in options.keys() {
        if legal.contains(&key) {
            continue;
        }
        return Err(GrammarError::new(
            origin.clone(),
            format!("`@{}` is not an option on `{}:`", key, prefix),
        )
        .with_hint(format!(
            "`{}:{}` takes: {}.",
            prefix,
            name,
            legal.join(", ")
        )));
    }
    validate_scope(origin, prefix, name, options)
}

/// Every option a package line may carry (II.2's table). Hooks are `*_install`
/// (`after_install`, `before_install`, …), so they are matched by suffix rather than listed.
///
/// `until` is here and refused below unless the line is `absent:` — II.2 puts it on
/// `absent:` only, and "not an option" would be the wrong error for a key that exists.
const PACKAGE_OPTION_KEYS: &[&str] = &[
    "version", "hold", "expires", "until", "requires", "sha256", "formats", "asset", "bin",
    "channel", "allow_http", "unverified", "health", "download_only",
];

/// Options that are only meaningful on a backend that resolves one name to several
/// downloadable artifacts, or that publishes several version streams. Each is refused by name
/// on any other backend: an option nobody reads is a line that does nothing.
/// Takes the backend and the options rather than a declaration, because the same rules apply
/// to a backend's options body in `priority` (VIII.2) and one of them had to be the caller.
pub fn validate_artifact_options(
    origin: &Origin,
    backend: Option<&str>,
    o: &Options,
) -> Result<()> {
    use crate::backends::artifact::{capability, AssetPattern, FormatOrder};

    for key in ["formats", "asset", "bin"] {
        if !o.contains(key) {
            continue;
        }
        // A line with no prefix is resolved through `priority` later, so the backend that will
        // answer it is not known here. Refusing would break `fd@formats=deb`; the resolver
        // enforces it once the backend is known.
        let Some(backend) = backend else { continue };
        if !capability::selects_artifacts(backend) {
            return Err(GrammarError::new(
                origin.clone(),
                format!("`@{}` is not an option on `{}`", key, backend),
            )
            .with_hint(format!(
                "`{}` picks between several files of one release. Backends that offer a \
                 choice: {}. Everywhere else the ecosystem already decided the file.",
                key,
                capability::artifact_backends()
            )));
        }
    }

    if o.contains("channel") {
        if let Some(backend) = backend {
            if !capability::has_channels(backend) {
                return Err(GrammarError::new(
                    origin.clone(),
                    format!("`@channel` is not an option on `{}`", backend),
                )
                .with_hint(format!(
                    "a channel is a version stream, not a file. Backends that publish \
                     channels: {}.",
                    capability::channel_backends()
                )));
            }
        }
        if o.all("channel").len() > 1 {
            return Err(GrammarError::new(
                origin.clone(),
                "`@channel` takes one value",
            )
            .with_hint(
                "there is no fallback across version streams — trying `edge` and settling for \
                 `stable` would silently downgrade the machine. Name the one you want.",
            ));
        }
    }

    for name in o.all("formats") {
        FormatOrder::parse_all([name])
            .map_err(|e| GrammarError::new(origin.clone(), e.to_string()))?;
    }

    if let Some(pattern) = o.one("asset") {
        AssetPattern::parse(pattern)
            .map_err(|e| GrammarError::new(origin.clone(), e.to_string()))?;
    }
    if o.all("asset").len() > 1 {
        return Err(GrammarError::new(
            origin.clone(),
            "`@asset` takes one pattern",
        )
        .with_hint(
            "one pattern, which may be a glob: `@asset=*musl*`. For every matching file, \
             `@asset=all`.",
        ));
    }

    // `@download_only` (D3b) means "fetch but do not install" — a distinction only a backend
    // that downloads a file can draw. Every other backend hands the whole job to a package
    // manager, so there is no fetch-without-install to ask for.
    if o.contains("download_only") {
        if let Some(backend) = backend {
            if !capability::downloads(backend) {
                return Err(GrammarError::new(
                    origin.clone(),
                    format!("`@download_only` is not an option on `{}`", backend),
                )
                .with_hint(format!(
                    "it fetches a file without installing it, which only {} do.",
                    capability::download_backends()
                )));
            }
        }
    }

    // SEC2's two opt-outs relax a rule that only exists where LiNix downloads and executes.
    // On any other backend they are a line that does nothing, which II.2 refuses.
    for key in ["allow_http", "unverified"] {
        if !o.contains(key) {
            continue;
        }
        let Some(backend) = backend else { continue };
        if !capability::downloads(backend) {
            return Err(GrammarError::new(
                origin.clone(),
                format!("`@{}` is not an option on `{}`", key, backend),
            )
            .with_hint(format!(
                "it relaxes a rule about downloading and running a file, which only {} do. \
                 Everywhere else the package manager's own index answers for the bytes.",
                capability::download_backends()
            )));
        }
    }

    // `@asset=all` installs every match, so there is no single artifact for one hash to cover.
    // Checked before the pinned-format rule below: both objections are true of
    // `@asset=all,sha256=…`, and this one names the reason the line cannot be fixed by
    // pinning a format.
    if o.one("asset").is_some_and(|a| a.eq_ignore_ascii_case("all")) && o.contains("sha256") {
        return Err(GrammarError::new(
            origin.clone(),
            "`@asset=all` and `@sha256=` cannot both be set",
        )
        .with_hint(
            "`all` installs several files and one hash cannot verify them. Pin one file, or \
             drop the checksum.",
        ));
    }

    // One hash cannot cover an asset that varies by machine (D6): a shared module says
    // `github:x/y` and the Debian box downloads the `.deb` while the Fedora box downloads the
    // `.rpm`. A hand-written hash is only a claim about a file when the line names one file,
    // so it is legal only where the format is pinned to exactly one. Everywhere else the hash
    // is generated content and lives in `locks/<backend>.toml`.
    if o.contains("sha256")
        && backend.is_some_and(capability::selects_artifacts)
        && o.all("formats").len() != 1
    {
        let said = o.all("formats").len();
        return Err(GrammarError::new(
            origin.clone(),
            format!(
                "`@sha256` needs the line to pin exactly one format, and it {}",
                if said == 0 {
                    "pins none".to_string()
                } else {
                    format!("lists {}", said)
                }
            ),
        )
        .with_hint(
            "one release ships several files and one hash cannot verify them all. Add \
             `@formats=` naming one, or drop the checksum — LiNix records the hash of what it \
             downloaded in `locks/` either way.",
        ));
    }

    Ok(())
}

/// Option rules from II.2's table that are about the options themselves rather than any
/// one backend.
fn validate_options(origin: &Origin, decl: &PackageDecl, absent: bool) -> Result<()> {
    let o = &decl.options;

    // II.2's table is the whole list. An unknown key used to be kept and handed downstream,
    // where something might act on it — `@lease=2h` is the one that mattered: II.16 retired
    // it, nothing writes it, and `StateRegistry::add` still read it and turned it into a
    // real expiry. So a key this document deleted was silently still a package that
    // uninstalls itself (S19). An option nobody reads is a line that does nothing; an
    // option someone still reads is worse.
    for key in o.keys() {
        if PACKAGE_OPTION_KEYS.contains(&key) || key.ends_with("_install") {
            continue;
        }
        let mut err = GrammarError::new(origin.clone(), format!("`@{}` is not an option", key));
        err = match key {
            // The one worth naming, because it used to work.
            "lease" | "duration" => err.with_hint(
                "a lease is a dated line now: `@expires=2026-07-17T14:00`. A file cannot hold \
                 \"2 hours\" — it would mean something different every time it was read.",
            ),
            _ => err.with_hint(format!(
                "options on a package are: {}, and the `*_install` hooks.",
                PACKAGE_OPTION_KEYS.join(", ")
            )),
        };
        return Err(err);
    }

    // A health check decides whether the machine is rolled back (XIII.5), so a line whose
    // check cannot be understood must not parse. `@health=port:donkey` that read as a shell
    // command would be a probe that fails every time and reverts every sync.
    if let Some(written) = o.one("health") {
        if crate::model::health::Probe::parse(written).is_none() {
            return Err(GrammarError::new(
                origin.clone(),
                format!("`@health={}` is not a check", written),
            )
            .with_hint(
                "a health check is `port:8080` — something must be listening — or a command \
                 that exits 0, written plainly or as `cmd:systemctl is-active nginx`.",
            ));
        }
    }

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
    // grammar has no way to act on — so it is refused there, naming the file and line,
    // rather than parsed and quietly ignored.
    if !absent && o.contains("until") {
        return Err(GrammarError::new(
            origin.clone(),
            "`@until` is only for `absent:` lines",
        )
        .with_hint(
            "`@until` lifts an `absent:` line on a date (absent now, present after). To make \
             a present line lapse on a date, use `@expires`.",
        ));
    }

    validate_artifact_options(origin, decl.backend.as_deref(), &decl.options)?;
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

    /// A `BackendNames` that also knows one group, `web = cargo, npm`, for the U18 tests.
    struct WithGroup;
    impl BackendNames for WithGroup {
        fn is_backend(&self, name: &str) -> bool {
            known(name)
        }
        fn expand_group(&self, name: &str) -> Option<Vec<String>> {
            (name == "web").then(|| vec!["cargo".to_string(), "npm".to_string()])
        }
    }

    /// U18: a group prefix expands to exactly the chain it names — `web:rg` is `cargo,npm:rg`.
    #[test]
    fn a_group_prefix_expands_to_its_chain() {
        let Statement::Package(d) = parse(&o(), "web:ripgrep", &WithGroup).unwrap() else {
            panic!("web:ripgrep did not parse as a package")
        };
        assert_eq!(d.backend, None, "a chain is not a pin");
        assert_eq!(
            d.candidates,
            Candidates::Named(vec!["cargo".into(), "npm".into()])
        );
    }

    /// A group composes with a backend in the chain, splicing its members in place.
    #[test]
    fn a_group_composes_with_a_backend_in_the_chain() {
        let Statement::Package(d) = parse(&o(), "web,apt:ripgrep", &WithGroup).unwrap() else {
            panic!()
        };
        assert_eq!(
            d.candidates,
            Candidates::Named(vec!["cargo".into(), "npm".into(), "apt".into()])
        );
    }

    #[test]
    fn a_bare_name_has_no_backend() {
        let Statement::Package(d) = p("ripgrep").unwrap() else {
            panic!()
        };
        assert_eq!(d.backend, None);
        assert_eq!(d.selector, Selector::Name("ripgrep".into()));
    }

    /// A health check decides whether the machine is rolled back, so a line whose check
    /// cannot be understood must not parse (XIII.5). `@health=port:donkey` reading as a shell
    /// command would be a probe that fails every time — and therefore reverts every sync.
    #[test]
    fn a_health_check_that_is_not_a_check_is_refused() {
        let err = p("apt:nginx@health=port:donkey").unwrap_err();
        assert!(err.to_string().contains("is not a check"), "{}", err);
        assert!(p("apt:nginx@health=").is_err());
    }

    #[test]
    fn both_shapes_of_health_check_parse() {
        for line in [
            "apt:nginx@health=port:80",
            "apt:nginx@health=systemctl is-active nginx",
            "apt:nginx@health=cmd:true",
        ] {
            assert!(p(line).is_ok(), "`{}` should parse", line);
        }
    }

    /// The `Candidates` of a line that parses, for the chain tests.
    fn cands(line: &str) -> (Option<String>, Candidates) {
        let Statement::Package(d) = p(line).unwrap() else {
            panic!("`{}` did not parse as a package", line)
        };
        (d.backend, d.candidates)
    }

    #[test]
    fn a_lone_backend_pins_and_a_chain_does_not() {
        // The distinction the whole design rests on: `apt:rg` is apt or nothing, so it is
        // still apt on a machine that also has cargo. Anything with a comma is a preference,
        // not a pin, and the machine gets to answer.
        assert_eq!(cands("apt:curl"), (Some("apt".into()), Candidates::Priority));
        assert_eq!(
            cands("apt,cargo:ripgrep"),
            (None, Candidates::Named(vec!["apt".into(), "cargo".into()]))
        );
    }

    #[test]
    fn list_is_how_a_bare_name_is_spelled_out() {
        assert_eq!(cands("ripgrep"), (None, Candidates::Priority));
        assert_eq!(cands("list:ripgrep"), (None, Candidates::Priority));
    }

    #[test]
    fn a_chain_can_end_in_the_whole_priority_list() {
        assert_eq!(
            cands("apt,list:ripgrep"),
            (None, Candidates::NamedThenPriority(vec!["apt".into()]))
        );
    }

    #[test]
    fn nothing_may_follow_list_in_a_chain() {
        // Unreachable syntax that parses is syntax that lies about what the line does.
        let err = p("list,apt:ripgrep").unwrap_err();
        assert!(err.what.contains("must come last"), "{}", err.what);
    }

    #[test]
    fn a_backend_named_twice_in_a_chain_is_refused() {
        let err = p("apt,cargo,apt:ripgrep").unwrap_err();
        assert!(err.what.contains("named twice"), "{}", err.what);
    }

    #[test]
    fn an_unknown_backend_inside_a_chain_is_still_unknown() {
        // C13 again, one level down: the chain must not become a place where an unchecked
        // prefix slips through.
        let err = p("apt,nope:ripgrep").unwrap_err();
        assert!(err.what.contains("`nope` is not a backend"), "{}", err.what);
    }

    #[test]
    fn an_empty_slot_in_a_chain_is_refused() {
        let err = p("apt,,cargo:ripgrep").unwrap_err();
        assert!(err.what.contains("empty backend"), "{}", err.what);
    }

    #[test]
    fn a_pattern_cannot_span_a_chain() {
        // A pattern is matched against one catalogue and frozen in one regex lock; a chain
        // gives it neither.
        let err = p("apt,cargo:re:^fonts-").unwrap_err();
        assert!(err.what.contains("exactly one backend"), "{}", err.what);
        assert!(p("apt:re:^fonts-").is_ok());
    }

    #[test]
    fn the_order_asked_is_the_order_written_then_priority() {
        let priority: Vec<String> = ["apt", "snap", "cargo"].iter().map(|s| s.to_string()).collect();
        assert_eq!(Candidates::Priority.order(&priority), priority);
        assert_eq!(
            Candidates::Named(vec!["cargo".into()]).order(&priority),
            vec!["cargo".to_string()],
            "a closed chain never reaches priority"
        );
        // The named head keeps its place and is not repeated when the tail names it again.
        assert_eq!(
            Candidates::NamedThenPriority(vec!["cargo".into()]).order(&priority),
            vec!["cargo".to_string(), "apt".to_string(), "snap".to_string()]
        );
    }

    #[test]
    fn a_chain_only_accepts_what_it_lists() {
        let priority: Vec<String> = ["apt", "snap"].iter().map(|s| s.to_string()).collect();
        let chain = Candidates::Named(vec!["apt".into()]);
        assert!(chain.accepts("apt", &priority));
        // The lock naming `snap` is not an answer to this line, even though the host lists it.
        assert!(!chain.accepts("snap", &priority));
        assert!(Candidates::Priority.accepts("snap", &priority));
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
    fn the_reserved_words_are_the_ones_the_prefix_grammar_spends() {
        // `re:` introduces a pattern and `list` names the priority file, so a backend
        // answering to either would make `re:foo` / `apt,list:foo` ambiguous forever. The
        // onboarder refuses these names; this pins the list it refuses.
        assert!(RESERVED_BACKEND_NAMES.contains(&"re"));
        assert!(RESERVED_BACKEND_NAMES.contains(&PRIORITY_KEYWORD));
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
            Statement::Use(Reference::Module("editors".into()), vec![])
        );
    }

    #[test]
    fn use_takes_a_profile_by_capitalized_name() {
        assert_eq!(
            p("use Work").unwrap(),
            Statement::Use(Reference::Profile("Work".into()), vec![])
        );
    }

    #[test]
    fn use_parses_module_arguments() {
        // U32: `use workstation(user=shaul, gpu=nvidia)`.
        assert_eq!(
            p("use workstation(user=shaul, gpu=nvidia)").unwrap(),
            Statement::Use(
                Reference::Module("workstation".into()),
                vec![
                    ("user".into(), "shaul".into()),
                    ("gpu".into(), "nvidia".into())
                ]
            )
        );
    }

    #[test]
    fn use_argument_values_may_contain_slashes() {
        // The `/` is in an argument value, not the `use` target, so it is not a path.
        assert_eq!(
            p("use m(path=/etc/foo)").unwrap(),
            Statement::Use(
                Reference::Module("m".into()),
                vec![("path".into(), "/etc/foo".into())]
            )
        );
    }

    #[test]
    fn a_profile_cannot_take_arguments() {
        let err = p("use Work(user=shaul)").unwrap_err();
        assert!(err.what.contains("passes arguments to a profile"), "{}", err);
    }

    #[test]
    fn an_unclosed_use_paren_is_an_error() {
        assert!(p("use m(user=shaul").is_err());
    }

    #[test]
    fn param_parses_with_and_without_a_default() {
        assert_eq!(
            p("param user").unwrap(),
            Statement::Param {
                name: "user".into(),
                default: None
            }
        );
        assert_eq!(
            p("param gpu = none").unwrap(),
            Statement::Param {
                name: "gpu".into(),
                default: Some("none".into())
            }
        );
    }

    #[test]
    fn param_names_must_be_identifiers() {
        assert!(p("param 9lives").is_err());
        assert!(p("param a-b").is_err());
        assert!(p("param").is_err());
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
        // V.47: the backend is named, and the spec keeps its own colons.
        assert_eq!(
            p("repo:apt:ppa:deadsnakes/ppa").unwrap(),
            Statement::Repo {
                backend: "apt".into(),
                spec: "ppa:deadsnakes/ppa".into()
            }
        );
    }

    #[test]
    fn a_repo_without_a_backend_is_refused() {
        // A repository belongs to one package manager; guessing runs the wrong system
        // command (V.47). `snap` isn't in this test's known set, so it also proves the
        // backend is validated.
        let err = p("repo:ppa:deadsnakes/ppa").unwrap_err();
        assert!(err.what.contains("not a backend"), "{}", err);
        assert!(err.hint.unwrap().contains("apt"));
    }

    #[test]
    fn shim_carries_its_source() {
        let Statement::Shim(name, opts) = p("shim:jq@source=cargo:jq").unwrap() else {
            panic!()
        };
        assert_eq!(name, "jq");
        assert_eq!(opts.one("source"), Some("cargo:jq"));
    }

    /// A typo'd key on one of these used to parse clean and then do nothing — the same
    /// silent-line defect the package table exists to prevent, through a different door.
    #[test]
    fn a_typo_on_an_extra_is_refused_by_name() {
        for line in [
            "shim:jq@sorce=cargo:jq",
            "service:nginx@enabld=true",
            "link:/a/b@targt=/c",
            "schedule:nightly@crron=0 2 * * *",
        ] {
            let err = p(line).unwrap_err();
            assert!(err.what.contains("is not an option"), "{}: {}", line, err);
        }
    }

    #[test]
    fn the_documented_keys_on_an_extra_are_accepted() {
        for line in [
            "shim:jq@source=cargo:jq",
            "service:nginx@enabled=true@status=started",
            "link:/a/b@target=/c@template=true",
            "schedule:nightly@cron=0 2 * * *,run=sync",
        ] {
            assert!(p(line).is_ok(), "{} was refused", line);
        }
    }

    #[test]
    fn a_setting_names_a_schema_a_key_and_a_value() {
        let Statement::Setting(name, opts) =
            p("setting:org.gnome.desktop.interface/color-scheme@value=prefer-dark").unwrap()
        else {
            panic!("not a setting");
        };
        assert_eq!(name, "org.gnome.desktop.interface/color-scheme");
        assert_eq!(opts.one("value"), Some("prefer-dark"));
    }

    #[test]
    fn a_setting_without_a_slash_is_not_schema_key() {
        let err = p("setting:color-scheme@value=prefer-dark").unwrap_err();
        assert!(err.what.contains("SCHEMA/KEY"), "{}", err);
    }

    #[test]
    fn a_setting_with_no_value_declares_nothing() {
        let err = p("setting:org.gnome.x/color-scheme").unwrap_err();
        assert!(err.what.contains("no value"), "{}", err);
    }

    #[test]
    fn a_setting_takes_one_value_not_two() {
        let err = p("setting:org.gnome.x/k@value=a,value=b").unwrap_err();
        assert!(err.what.contains("two values"), "{}", err);
    }

    #[test]
    fn a_typo_on_a_setting_is_refused_like_any_other_extra() {
        let err = p("setting:org.gnome.x/k@vale=dark").unwrap_err();
        assert!(err.what.contains("is not an option"), "{}", err);
    }

    #[test]
    fn every_error_names_the_file_and_line() {
        let err = p("aptt:curl").unwrap_err();
        assert!(err.to_string().contains("modules/dev.txt:7"), "{}", err);
    }
}

#[cfg(test)]
mod option_key_tests {
    use super::*;

    fn known(name: &str) -> bool {
        matches!(name, "apt" | "cargo")
    }

    fn parse_line(line: &str) -> Result<Statement> {
        parse(&Origin::new("modules/dev.txt", 1), line, &known)
    }

    #[test]
    fn lease_is_refused_and_points_at_the_dated_line() {
        // S19. II.16 retired `@lease=2h`, and nothing LiNix writes used it — but
        // `StateRegistry::add` still READ it and turned it into a real expiry, so a
        // hand-written lease was silently a package that uninstalls itself, on a path the
        // guard does not see (C3).
        let err = parse_line("apt:jq@lease=2h").unwrap_err();
        assert!(err.what.contains("`@lease` is not an option"), "{}", err);
        assert!(err.hint.unwrap().contains("@expires="), "must teach the replacement");
    }

    #[test]
    fn an_unknown_key_lists_the_real_ones() {
        // II.2's table is the whole list. A key nobody reads is a line that does nothing; a
        // key someone still reads is worse.
        let err = parse_line("apt:jq@colour=blue").unwrap_err();
        assert!(err.what.contains("`@colour` is not an option"), "{}", err);
        let hint = err.hint.unwrap();
        assert!(hint.contains("version"), "{}", hint);
        assert!(hint.contains("requires"), "{}", hint);
    }

    #[test]
    fn every_key_in_the_table_is_accepted() {
        for line in [
            "apt:jq@version=1.6",
            "apt:jq@hold",
            "apt:jq@expires=2026-07-17T14:00",
            "apt:jq@requires=apt:libfoo",
            "apt:nginx@after_install=./setup.sh",
            "apt:nginx@before_install=./pre.sh",
        ] {
            assert!(parse_line(line).is_ok(), "{} must parse", line);
        }
        // `until` belongs to `absent:` (II.2), and is accepted there.
        assert!(parse_line("absent:apt:steam@until=2026-07-20T00:00").is_ok());
    }

    #[test]
    fn until_on_a_present_line_is_refused() {
        // II.2: `@until` is for `absent:` only (absent now, present after). On a present line
        // it means "install this later", which nothing can act on. It used to parse clean.
        let err = parse_line("apt:steam@until=2026-07-20T00:00").unwrap_err();
        assert!(err.what.contains("only for `absent:`"), "{}", err);
        assert!(err.hint.unwrap().contains("@expires"), "must point at the present-line form");
    }

    #[test]
    fn a_link_with_a_windows_path_is_a_link_not_an_expression() {
        // II.2 vs II.4: `looks_like_expression` fires on `\`, and a Windows path is full of
        // them. The typed prefix has to win, or `link:C:\Users\me\.vimrc` parses as set math.
        let stmt = parse_line(r"link:C:\Users\me\.vimrc@target=~/.vimrc").unwrap();
        assert!(matches!(stmt, Statement::Link(..)), "got {:?}", stmt);
        // And an actual expression with no statement prefix still reads as one.
        assert!(matches!(
            parse_line("editors | fonts").unwrap(),
            Statement::Expr(_)
        ));
    }
}

#[cfg(test)]
mod artifact_option_tests {
    use super::*;

    fn known(name: &str) -> bool {
        matches!(name, "apt" | "github" | "snap" | "flatpak" | "appimage")
    }

    fn p(line: &str) -> Result<Statement> {
        parse(&Origin::new("modules/dev.txt", 3), line, &known)
    }

    fn options_of(line: &str) -> Options {
        match p(line).unwrap() {
            Statement::Package(d) => d.options,
            other => panic!("expected a package, got {:?}", other),
        }
    }

    #[test]
    fn formats_is_read_on_a_backend_that_offers_a_choice() {
        let o = options_of("github:sharkdp/fd@formats=deb");
        assert_eq!(o.all("formats"), vec!["deb"]);
    }

    #[test]
    fn a_repeated_formats_key_is_an_ordered_list() {
        let o = options_of("github:sharkdp/fd@formats=deb,formats=tarball");
        assert_eq!(o.all("formats"), vec!["deb", "tarball"]);
    }

    #[test]
    fn an_unknown_format_names_the_legal_set() {
        let err = p("github:sharkdp/fd@formats=snapcraft").unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("snapcraft"));
        assert!(msg.contains("appimage"), "the error must list the vocabulary");
    }

    #[test]
    fn formats_on_a_backend_that_decided_already_is_an_error() {
        let err = p("apt:curl@formats=deb").unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("not an option on `apt`"));
        assert!(msg.contains("github"), "the error must name where it is legal");
    }

    #[test]
    fn formats_on_appimage_is_a_contradiction_and_is_refused() {
        assert!(p("appimage:foo@formats=deb").is_err());
    }

    #[test]
    fn download_only_is_read_on_a_download_backend_and_refused_elsewhere() {
        // D3b: fetch-without-install is a distinction only a downloading backend can draw.
        assert_eq!(
            options_of("github:sharkdp/fd@download_only").one("download_only"),
            Some("true")
        );
        assert_eq!(
            options_of("appimage:https://host/x.AppImage@download_only").one("download_only"),
            Some("true")
        );
        let err = p("apt:curl@download_only").unwrap_err();
        assert!(format!("{}", err).contains("not an option on `apt`"));
    }

    #[test]
    fn channel_is_read_on_snap_and_flatpak() {
        assert_eq!(options_of("snap:code@channel=stable").one("channel"), Some("stable"));
        assert_eq!(
            options_of("flatpak:org.gimp.GIMP@channel=stable").one("channel"),
            Some("stable")
        );
    }

    #[test]
    fn channel_on_a_backend_without_version_streams_is_an_error() {
        let err = p("github:sharkdp/fd@channel=stable").unwrap_err();
        assert!(format!("{}", err).contains("not an option on `github`"));
    }

    #[test]
    fn there_is_no_fallback_across_channels() {
        let err = p("snap:code@channel=edge,channel=stable").unwrap_err();
        assert!(format!("{}", err).contains("one value"));
    }

    #[test]
    fn an_asset_pattern_is_validated_at_parse_time() {
        assert_eq!(
            options_of("github:sharkdp/fd@asset=*musl*").one("asset"),
            Some("*musl*")
        );
    }

    #[test]
    fn asset_all_and_a_checksum_cannot_both_be_set() {
        let err = p("github:sharkdp/fd@asset=all,sha256=abc").unwrap_err();
        assert!(format!("{}", err).contains("cannot both be set"));
    }

    #[test]
    fn a_checksum_needs_the_line_to_pin_one_format() {
        // D6: `github:x/y@sha256=…` with no format pinned means the Debian box downloads the
        // `.deb` and the Fedora box the `.rpm`, and one hash cannot verify two files.
        let err = p("github:sharkdp/fd@sha256=abc").unwrap_err();
        assert!(format!("{}", err).contains("exactly one format"), "{}", err);
        assert!(format!("{}", err).contains("locks/"), "{}", err);
    }

    #[test]
    fn a_checksum_beside_one_pinned_format_is_legal() {
        assert!(p("github:sharkdp/fd@sha256=abc,formats=deb").is_ok());
    }

    #[test]
    fn a_checksum_beside_a_list_of_formats_is_not() {
        let err = p("github:sharkdp/fd@sha256=abc,formats=deb,formats=rpm").unwrap_err();
        assert!(format!("{}", err).contains("lists 2"), "{}", err);
    }

    #[test]
    fn a_checksum_on_a_backend_that_selects_nothing_is_untouched() {
        // `appimage:` already names one file — the backend name is the format — so there is
        // nothing to pin, and demanding `@formats=` there would be unanswerable.
        assert!(p("appimage:https://example.com/tool.AppImage@sha256=abc").is_ok());
    }

    #[test]
    fn bin_names_the_executable_inside_an_archive() {
        assert_eq!(
            options_of("github:foo/bar@bin=build/bar").one("bin"),
            Some("build/bar")
        );
    }

    #[test]
    fn a_bare_name_defers_the_capability_check_to_the_resolver() {
        // No prefix means `priority` decides the backend, so the grammar cannot know yet
        // whether `formats` is legal — refusing here would break every unprefixed line.
        assert!(p("fd@formats=deb").is_ok());
    }
}

#[cfg(test)]
mod exec_tests {
    use super::*;

    fn o() -> Origin {
        Origin::new("modules/dev.txt", 7)
    }
    fn known(name: &str) -> bool {
        matches!(name, "apt" | "cargo")
    }
    fn p(line: &str) -> Result<Statement> {
        parse(&o(), line, &known)
    }
    /// Parse and validate, the way a real file is read — `parse` alone does not check options.
    fn pv(line: &str) -> Result<Statement> {
        let s = p(line)?;
        validate(&o(), &s)?;
        Ok(s)
    }

    #[test]
    fn an_exec_names_a_script() {
        let Statement::Exec(script, opts) = pv("exec:./bin/enroll-tpm.sh").unwrap() else {
            panic!("not an exec");
        };
        assert_eq!(script, "./bin/enroll-tpm.sh");
        assert!(opts.one("runs").is_none(), "no ceiling means the default");
    }

    #[test]
    fn an_exec_takes_a_runs_ceiling() {
        let Statement::Exec(_, opts) = pv("exec:./setup.sh@runs=3").unwrap() else {
            panic!("not an exec");
        };
        assert_eq!(opts.one("runs"), Some("3"));
        let Statement::Exec(_, opts) = pv("exec:./tick.sh@runs=always").unwrap() else {
            panic!("not an exec");
        };
        assert_eq!(opts.one("runs"), Some("always"));
    }

    /// A path with punctuation is a path, not set math — the same rule that keeps a Windows
    /// `link:` target from being read as a difference.
    #[test]
    fn a_windows_path_is_a_script_not_an_expression() {
        let Statement::Exec(script, _) = pv(r"exec:C:\Users\me\bin\setup.ps1").unwrap() else {
            panic!("not an exec");
        };
        assert_eq!(script, r"C:\Users\me\bin\setup.ps1");
    }

    #[test]
    fn an_exec_that_names_nothing_is_refused() {
        assert!(p("exec:").is_err());
    }

    #[test]
    fn an_unknown_exec_option_is_refused_and_names_the_real_one() {
        let err = pv("exec:./s.sh@run=2").unwrap_err();
        assert!(err.what.contains("unknown option `run`"), "{}", err);
        assert!(err.to_string().contains("runs"), "{}", err);
    }

    /// `runs=0` would mean "never runs", which is what deleting the line means. A ceiling that
    /// silently disables the statement is the kind of quiet no-op II.2 refuses.
    #[test]
    fn a_zero_or_garbage_ceiling_is_refused() {
        for bad in ["exec:./s.sh@runs=0", "exec:./s.sh@runs=lots", "exec:./s.sh@runs=-1"] {
            assert!(pv(bad).is_err(), "{} was accepted", bad);
        }
    }

    /// `exec:` is a verb: it must never be keyed into the extras teardown ledger, or a script
    /// whose `when` went false would be "undone" (XIII.3's flapping bug).
    #[test]
    fn an_exec_is_not_an_extra_with_a_teardown_key() {
        let stmt = pv("exec:./bin/enroll-tpm.sh").unwrap();
        assert_eq!(crate::core::extra_key(&stmt), None);
    }
}

/// U19: `@scope=user|system` on the three statements where it can differ.
#[cfg(test)]
mod scope_tests {
    use super::*;

    fn o() -> Origin {
        Origin::new("modules/dev.txt", 3)
    }
    fn known(name: &str) -> bool {
        matches!(name, "apt" | "cargo")
    }
    fn pv(line: &str) -> Result<Statement> {
        let s = parse(&o(), line, &known)?;
        validate(&o(), &s)?;
        Ok(s)
    }

    #[test]
    fn scope_is_accepted_on_the_three_statements_that_can_vary() {
        for line in [
            "setting:org.gnome.desktop.interface/color-scheme@value=dark,scope=user",
            "link:./dotfiles/gitconfig@target=~/.gitconfig,scope=user",
            "shim:rg@scope=user",
        ] {
            assert!(pv(line).is_ok(), "{} was refused", line);
        }
    }

    /// Owner ruling: writing the scope that is already the default is accepted, not refused as
    /// redundant. A configuration may state a thing it would also get for free — saying it out
    /// loud is how a reader learns the answer without going to look it up.
    #[test]
    fn writing_the_default_scope_is_not_an_error() {
        assert!(pv("shim:rg@scope=user").is_ok());
        assert!(pv("link:./f@target=~/.f,scope=user").is_ok());
    }

    /// A statement where the question does not arise does not take the key: a key that means
    /// nothing where it is written is a key that gets written there and silently ignored.
    #[test]
    fn scope_is_refused_where_it_means_nothing() {
        for line in ["service:nginx@scope=system", "schedule:nightly@cron=@daily,run=sync,scope=user"] {
            let err = pv(line).unwrap_err();
            assert!(err.what.contains("not an option"), "{}: {}", line, err);
        }
    }

    /// A misspelling must not read as "the default" — that would be a line that looks like a
    /// decision and behaves as if nobody made one.
    #[test]
    fn a_misspelled_scope_is_refused_and_lists_the_legal_ones() {
        for bad in ["shim:rg@scope=machine", "shim:rg@scope=global", "shim:rg@scope=User"] {
            let err = pv(bad).unwrap_err();
            assert!(err.what.contains("invalid `scope="), "{}: {}", bad, err);
            let full = err.to_string();
            assert!(full.contains("user") && full.contains("system"), "{}", full);
        }
    }
}

/// Part XI: `firewall:` lines, and the one option only a default policy takes.
#[cfg(test)]
mod firewall_tests {
    use super::*;

    fn o() -> Origin {
        Origin::new("modules/net.txt", 2)
    }
    fn known(name: &str) -> bool {
        matches!(name, "apt")
    }
    fn pv(line: &str) -> Result<Statement> {
        let s = parse(&o(), line, &known)?;
        validate(&o(), &s)?;
        Ok(s)
    }

    #[test]
    fn a_port_rule_is_its_own_whole_declaration() {
        let Statement::Firewall(name, opts) = pv("firewall:22/tcp").unwrap() else {
            panic!("not a firewall rule");
        };
        assert_eq!(name, "22/tcp");
        assert!(opts.one("value").is_none());
    }

    /// N4: the default policy is declarable, and it must say which way it goes — it is the most
    /// consequential line in a firewall, so a silent one is the worst case.
    #[test]
    fn a_default_policy_needs_a_direction_and_a_value() {
        assert!(pv("firewall:default/incoming@value=deny").is_ok());
        assert!(pv("firewall:default/outgoing@value=allow").is_ok());

        let err = pv("firewall:default/incoming").unwrap_err();
        assert!(err.what.contains("needs a policy"), "{}", err);

        assert!(pv("firewall:default/sideways@value=deny").is_err());
        assert!(pv("firewall:default/incoming@value=maybe").is_err());
    }

    /// A declared port is open — that is what declaring it means. `@value=` on one would be a
    /// second way to say the same thing, and `firewall:22/tcp @value=deny` reads as both.
    #[test]
    fn a_port_rule_refuses_a_value() {
        let err = pv("firewall:22/tcp@value=deny").unwrap_err();
        assert!(err.what.contains("takes no `value`"), "{}", err);
        assert!(err.to_string().contains("delete the line"), "{}", err);
    }

    #[test]
    fn a_rule_the_grammar_cannot_read_is_refused_at_parse_time() {
        for bad in ["firewall:22", "firewall:http/tcp", "firewall:22/sctp", "firewall:0/tcp"] {
            assert!(pv(bad).is_err(), "{} was accepted", bad);
        }
    }

    /// A firewall rule is a noun with a teardown — unlike `exec:`, it belongs in the extras
    /// ledger so that deleting the line closes the port (N5).
    #[test]
    fn a_firewall_rule_is_an_extra_with_a_teardown_key() {
        let stmt = pv("firewall:22/tcp").unwrap();
        assert_eq!(
            crate::core::extra_key(&stmt).as_deref(),
            Some("firewall:22/tcp")
        );
    }
}
