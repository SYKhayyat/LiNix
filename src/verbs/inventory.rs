//! **The read-only verbs that describe what is here: `audit`, `sbom`, `export`, `why`.**
//!
//! None of them changes anything, and each answers a different question about the same machine —
//! what is vulnerable, what is installed, what would move to another machine, and what put this
//! package here. They were spread through a file called `history`.

use crate::verbs::prelude::*;

pub async fn handle_audit(app: &App, out: Output) -> Result<()> {
    let report = crate::app::insight::audit(app).await?;
    crate::app::insight::print_audit(&report, out).map_err(|e| e.into())
}

pub async fn handle_sbom(app: &App) -> Result<()> {
    println!("{}", crate::app::insight::sbom(app).await?);
    Ok(())
}

pub async fn handle_export(
    app: &App,
    format: Option<&str>,
    out: &str,
    stdout: bool,
    force: bool,
) -> Result<()> {
    use crate::app::export::{export, Format, Outcome};
    let fmt = match format {
        Some(s) => Some(
            Format::parse(s)
                .with_context(|| format!("unknown export format '{}' (brew|pip|npm|apt)", s))?,
        ),
        None => None,
    };
    if stdout && fmt.is_none() {
        anyhow::bail!("--stdout needs a single --format (brew|pip|npm|apt).");
    }
    let out_dir = std::path::PathBuf::from(out);
    let results = export(app, fmt, &out_dir, stdout, force, app.config.dry_run).await?;
    for (file, outcome) in &results {
        match outcome {
            Outcome::NoPackages => println!("  skipped {} (no matching packages)", file),
            Outcome::Wrote(path) => println!("  wrote   {}", path.display()),
            Outcome::WouldWrite(path) => {
                println!(
                    "  {} would write {}",
                    crate::core::dry_run::MARKER,
                    path.display()
                )
            }
            Outcome::WroteBeside { taken, renamed } => {
                println!("  wrote   {}", renamed.display());
                println!(
                    "          ({} already exists and was left alone; re-run with --force to replace it)",
                    taken.display()
                );
            }
        }
    }
    Ok(())
}

pub async fn handle_why(app: &App, package: &str, out: Output) -> Result<()> {
    // Q9: `why nosuchbackend:foo` reported it "not under Shall management" at exit 0 — true of
    // the string and useless, because the manager is the part that does not exist.
    app.require_known_spec_backends(std::slice::from_ref(&package.to_string()))
        .await?;
    crate::app::insight::why(app, package, out)
        .await
        .map_err(|e| e.into())
}
