use crate::core::Result;
use tracing::{info, warn};

/// Bootstrap holds only what it uses. It is built from an [`App`](crate::app::App) by
/// `App::bootstrap()` and can be built without one.
pub struct Bootstrap<'a> {
    pub(crate) config: &'a std::sync::Arc<crate::config::Config>,
    pub(crate) executor: &'a crate::core::CommandExecutor,
    pub(crate) registry: &'a std::sync::Arc<crate::backends::BackendRegistry>,
}

impl Bootstrap<'_> {
    /// The `[[bootstrap]]` rows this repo carries, if the file is approved (7c/U10).
    pub fn rows(&self) -> Vec<crate::model::bootstrap::BootstrapDef> {
        let layout = self.config.layout();
        let Some(body) = crate::backends::onboarder::read_approved_definitions(
            &layout.adapter_bootstrap_file(),
            &layout.locks_dir(),
        ) else {
            return Vec::new();
        };
        match toml::from_str::<crate::model::bootstrap::BootstrapFile>(&body) {
            Ok(f) => f.bootstrap,
            Err(e) => {
                warn!(
                    "{}",
                    crate::app::adapters::cannot_use(
                        crate::app::adapters::surface("bootstrap").expect("a declared surface"),
                        e,
                    )
                );
                Vec::new()
            }
        }
    }
    /// Offer to obtain a package manager the configuration declares and this machine lacks
    /// (7c). **Ask, then do** — the command is printed in full and confirmed before it runs.
    ///
    /// A bootstrap is usually a vendor's `curl | sh`, which is exactly the thing that must not
    /// run because a config file said so and nobody looked. It is approved through II.12 (the
    /// file) *and* confirmed at the moment it would run (the command), because those answer
    /// different questions: whether this repo may carry the instruction, and whether you want
    /// it executed on this machine now.
    ///
    /// Missing managers with no bootstrap row are left alone — that is the ordinary state of a
    /// machine that simply does not use them, not a fault to report.
    pub async fn offer(&self, state: &crate::model::DesiredState) -> Result<()> {
        use std::io::IsTerminal;

        let rows = self.rows();
        if rows.is_empty() {
            return Ok(());
        }
        let os = std::env::consts::OS;

        // Only managers this configuration actually asks for. A row for a manager nothing
        // declares is an offer nobody needs.
        let mut wanted: Vec<String> = state.packages.keys().cloned().collect();
        wanted.sort();
        wanted.dedup();

        for manager in wanted {
            // Registered AND runnable means there is nothing to obtain.
            if self
                .registry
                .get(&manager)
                .map(|b| b.is_available())
                .unwrap_or(false)
            {
                continue;
            }
            let Some(row) = crate::model::bootstrap::for_manager(&rows, &manager, os) else {
                continue;
            };

            println!(
                "\n`{}` is declared in your configuration and is not installed here.\n\
                 Your repo says it is obtained with:\n\n    {}\n",
                manager,
                row.command_line()
            );

            if self.config.dry_run {
                crate::would_print!("a real run would ask before running that.");
                continue;
            }
            // Declines rather than refuses: an unattended run is better off without the
            // installer than failed over it. Never runs it unasked — an installer that arrives
            // with a pulled repo and executes because nobody was there to say no is the risk.
            let unattended = format!(
                "Not asking in a non-interactive shell — run `linix sync` yourself, or install \
                 `{manager}` by hand."
            );
            let proceed = crate::core::prompt::confirm(
                self.config.yes,
                &format!("Run that to install {manager}?"),
                crate::core::prompt::Unattended::Decline(&unattended),
            )
            .unwrap_or(false);
            if !proceed {
                if std::io::stdin().is_terminal() {
                    println!("Left `{}` alone.", manager);
                }
                continue;
            }

            let (program, args) = row.run.split_first().expect("a usable row has a command");
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            info!("obtaining `{}`...", manager);
            match self.executor.run(program, &refs, false).await {
                Ok(_) => info!("`{}` installed. The sync will use it.", manager),
                // Reported, not fatal: the rest of the sync is still worth doing, and the
                // packages that needed this manager will fail by name a moment later.
                Err(e) => warn!("could not obtain `{}`: {}", manager, e),
            }
        }
        Ok(())
    }
}
