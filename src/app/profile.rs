use crate::app::sync::{ChangePlanner, PlanScope, StateResolver, SyncEngine};
use crate::app::vocab::Vocab;
use crate::app::Machinery;
use crate::config::grammar::Origin;
use crate::config::parser::HostFacts;
use crate::core::{Error, Result};
use crate::model::profiles::{
    blocks_in_active, describe_gate, parse_active, read_active, remove_from_active, ProfileLoader,
};
use crate::model::Layout;
use tracing::{info, instrument};

/// Turns profiles on and off (SPEC II.6).
///
/// **Only profiles can be activated**, and activating one edits exactly one file: `active`,
/// a plain list of profile names. Nothing is materialised, because a materialised copy is a
/// second place the same fact lives, and the day it disagrees with your files it wins
/// silently (P4). The resolver reads `active` on every run and composes from there.
pub struct ProfileManager {
    /// What it takes to converge: activating a profile is a full sync, and it spends the same
    /// ceilings — including the command's removal budget — as any other.
    m: Machinery,
    layout: Layout,
}

impl ProfileManager {
    pub fn new(m: Machinery) -> Self {
        let layout = m.config.layout();
        Self { m, layout }
    }

    /// What this machine is, plus this run's variables — so a `when $role == travel` block
    /// in `active` is one these verbs can read rather than an unknown key (W8).
    async fn facts(&self) -> Result<HostFacts> {
        StateResolver::new(&self.m.config, self.m.registry.clone(), false)
            .await
            .facts_for_host()
            .await
    }

    async fn vocab(&self) -> Result<Vocab> {
        let priority = StateResolver::new(&self.m.config, self.m.registry.clone(), false)
            .await
            .priority_for_host()
            .await?;
        Ok(Vocab::new(&self.m.registry, &self.m.config, &priority))
    }

    /// `activate NAME…` — **`active` becomes exactly this list** (II.6).
    ///
    /// It is the set form, so it sets: `when` blocks in the file are overwritten with the
    /// rest. That is not an oversight and gets no extra refusal — a set form that quietly
    /// kept part of the old file would leave the machine in a state you did not type, and
    /// the file is in git (V.44).
    #[instrument(skip(self))]
    pub async fn activate(&self, names: &[String], add: bool) -> Result<()> {
        // `shall activate $PROFILE` with `$PROFILE` unset would otherwise read as "turn
        // everything off" and be perfectly valid. The guard would catch the removals, but
        // the guard is for decisions you meant, and nobody means this one (V.44).
        if names.is_empty() {
            return Err(Error::Config(
                "activate needs a profile name. To turn everything off, edit `active` \
                 yourself."
                    .into(),
            ));
        }
        for name in names {
            self.must_exist(name).await?;
        }

        if add {
            return self.add_to_active(names).await;
        }

        // The set form sets, blocks included — but automatic and silent are different
        // things (S6), and only one of them is a decision you get to review afterwards.
        let file = self.layout.active_file();
        let old = tokio::fs::read_to_string(&file).await.unwrap_or_default();
        let dropped = blocks_in_active(&old);
        let facts = self.facts().await?;

        if !self.write_active(names).await? {
            // Returning here rather than falling through to `sync_now`: with the write
            // previewed, a sync would converge to the profile that is *still* active and
            // report a plan for the wrong question.
            crate::would!(
                "would set active to {} ({} block(s) would be dropped), then sync.",
                names.join(", "),
                dropped.len()
            );
            return Ok(());
        }
        info!("active is now {}.", names.join(", "));
        for b in &dropped {
            info!(
                "Removed the {} block on line {}.",
                describe_gate(&b.predicate, &facts),
                b.line
            );
        }
        self.sync_now().await
    }

    /// `activate -a NAME…` — add to the list, leaving the rest of the file alone.
    async fn add_to_active(&self, names: &[String]) -> Result<()> {
        let file = self.layout.active_file();
        let body = tokio::fs::read_to_string(&file).await.unwrap_or_default();
        let facts = self.facts().await?;
        let entries = read_active(&file, &body, &facts)?;

        let mut added: Vec<String> = Vec::new();
        for name in names {
            match entries.iter().find(|e| &e.name == name) {
                // Not an error: the end state is what was asked for (II.6).
                Some(e) => match &e.gate {
                    Some(pred) => info!(
                        "{} is already listed, inside {} on line {}.",
                        name,
                        describe_gate(pred, &facts),
                        e.line
                    ),
                    None => info!("{} is already active.", name),
                },
                None => added.push(name.clone()),
            }
        }
        if added.is_empty() {
            return Ok(());
        }

        // Appended at the top level, never inside a `when` block: a block is something you
        // wrote, and Shall does not edit it (II.6).
        let mut body = body;
        if !body.is_empty() && !body.ends_with('\n') {
            body.push('\n');
        }
        for name in &added {
            body.push_str(name);
            body.push('\n');
        }
        if !crate::utils::file::persist(&file, &body)? {
            crate::would!("would add {} to active.", added.join(", "));
            return Ok(());
        }

        let now = self.active_profiles().await?;
        info!(
            "Added {}. active is now {}.",
            added.join(", "),
            now.join(", ")
        );
        self.sync_now().await
    }

    /// `deactivate NAME…` — take names out of the list, then converge.
    ///
    /// "Deactivate" has to mean it: the name goes from the top level **and** from every
    /// `when` block that applies to this host (II.6). A block that does not apply here is
    /// activating nothing, so it is left alone and said so — `active` is committed and
    /// shared, and reaching into another host's arm changes a machine nobody is sitting at.
    #[instrument(skip(self))]
    pub async fn deactivate(&self, names: &[String]) -> Result<()> {
        let file = self.layout.active_file();
        let body = tokio::fs::read_to_string(&file).await.unwrap_or_default();
        let facts = self.facts().await?;
        let edit = remove_from_active(&file, &body, names, &facts)?;

        for r in &edit.removed {
            match &r.gate {
                Some(pred) => info!(
                    "Removed {} from the {} block on line {}.",
                    r.name,
                    describe_gate(pred, &facts),
                    r.line
                ),
                None => info!("Removed {}.", r.name),
            }
        }
        for b in &edit.emptied {
            info!(
                "Removed the now-empty {} block on line {}.",
                describe_gate(&b.predicate, &facts),
                b.line
            );
        }
        for e in &edit.elsewhere {
            info!(
                "{} is not active on this host. `active` line {} activates it {} — edit \
                 that by hand if you meant every machine.",
                e.name,
                e.line,
                describe_gate(&e.predicate, &facts)
            );
        }
        // Not an error: the end state is what was asked for (II.6).
        for name in &edit.absent {
            info!("{} was not active.", name);
        }

        if !edit.changed() {
            return Ok(());
        }

        if !crate::utils::file::persist(&file, &edit.body)? {
            crate::would!("would deactivate {}.", names.join(", "));
            return Ok(());
        }

        let now = self.active_profiles().await?;
        info!(
            "active is now {}.",
            if now.is_empty() {
                "empty".to_string()
            } else {
                now.join(", ")
            }
        );
        self.sync_now().await
    }

    pub async fn list_profiles(&self) -> Result<Vec<String>> {
        let vocab = self.vocab().await?;
        Ok(ProfileLoader::new(&self.layout, &vocab).available())
    }

    /// The currently-active profiles, in the order `active` lists them.
    pub async fn active_profiles(&self) -> Result<Vec<String>> {
        let file = self.layout.active_file();
        let body = tokio::fs::read_to_string(&file).await.unwrap_or_default();
        Ok(parse_active(&file, &body, &self.facts().await?)?)
    }

    /// What a profile expands to, as `backend:name` lines.
    ///
    /// Resolved by the same code that resolves a sync, so what this prints and what a sync
    /// does cannot drift apart. It reports the profile as if it alone were active.
    ///
    /// **"As if" is now literal, and it used to be a pair of writes.** This wrote the profile's
    /// name into the machine's real `active` file, resolved, and wrote the old contents back —
    /// a read-only command changing what the machine is set to, for the length of a resolve
    /// that reads every profile and module and probes bare names over the network. A `^C` in
    /// that window, a panic, or a second write that failed left `active` pointing at a profile
    /// the user had asked only to look at, and the next `sync` converged to it. Nothing in the
    /// output would have said so.
    ///
    /// `as_if_active` is the whole fix: the resolver takes the body it would have read.
    pub async fn show(&self, name: &str) -> Result<Vec<String>> {
        self.must_exist(name).await?;

        let mut out: Vec<String> =
            StateResolver::new(&self.m.config, self.m.registry.clone(), false)
                .await
                .as_if_active(active_body(&[name.to_string()]))
                .resolve_desired_state()
                .await?
                .values()
                .flatten()
                .filter(|s| s.present)
                .map(|s| format!("{}:{}", s.backend, s.name))
                .collect();
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// Scaffold an empty profile.
    pub async fn create(&self, name: &str) -> Result<()> {
        self.check_name(name)?;
        let path = self.layout.profile_file(name);
        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return Err(Error::Config(format!(
                "Profile '{}' already exists at {}. Delete it first, or edit it.",
                name,
                path.display()
            )));
        }
        if crate::core::dry_run::active() {
            crate::would!("would create profile '{}' at {}.", name, path.display());
            return Ok(());
        }
        tokio::fs::create_dir_all(self.layout.profiles_dir())
            .await
            .ok();
        crate::utils::file::persist(&path, PROFILE_TEMPLATE)?;
        info!("Created profile '{}' at {}.", name, path.display());
        Ok(())
    }

    /// Snapshot what this machine currently wants into a new profile.
    pub async fn save_current_as(&self, name: &str) -> Result<()> {
        self.check_name(name)?;
        let desired = StateResolver::new(&self.m.config, self.m.registry.clone(), false)
            .await
            .resolve_desired_state()
            .await?;

        let mut lines: Vec<String> = desired
            .values()
            .flatten()
            .filter(|s| s.present)
            .map(|s| format!("{}:{}", s.backend, s.name))
            .collect();
        lines.sort();
        lines.dedup();

        if !crate::core::dry_run::active() {
            tokio::fs::create_dir_all(self.layout.profiles_dir())
                .await
                .ok();
        }
        let path = self.layout.profile_file(name);
        let body = format!(
            "# Profile '{name}' — what this machine wanted when it was saved.\n\
             #\n\
             # These are package lines held directly by the profile, so no module can reach\n\
             # them. Move them into a module to share them between profiles.\n\n{}\n",
            lines.join("\n")
        );
        if !crate::utils::file::persist(&path, &body)? {
            crate::would!(
                "would save profile '{}' with {} package(s) to {}.",
                name,
                lines.len(),
                path.display()
            );
            return Ok(());
        }
        info!(
            "Saved profile '{}' with {} package(s) to {}.",
            name,
            lines.len(),
            path.display()
        );
        Ok(())
    }

    /// II.5: profiles are Capitalized. A lowercase name would mint a module.
    fn check_name(&self, name: &str) -> Result<()> {
        if name.chars().next().is_some_and(char::is_uppercase) {
            return Ok(());
        }
        Err(Error::Config(format!(
            "`{}` is not a profile name — profiles are Capitalized, modules are lowercase.\n  \
             Did you mean `{}`? Only profiles can be activated.",
            name,
            capitalize(name)
        )))
    }

    async fn must_exist(&self, name: &str) -> Result<()> {
        self.check_name(name)?;
        let vocab = self.vocab().await?;
        let loader = ProfileLoader::new(&self.layout, &vocab);
        if loader.exists(name) {
            return Ok(());
        }
        // II.5's error teaches the rule rather than just saying no.
        let err = loader
            .resolve(
                name,
                &Origin::argument(),
                &self.facts().await?,
                &mut Vec::new(),
                &Vec::new(),
            )
            .unwrap_err();
        Err(err.into())
    }

    /// `active` is a plain list of profile names and nothing else goes in it (II.6).
    /// The one write behind `activate`, `activate -a` and `deactivate`.
    ///
    /// It goes through `persist` rather than `tokio::fs::write` because `active` decides
    /// which modules are in the model, and therefore what the next `sync` installs and removes.
    /// A preview of "what would switching to Work do" that leaves you on Work has answered a
    /// question nobody asked, and until 2026-07-28 it did so without printing a line.
    async fn write_active(&self, active: &[String]) -> Result<bool> {
        if !crate::core::dry_run::active() {
            tokio::fs::create_dir_all(self.layout.config_root())
                .await
                .ok();
        }
        crate::utils::file::persist(&self.layout.active_file(), &active_body(active))
    }

    /// Converge to whatever `active` now says.
    async fn sync_now(&self) -> Result<()> {
        let engine = SyncEngine::new(self.m.clone());

        let resolver = StateResolver::new(&self.m.config, self.m.registry.clone(), false).await;
        let desired = resolver.resolve_desired_state().await?;
        // Activating a profile is a full converge — the whole config is the desired set — so it
        // reaps, and reaps only what `priority` names. It used to reap every backend on the box:
        // `sync` confined removals to the managers this host lists and `activate` did not, which
        // made the narrower-sounding command the more destructive one.
        let hosts = resolver.host_backends().await;

        let changes = {
            let state_guard = self.m.state.lock().await;
            let planner = ChangePlanner::new(self.m.registry.clone(), &state_guard, &self.m.config);
            planner.plan(&desired, PlanScope::Whole(hosts)).await?
        };

        if changes.is_empty() {
            info!("This machine already matches the active profiles.");
            return Ok(());
        }

        engine
            .sync(changes, crate::app::sync::guard::GuardScope::Sync)
            .await?;
        Ok(())
    }
}

fn capitalize(name: &str) -> String {
    let mut c = name.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

const PROFILE_TEMPLATE: &str = "\
# A profile chooses; modules hold. Bring a module in with `use`:
#
#   use editors
#   use Work            (a profile — profiles are Capitalized, modules are lowercase)
#
# You can write packages here directly, but no module can reach them:
#
#   apt:curl
#   ripgrep             (no backend named — Shall asks each one in `priority` order)
#
# Set math, if you need it:
#
#   exclude heavy       (drop that module's packages)
#   intersect approved  (keep only what is also in there)
#   -steam              (drop one package)
#   (Work | gaming) & security
#
# Gather, then narrow, then subtract — subtraction always wins.
";

/// `active`'s body for a set of profile names: one per line, and empty when the set is.
fn active_body(active: &[String]) -> String {
    if active.is_empty() {
        String::new()
    } else {
        format!("{}\n", active.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::capitalize;

    #[test]
    fn the_suggestion_capitalizes_the_name_you_typed() {
        assert_eq!(capitalize("work"), "Work");
        assert_eq!(capitalize(""), "");
    }
}
