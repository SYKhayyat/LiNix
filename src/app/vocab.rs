use crate::backends::BackendRegistry;
use crate::config::grammar::BackendNames;
use crate::config::Config;
use crate::model::Priority;
use std::collections::HashSet;

/// Which `prefix:` names a backend, for the grammar.
///
/// One vocabulary, owned rather than borrowed, because every path that reads a line must
/// agree with every path that writes one: if `apt:jq` parses as a backend when resolving and
/// as a package name when editing, `uninstall` silently fails to find the line `install`
/// wrote.
///
/// Three sources feed it. The registry knows what is compiled in and what the onboarder
/// added; `aliases` renames them; and `priority` names what this setup uses — including a
/// backend this OS does not build, which must still parse so that `priority` is what refuses
/// it (V.15) rather than a baffling "unrecognised line".
#[derive(Debug, Clone, Default)]
pub struct Vocab {
    names: HashSet<String>,
}

impl Vocab {
    pub fn new(registry: &BackendRegistry, config: &Config, priority: &Priority) -> Self {
        let mut names: HashSet<String> = registry
            .all()
            .iter()
            .map(|b| b.name().to_string())
            .collect();
        names.extend(config.aliases.keys().cloned());
        names.extend(priority.order().iter().cloned());
        Self { names }
    }

    /// For paths with no `priority` to hand. Anything `priority` would have added is
    /// missing, so use it only where a backend this OS does not build cannot appear.
    pub fn without_priority(registry: &BackendRegistry, config: &Config) -> Self {
        Self::new(registry, config, &Priority::default())
    }
}

impl BackendNames for Vocab {
    fn is_backend(&self, name: &str) -> bool {
        self.names.contains(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_names_a_backend_this_os_does_not_build() {
        // On Windows nothing registers `apt`, so without this the grammar calls `apt:curl`
        // an unrecognised line — and V.15's "apt isn't in your priority list" never fires.
        let cfg = Config::default();
        let reg = BackendRegistry::new();
        let priority = Priority::from_backends(vec!["apt".into()]);
        assert!(Vocab::new(&reg, &cfg, &priority).is_backend("apt"));
        assert!(!Vocab::new(&reg, &cfg, &priority).is_backend("nonsense"));
    }

    #[test]
    fn an_alias_is_a_backend_name() {
        let mut cfg = Config::default();
        cfg.aliases.insert("pkg".into(), "apt".into());
        let reg = BackendRegistry::new();
        assert!(Vocab::without_priority(&reg, &cfg).is_backend("pkg"));
    }
}
