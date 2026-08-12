//! **A built-in backend is a row in `builtin_backends.toml`, and the row is checked.**
//!
//! Twenty-three backends stopped being `fn register_npm(…)` in `registry.rs` and became rows in
//! a table. That trade is only worth taking if the table is checked as hard as the compiler
//! checked the functions, because a row can be wrong in ways a function cannot:
//!
//! - **A reader name that resolves to nothing.** Every named-reader field is read with
//!   `.as_deref().and_then(named::installed)` — a typo yields `None`, and `None` is a legal
//!   value meaning *fall back to the described parser*. `reads = "ws_name_versoin"` therefore
//!   compiles, loads, registers, and reports an empty machine. That is Q40's class arriving
//!   through a new door.
//! - **A `search_args` with no `searches`.** `NamedParser::new` takes `searches: Option<Search>`
//!   and substitutes a closure returning the empty vector. A row that can search and named no
//!   search reader has a live `Searchable` capability answering "no such package" to everything.
//! - **A name that is also a registrar.** `register_builtin_backends` runs first in
//!   `create_default_registry` so a hand-written registration wins a collision — deliberate, but
//!   only safe while somebody notices the collision exists.
//! - **An `install_source_option` the grammar does not know about.** The option is written in
//!   the row; the grammar decides whether `@url` is legal on this backend from
//!   `capability::install_source_key`. Two spellings of one fact, in two files.
//!
//! The argv assertions the twenty-three already had did not move: `registry.rs`'s test module
//! keeps them, with `rows_as_registrars!` standing in for the deleted functions. What did move
//! is coverage — `every_registrar_has_an_argv_row_or_a_written_reason` scans for registrars, and
//! a row is not one. `every_row_has_an_argv_row` below is that gate's other half.

use std::collections::BTreeSet;

use shall::backends::capability;
use shall::backends::onboarder::{builtin_rows, CustomBackendDef};
use shall::parsers::named;

/// Far below twenty-three, and it is a floor on the *scan*, not a count of the backends: a
/// parse that returned nothing would pass every assertion in this file.
const FLOOR: usize = 15;

fn rows() -> Vec<CustomBackendDef> {
    let rows = builtin_rows();
    assert!(
        rows.len() >= FLOOR,
        "read only {} rows out of builtin_backends.toml — the table did not parse, or it was \
         emptied; either way the checks below are vacuous",
        rows.len()
    );
    rows
}

/// Every named reader a row asks for, paired with the resolver that will actually be asked.
///
/// The pairing is the point. `outdated_reads` goes through `named::probe` and `essential_reads`
/// through `named::names`; a reader that exists under one and not the other resolves to `None`
/// at the call site that matters while looking present in the crate.
/// A row's field, the value in it, and whether the resolver that field goes through answers.
type Field<'a> = (&'static str, &'a Option<String>, fn(&str) -> bool);

fn named_readers(def: &CustomBackendDef) -> Vec<(&'static str, &str, bool)> {
    let fields: [Field; 6] = [
        ("reads", &def.reads, |n| named::installed(n).is_some()),
        ("searches", &def.searches, |n| named::search(n).is_some()),
        ("outdated_reads", &def.outdated_reads, |n| {
            named::probe(n).is_some()
        }),
        ("machine_list_reads", &def.machine_list_reads, |n| {
            named::installed(n).is_some()
        }),
        ("essential_reads", &def.essential_reads, |n| {
            named::names(n).is_some()
        }),
        ("depends_reads", &def.depends_reads, |n| {
            named::names(n).is_some()
        }),
    ];
    fields
        .into_iter()
        .filter_map(|(field, value, found)| value.as_deref().map(|v| (field, v, found(v))))
        .collect()
}

#[test]
fn every_row_can_read_what_it_asks_for() {
    let rows = rows();
    let mut checked = 0usize;
    let mut broken: Vec<String> = Vec::new();
    for def in &rows {
        for (field, name, resolves) in named_readers(def) {
            checked += 1;
            if !resolves {
                broken.push(format!("  {}: {field} = \"{name}\"", def.name));
            }
        }
    }
    assert!(
        checked >= FLOOR,
        "found only {checked} named readers across {} rows — either the table stopped naming \
         readers, in which case every built-in is back to a described parser, or this scan \
         stopped seeing them",
        rows.len()
    );
    assert!(
        broken.is_empty(),
        "these rows name a reader that resolves to nothing, which is not a load error — the \
         field is an `Option` and `None` means *use the described parser instead*, so the \
         backend registers and reports an empty machine:\n{}\n\nAdd the reader to \
         `src/parsers/named.rs` (and to the matching `*_NAMES` list), or fix the spelling.",
        broken.join("\n")
    );
}

#[test]
fn a_row_that_can_search_names_a_search_reader() {
    let mut silent: Vec<String> = Vec::new();
    for def in &rows() {
        if def.search_args.is_empty() || def.reads.is_none() {
            continue;
        }
        if def.searches.is_none() {
            silent.push(def.name.clone());
        }
    }
    assert!(
        silent.is_empty(),
        "these rows have `search_args` and a named `reads`, so their parser is a `NamedParser` \
         — and `NamedParser::new` substitutes a closure returning the empty vector when \
         `searches` is absent. `shall search` on {silent:?} would answer \"no such package\" to \
         every query, from a backend advertising `Searchable`. Name a reader in `searches`."
    );
}

#[test]
fn a_row_that_lists_names_a_reader_or_describes_one() {
    let mut guessing: Vec<String> = Vec::new();
    for def in &rows() {
        if def.list_args.is_empty() {
            continue;
        }
        if def.reads.is_none() && def.parser.is_none() {
            guessing.push(def.name.clone());
        }
    }
    assert!(
        guessing.is_empty(),
        "these rows can list and said nothing about the shape of the listing, so they get \
         `ParserSpec::default()` — one bare name per line. That is a real shape and a wrong \
         answer for anything that prints a version beside the name: the version becomes part of \
         the name and every package reads as drift. Give {guessing:?} a `reads` or a `parser`."
    );
}

/// The two places the same fact is written: the row carries the option, the grammar decides
/// whether `@source` is legal on the backend from `capability::install_source_key`.
#[test]
fn a_row_that_installs_from_a_source_agrees_with_the_grammar() {
    let mut disagreements: Vec<String> = Vec::new();
    for def in &rows() {
        let in_row = def.install_source_option.as_deref();
        let in_grammar = capability::install_source_key(&def.name);
        if in_row != in_grammar {
            disagreements.push(format!(
                "  {}: row says {in_row:?}, capability::install_source_key says {in_grammar:?}",
                def.name
            ));
        }
    }
    assert!(
        disagreements.is_empty(),
        "the row and the grammar disagree about where this backend installs from:\n{}\n\nThe \
         row's option is what gets appended to the argv; the grammar's key is what decides \
         whether the user is allowed to write it. A backend with the option and no key refuses \
         a line it could have run; one with the key and no option accepts the line and drops \
         the source on the floor.",
        disagreements.join("\n")
    );
}

/// Registrars still written by hand, in the two shapes `registry.rs` writes them.
///
/// Read from `create_default_registry` only — the test module names a registrar per row (see
/// `rows_as_registrars!`), and those are precisely the shims that must NOT count as a second
/// registration.
///
/// **What this cannot see:** a registrar whose function name differs from the backend name it
/// registers. `register_pubdart` registers `pub`, because `pub` is a keyword. There is one such
/// name today and it is a row, not a registrar; a future second one would need its own line
/// here rather than a wider scan, because widening the scan to *every* `register_*` call would
/// re-admit the test module.
fn hand_written_registrations(src: &str) -> BTreeSet<String> {
    let production = src
        .split_once("pub async fn create_default_registry(")
        .expect("create_default_registry moved or was renamed")
        .1
        .split_once("\n}")
        .expect("create_default_registry has no end")
        .0;

    let mut out = BTreeSet::new();
    for line in production.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("register_") {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() && rest[name.len()..].starts_with('(') {
                out.insert(name);
            }
        }
        for chunk in line.split("crate::backends::").skip(1) {
            let module: String = chunk
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !module.is_empty() && chunk[module.len()..].starts_with("::register(") {
                out.insert(module);
            }
        }
    }
    out
}

#[test]
fn no_backend_is_both_a_row_and_a_registrar() {
    let src = crate::harness::registry_source();
    let hand = hand_written_registrations(&src);
    assert!(
        hand.len() > 10,
        "found only {} hand-written registrations — the scan is broken, not the code",
        hand.len()
    );

    let both: Vec<String> = rows()
        .into_iter()
        .map(|d| d.name)
        .filter(|n| hand.contains(n))
        .collect();
    assert!(
        both.is_empty(),
        "{both:?} exist twice: once as a row in `builtin_backends.toml` and once as a \
         registration in `create_default_registry`. Which one the machine runs is decided by the \
         order of two calls — today the hand-written one wins, because the rows go in first. \
         Delete one. Two of everything is how this repo got into trouble."
    );
}

/// The other half of `every_registrar_has_an_argv_row_or_a_written_reason`, which scans for
/// registrars and therefore stopped seeing these twenty-three the moment they became rows.
#[test]
fn every_row_has_an_argv_row() {
    let table = crate::harness::registry_argv_table();
    let cases =
        table.matches("ArgvCase::pkg(").count() + table.matches("ArgvCase::shaped(").count();
    assert!(
        cases > 40,
        "counted only {cases} argv rows — the scan is broken, not the code"
    );

    let uncovered: Vec<String> = rows()
        .iter()
        .map(|d| d.name.clone())
        .filter(|n| !table.contains(&format!("\"{n}\",")))
        .collect();
    assert!(
        uncovered.is_empty(),
        "{uncovered:?} are rows in `builtin_backends.toml` with no case in `argv_cases()`. \
         Becoming data is not a reason to stop checking what the backend actually runs — the \
         argv is the part a user notices being wrong."
    );
}

// ---------------------------------------------------------------------------------------------
// The fixture column: a row that reads a manager carries the bytes that manager printed.
// ---------------------------------------------------------------------------------------------

/// How many rows may still say `UNVERIFIED` in their fixture's `source`.
///
/// **A ratchet, not a budget.** It starts at the number of managers that had no image to hand on
/// the day the column was added, and the only legal edit to this line is downward. An unverified
/// fixture is bytes somebody typed from an upstream README: better than the seven hand-typed
/// words that served eight managers before it, and not the same thing as evidence.
const UNVERIFIED_CEILING: usize = 7;

#[test]
fn every_row_that_reads_a_listing_carries_its_manager_s_bytes() {
    let mut bare: Vec<String> = Vec::new();
    for def in &rows() {
        if def.list_args.is_empty() {
            continue;
        }
        match &def.fixture {
            None => bare.push(def.name.clone()),
            Some(f) if f.list.is_none() => {
                bare.push(format!("{} (fixture with no `list`)", def.name))
            }
            Some(f) if f.source.trim().is_empty() => {
                bare.push(format!("{} (fixture with no `source`)", def.name))
            }
            Some(_) => {}
        }
    }
    assert!(
        bare.is_empty(),
        "these rows can list and carry no bytes their manager printed: {bare:?}\n\nA reader is a \
         claim about a tool and only the tool's output settles it. `ws_name_version` served eight \
         managers on seven words typed by hand and labelled `helm`; the `[backend.fixture]` block \
         is what stops the ninth. Capture the output, paste it into `list`, write what it should \
         read as into `expect`, and say in `source` where it came from."
    );
}

#[test]
fn every_fixture_reads_the_way_its_row_says_it_does() {
    let rows = rows();
    let mut checked = 0usize;
    let mut wrong: Vec<String> = Vec::new();
    for def in &rows {
        if def.fixture.is_some() {
            checked += 1;
            wrong.extend(shall::backends::onboarder::fixture_disagreements(def));
        }
    }
    // Content before the floor, deliberately. A run that found nine fixtures and a real
    // disagreement should say which manager disagrees, not "only nine fixtures".
    assert!(
        wrong.is_empty(),
        "these managers print something this build reads differently from what the row \
         claims:\n  {}\n\nOn the installed side that gap is the whole bug: a listing read as \
         empty is a machine `sync` answers by installing everything declared.",
        wrong.join("\n  ")
    );
    assert!(
        checked >= FLOOR,
        "only {checked} rows carry a fixture — the scan found nothing to check, which passes \
         every assertion above it"
    );
}

#[test]
fn the_number_of_fixtures_nobody_captured_only_falls() {
    let rows = rows();
    let unverified: Vec<String> = rows
        .iter()
        .filter_map(|d| {
            let f = d.fixture.as_ref()?;
            (!f.is_verified()).then(|| d.name.clone())
        })
        .collect();
    assert!(
        unverified.len() <= UNVERIFIED_CEILING,
        "{} rows carry a fixture whose `source` says UNVERIFIED, and the ceiling is \
         {UNVERIFIED_CEILING}: {unverified:?}. Raising the ceiling is not a fix — run the manager \
         and paste what it printed.",
        unverified.len()
    );
    assert!(
        unverified.len() >= UNVERIFIED_CEILING || UNVERIFIED_CEILING == 0,
        "only {} rows are still unverified but the ceiling says {UNVERIFIED_CEILING} — lower the \
         constant in this file so the next one that slips is caught",
        unverified.len()
    );
}

/// A row that says `os = "linux"` is registered on Linux and nowhere else, through
/// `AdapterRow::applies_here` — the same gate every other adapter table goes through.
#[test]
fn a_row_pinned_to_one_os_is_registered_on_that_os_alone() {
    use shall::core::adapter::AdapterRow;

    let rows = rows();
    let pinned: Vec<&CustomBackendDef> = rows.iter().filter(|d| d.os.is_some()).collect();
    assert!(
        !pinned.is_empty(),
        "no row names an OS — either the table lost its OS-native managers, or `os` stopped \
         being read, in which case `emerge` is about to register on Windows"
    );
    for def in pinned {
        let os = def.os.as_deref().expect("filtered on `is_some`");
        assert!(
            def.applies_to(os),
            "`{}` says os = \"{os}\" and does not apply to it — the spelling is not one \
             `applies_to` answers to",
            def.name
        );
        for other in ["linux", "macos", "windows"] {
            if other != os {
                assert!(
                    !def.applies_to(other),
                    "`{}` is pinned to {os} and applies to {other} as well",
                    def.name
                );
            }
        }
    }
}
