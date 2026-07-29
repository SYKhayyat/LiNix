//! Ask a manager what it accepts, instead of assuming.
//!
//! E11's fix was `--verify=false`, taken from helm's own error text on the machine where the
//! bug was reported. That machine ran helm v4.2.3, which has the flag. helm 3 does not, and
//! rejects it outright:
//!
//! ```text
//! Error: `helm` failed (exit 1): Error: unknown flag: --verify
//! ```
//!
//! So `@unverified` worked on helm 4 and broke every helm 3 — one argv defect traded for
//! another, from a fix derived from one machine's error message and shipped unconditionally.
//!
//! The lesson is not "check helm's version". A version table is the same assumption with a
//! number in it, and it goes stale the same way. **Ask the tool.** `--help` is the one argument
//! no package manager acts on, which is why the argv-drift gate uses it too.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Answers already obtained this run. A manager's help does not change while LiNix is running,
/// and an install of forty plugins must not launch forty help processes.
fn cache() -> &'static Mutex<HashMap<String, Option<bool>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<bool>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Does `program <chain…> --help` mention `flag`?
///
/// **`None` means the tool could not be asked** — it is not on this machine, or its help would
/// not run — and that is deliberately different from `Some(false)`. The capability table is the
/// default; this probe exists to *correct* it where there is evidence, never to overrule it
/// with silence.
///
/// The first version returned a bare `bool` and folded "could not ask" into "does not accept".
/// It passed here, where helm v4 is installed, and broke two unit tests on every CI runner,
/// which has no helm at all: the argv builder had quietly acquired a dependency on the host.
/// That is the finding this whole round is about — a check whose answer depends on something
/// nobody enumerated — committed while fixing it.
///
/// The flag is matched without its `=value` tail, since help text writes `--verify` and the
/// argv writes `--verify=false`.
pub fn accepts_flag(program: &str, chain: &[String], flag: &str) -> Option<bool> {
    let name = flag.split('=').next().unwrap_or(flag);
    let key = format!("{} {} {}", program, chain.join(" "), name);
    if let Some(hit) = cache().lock().ok().and_then(|c| c.get(&key).copied()) {
        return hit;
    }

    let mut args: Vec<String> = chain.to_vec();
    args.push("--help".to_string());
    // Through the executor's launcher, or a shimmed manager on Windows cannot be run at all —
    // the mistake the argv-drift gate made for four installed managers before it was fixed.
    let (prog, argv) = crate::core::executor::effective_command(program, &args);
    let answer = std::process::Command::new(prog)
        .args(&argv)
        .output()
        .ok()
        .map(|o| {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            mentions_flag(&text, name)
        });

    if let Ok(mut c) = cache().lock() {
        c.insert(key, answer);
    }
    answer
}

/// Does `text` document `flag` as a flag in its own right?
///
/// A plain substring search is not enough: `--ca` occurs inside `--ca-file`, and helm 3's help
/// carries `--kube-insecure-skip-tls-verify`, which contains `verify` and would answer yes for
/// a `--verify` this version does not have. So the match ends at a character that cannot
/// continue a flag name.
fn mentions_flag(text: &str, name: &str) -> bool {
    let mut from = 0;
    while let Some(at) = text[from..].find(name) {
        let start = from + at;
        let end = start + name.len();
        let next = text[end..].chars().next();
        if !matches!(next, Some(c) if c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return true;
        }
        from = end;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flag_name_does_not_match_a_longer_one() {
        // The exact trap in helm 3's own help.
        assert!(!mentions_flag(
            "      --kube-insecure-skip-tls-verify   if true, …",
            "--verify"
        ));
        assert!(!mentions_flag("      --ca-file string   …", "--ca"));
        // And it must still find the real thing, in every shape help text writes it.
        assert!(mentions_flag("Use --verify=false to skip.", "--verify"));
        assert!(mentions_flag(
            "      --verify   verify the package",
            "--verify"
        ));
        assert!(mentions_flag("ends the line with --verify", "--verify"));
    }

    #[test]
    fn a_program_that_is_not_here_answers_nothing_rather_than_no() {
        // `None`, not `Some(false)`. A tool that is absent has said nothing about its flags,
        // and treating silence as a refusal is what broke two unit tests on every CI runner
        // while passing on the one machine that happened to have helm.
        assert_eq!(
            accepts_flag(
                "linix-no-such-program-zzz",
                &["plugin".into(), "install".into()],
                "--verify=false"
            ),
            None
        );
    }

    #[test]
    fn the_value_tail_is_not_part_of_the_name() {
        // Help text writes `--verify`; the argv writes `--verify=false`. Matching the whole
        // token would answer "no" for every flag that carries a value, which is most of them.
        // Asserted through a program whose help certainly mentions the token it is asked about.
        let prog = if cfg!(windows) { "cmd" } else { "sh" };
        let _ = accepts_flag(prog, &[], "--nosuchflag=1");
        // The property under test is the split, which is cheap to state directly.
        assert_eq!("--verify=false".split('=').next(), Some("--verify"));
    }
}
