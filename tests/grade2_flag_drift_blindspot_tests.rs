//! The flag half of the argv-drift gate, asserted in **both** directions.
//!
//! GRADER round 3 found G-8: `VERIFIES_ITSELF = [("helm", "--verify=false")]` was derived from
//! one machine's helm 4 error message and emitted unconditionally, and helm 3 answers `unknown
//! flag: --verify`. The gate built to catch an argv upstream will not accept could not see it,
//! because it only examined subcommands. The round-2 fix added a flag half to
//! `tests/argv_drift_tests.rs` and a runtime probe (`tool_help::accepts_flag`) that withholds
//! the flag when the tool's help does not document it — and the probe defeated the gate:
//!
//!     # one character of drift planted in the capability table
//!     $ sed -i 's/--verify=false/--linix-bogus-flag-zzz/' src/backends/artifact/capability.rs
//!     $ DRIFT_DUMP=1 cargo test --test argv_drift_tests -- --include-ignored
//!     CALL: helm plugin install -- https://example.invalid/linix-drift-probe
//!     test result: ok. 1 passed
//!
//! The bogus flag never reaches an argv, so a gate that reads argvs has nothing to check.
//!
//! **This file's first version asserted one direction — the flag must survive into the argv —
//! and was red on every helm 3 for a reason that was never drift.** `Q14`, ruled 2026-07-30:
//! helm 3 does not verify plugins *at all* (`helm plugin install --help` documents `--help` and
//! `--version` and nothing else — no `--verify`, no `--keyring`, no provenance). So on helm 3
//! the state `@unverified` asks for is the state the machine is already in. **"Accepted and
//! does nothing" is a defect; "accepted and already true" is a correct no-op**, and reading the
//! second as the first would have refused a correct line and removed the only way to install a
//! helm plugin on helm 3 — the capability Q5's ruling existed to create.
//!
//! So the assertion is now *the argv agrees with the tool's own help*, in both directions —
//! and the branch is chosen by **whether the tool documents verification at all**, not by
//! whether it documents our flag:
//!
//!   * the tool documents verification -> the table's flag must be one it accepts, and must be
//!     sent. A renamed flag is DRIFT and goes red here.
//!   * the tool documents none         -> nothing sent, and nothing said about it. helm 3 is
//!     this case (V.104): a warning on a run that did the right thing teaches people warnings
//!     are noise, and the case where silence would be wrong is not lost — the install then
//!     fails and `verification_note` speaks at the one moment the distinction matters.
//!
//! **The discriminator is the whole point, and the obvious version of this test does not have
//! it.** Asserting only "the argv agrees with `accepts_flag`" is green whatever the table says:
//! plant `--linix-bogus-flag-zzz`, helm 4 does not document it, LiNix withholds it, and the
//! test calls that correct. That is G-8 all over again inside its own regression test. Asking
//! `documents_verification` separates "this tool never verified" from "our flag is the old
//! name", which is the one question `accepts_flag` cannot answer.
//!
//! Both branches can go red, on either helm version, which is what the one-directional version
//! could not do. Verified in both directions before committing — see the commit message.

use std::io::Write;
use std::sync::{Arc, Mutex};

/// A `MakeWriter` that keeps everything `tracing` emits, so the silence half of the assertion
/// is measured rather than assumed.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Captured;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// The program and the subcommand chain a call was built with: the non-flag prefix, which is
/// the help that documents the flag. Read off the argv LiNix produced rather than copied from
/// the backend, so the test asks the tool about the exact command LiNix ran.
fn chain_of(call: &str) -> (String, Vec<String>) {
    let mut words = call.split_whitespace().map(str::to_string);
    let program = words.next().unwrap_or_default();
    let chain = words.take_while(|w| !w.starts_with('-')).collect();
    (program, chain)
}

#[tokio::test]
async fn a_capability_flag_is_sent_exactly_when_the_tool_documents_it() {
    use dashmap::DashMap;
    use linix::core::executor::MockExecutor;
    use linix::core::{CommandExecutor, PackageSpec};

    let captured = Captured::default();
    let _log = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_writer(captured.clone())
            .with_max_level(tracing::Level::WARN)
            .finish(),
    );

    let vfs = Arc::new(DashMap::new());
    let mock = Arc::new(MockExecutor::new(vfs.clone()));
    let exec =
        CommandExecutor::with_layer(true, false, mock.clone(), vfs, Arc::new(DashMap::new()));
    let config = linix::config::Config::default();
    let registry = linix::backends::create_default_registry(
        exec,
        &config,
        Arc::new(linix::app::hooks::LuaHooks::new(&config).expect("hooks")),
    )
    .await;

    let mut verdicts: Vec<String> = Vec::new();
    let mut wrong: Vec<String> = Vec::new();

    for backend in registry.available() {
        let name = backend.name().to_string();
        let Some(flag) = linix::backends::capability::unverified_arg(&name) else {
            continue;
        };
        let Some(installable) = backend.as_installable() else {
            continue;
        };

        let mut spec = PackageSpec {
            name: "jq".into(),
            backend: name.clone(),
            ..Default::default()
        };
        spec.options
            .insert("unverified".to_string(), "true".to_string());
        // The install source, where the backend demands one — helm's `plugin install` takes a
        // URL, and without it the call fails before it builds an argv at all.
        if let Some(key) = linix::backends::capability::install_source_key(&name) {
            spec.options.insert(
                key.to_string(),
                "https://example.invalid/linix-drift-probe".to_string(),
            );
        }

        let before = mock.get_calls().await.len();
        let _ = installable
            .install(std::slice::from_ref(&spec), false)
            .await;
        let calls = mock.get_calls().await;
        let produced: Vec<String> = calls.into_iter().skip(before).collect();

        assert!(
            !produced.is_empty(),
            "`{name}` produced no argv for an `@unverified` install, so this check would pass \
             by testing nothing — the same way the drift gate did before the install source was \
             threaded through it."
        );

        let (program, chain) = chain_of(&produced[0]);
        let sent = produced.iter().any(|c| c.contains(flag));

        // Which branch this tool is in — asked of the tool, about the same command. `None`
        // means the probe could not ask (no such program here, or its help would not run), and
        // the capability table stays in charge, so there is nothing to assert either way.
        let verifies = linix::core::tool_help::documents_verification(&program, &chain);
        let accepts = linix::core::tool_help::accepts_flag(&program, &chain, flag);

        match verifies {
            None => verdicts.push(format!(
                "`{name}`: `{program} {} --help` could not be asked, so the table is in charge \
                 and this backend is not measured",
                chain.join(" ")
            )),
            // The tool verifies. Then `@unverified` has something to turn off, and the table's
            // flag has to be the name this version of the tool uses for it.
            Some(true) => {
                verdicts.push(format!(
                    "`{name}`: {program} documents verification — `{flag}` must be its name and \
                     must be sent"
                ));
                if accepts != Some(true) {
                    wrong.push(format!(
                        "`{name}`: `{program}` documents verification but not `{flag}`, so the \
                         capability table holds a name this version does not use. THIS IS THE \
                         DRIFT, and `accepts_flag` alone reports it as a tool that simply lacks \
                         the flag — which is why the branch is chosen by \
                         `documents_verification`.\n      argv: {}",
                        produced.join("\n            ")
                    ));
                } else if !sent {
                    wrong.push(format!(
                        "`{name}`: `{program}` documents `{flag}` and LiNix built an argv \
                         without it:\n      {}",
                        produced.join("\n      ")
                    ));
                }
            }
            // The tool does not verify at all. `@unverified` asks for a state it is already in.
            Some(false) => {
                verdicts.push(format!(
                    "`{name}`: {program} documents no verification of any kind — nothing may be \
                     sent, and nothing may be said"
                ));
                if sent {
                    wrong.push(format!(
                        "`{name}`: `{program}` documents no verification and LiNix sent \
                         `{flag}` anyway, which is `unknown flag` on every one of \
                         them:\n      {}",
                        produced.join("\n      ")
                    ));
                }
                let said = captured.text();
                if said.contains(flag) || said.contains("does not accept") {
                    wrong.push(format!(
                        "`{name}`: the flag was correctly withheld and LiNix warned about it. \
                         Q14 (V.104): this tool does not verify, so `@unverified` is already \
                         true here and a correct no-op is not something to warn about. It \
                         said:\n      {}",
                        said.trim()
                    ));
                }
            }
        }
    }

    // Named, never silent: a host with no such backend measured nothing, and that has to read
    // differently from a host where everything agreed.
    assert!(
        !verdicts.is_empty(),
        "no backend in `VERIFIES_ITSELF` is installed on this machine, so this gate examined \
         nothing. That is a named skip, not a pass — run it on a host with helm."
    );
    eprintln!("flag-drift verdicts:\n  {}", verdicts.join("\n  "));

    assert!(
        wrong.is_empty(),
        "the argv LiNix builds disagrees with the tool's own help:\n  {}",
        wrong.join("\n  ")
    );
}
