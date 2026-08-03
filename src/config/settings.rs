//! LiNix's own settings — the one file that is not in your repo.
//!
//! It holds where your repo is, and nothing else. A key inside the repo saying where the repo
//! is would have to be read out of the file whose location it defines, and no ordering
//! resolves that. A key in a fixed location resolves it in one step: LiNix reads its own
//! settings from a place it always knows, learns the repo path, and everything after that is
//! the ordinary model.

use crate::core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The only key this file may hold. Enforced by the parser rather than by discipline: a file
/// holding exactly one key is the file that grows a second one, and the moment it does there
/// are two preference systems and a new question about which wins on every key either could
/// hold. The one key it holds is the one key `preferences.toml` structurally cannot.
const ONLY_KEY: &str = "config_root";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_root: Option<PathBuf>,
}

/// Which of the four sources answered "where is the repo". Carried rather than discarded so
/// `linix path` can say why the answer is what it is — a wrong answer is then debuggable in
/// one command instead of by elimination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootSource {
    Flag,
    Environment,
    SettingsFile,
    Default,
}

impl RootSource {
    pub fn describe(self) -> &'static str {
        match self {
            RootSource::Flag => "--config-dir",
            RootSource::Environment => "$LINIX_CONFIG_DIR",
            RootSource::SettingsFile => "your LiNix settings file",
            RootSource::Default => "the built-in default",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedRoot {
    pub path: PathBuf,
    pub source: RootSource,
}

impl Settings {
    /// The platform config directory, not the data directory: this is configuration, and
    /// putting it beside the data invites the assumption that deleting the data dir is safe.
    ///
    /// A file rather than `linix/settings.toml`, because **the default repo is
    /// `<config dir>/linix`** — so the obvious nested spelling puts this file inside the repo
    /// it exists to locate, where git would commit a machine-specific absolute path and a
    /// fleet would share one box's answer.
    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("linix.settings.toml")
    }

    pub fn load() -> Result<Self> {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            // No settings file is the normal case, not a failure: the default repo location
            // is what most machines use.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
            Err(e) => return Err(Error::Io(e.to_string())),
        };
        Self::parse(&text, path)
    }

    fn parse(text: &str, path: &Path) -> Result<Self> {
        let table: toml::Table = toml::from_str(crate::config::without_bom(text))
            .map_err(|e| Error::Config(format!("{} is not valid TOML: {}", path.display(), e)))?;

        for key in table.keys() {
            if key != ONLY_KEY {
                return Err(Error::Config(format!(
                    "`{}` is not allowed in {}.\n  This file holds `{}` and nothing else — \
                     where your repo is. Everything about how LiNix behaves goes in \
                     `preferences.toml`, inside the repo, where it is versioned with the \
                     config it describes.",
                    key,
                    path.display(),
                    ONLY_KEY
                )));
            }
        }

        let config_root = match table.get(ONLY_KEY) {
            None => None,
            Some(v) => {
                let raw = v.as_str().ok_or_else(|| {
                    Error::Config(format!(
                        "`{}` in {} must be a path in quotes.",
                        ONLY_KEY,
                        path.display()
                    ))
                })?;
                Some(absolute_or_refuse(
                    PathBuf::from(raw),
                    &format!("`{}` in {}", ONLY_KEY, path.display()),
                )?)
            }
        };

        Ok(Settings { config_root })
    }

    /// Returns whether the bytes reached the disk; a preview writes none.
    pub fn save(&self) -> Result<bool> {
        let text = toml::to_string_pretty(self).map_err(|e| Error::Toml(e.to_string()))?;
        crate::utils::file::persist(&Self::path(), &text)
    }
}

/// The one refusal, so every door that names a directory says the same sentence.
///
/// It said it in one place — `linix path --set` — and the other three doors said nothing, which
/// is how `--config-dir ./sandbox` came to read `preferences.toml` from the sandbox and
/// `modules/` from the real repo (AU2). `source` names the door, because "which of these four
/// did I get wrong" is the next question after "one of them is relative".
pub fn absolute_or_refuse(path: PathBuf, source: &str) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(Error::Config(format!(
            "{} is `{}`, which is not an absolute path.\n  A relative path would mean a \
             different directory depending on where you ran LiNix from.",
            source,
            path.display()
        )));
    }
    Ok(path)
}

/// Command line beats environment beats stored beats default.
///
/// `flag` is `--config-dir`. The environment variable is read here rather than by the caller
/// so that every path answering "where is the repo" answers it the same way.
///
/// **Fallible, because a relative answer is refused rather than replaced.** Discarding it and
/// carrying on is what inverted the documented precedence: the flag was dropped downstream and
/// `$LINIX_CONFIG_DIR` — which `--help` says the flag outranks — was picked back up in its
/// place, silently, by the fallback.
pub fn resolve_root(flag: Option<&Path>, settings: &Settings) -> Result<ResolvedRoot> {
    if let Some(path) = flag {
        return Ok(ResolvedRoot {
            path: absolute_or_refuse(path.to_path_buf(), "`--config-dir`")?,
            source: RootSource::Flag,
        });
    }
    if let Some(dir) = std::env::var_os("LINIX_CONFIG_DIR").filter(|v| !v.is_empty()) {
        return Ok(ResolvedRoot {
            path: absolute_or_refuse(PathBuf::from(dir), "`$LINIX_CONFIG_DIR`")?,
            source: RootSource::Environment,
        });
    }
    if let Some(stored) = &settings.config_root {
        return Ok(ResolvedRoot {
            path: stored.clone(),
            source: RootSource::SettingsFile,
        });
    }
    Ok(ResolvedRoot {
        path: crate::utils::safe_config_dir(),
        source: RootSource::Default,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(name: &str) -> PathBuf {
        PathBuf::from(name)
    }

    /// An absolute path on whichever platform the suite is running on. A unix-shaped
    /// literal is merely relative on Windows, which makes the assertion test nothing.
    fn absolute(tail: &str) -> String {
        if cfg!(windows) {
            format!("C:/{}", tail)
        } else {
            format!("/{}", tail)
        }
    }

    #[test]
    fn the_settings_file_is_never_inside_the_repo_it_locates() {
        // The default repo is `<config dir>/linix`. A settings file nested under it would be
        // committed to git and would carry one machine's absolute path to every other.
        let settings = Settings::path();
        let default_repo = crate::utils::safe_config_dir();
        assert!(
            !settings.starts_with(&default_repo),
            "{} is inside the default repo {}",
            settings.display(),
            default_repo.display()
        );
    }

    #[test]
    fn an_absent_settings_file_is_not_an_error() {
        let s = Settings::load_from(&at("definitely/not/here/settings.toml")).unwrap();
        assert!(s.config_root.is_none());
    }

    #[test]
    fn the_one_key_is_read() {
        let root = absolute("srv/linix");
        let s =
            Settings::parse(&format!("config_root = \"{}\"", root), &at("settings.toml")).unwrap();
        assert_eq!(s.config_root, Some(PathBuf::from(root)));
    }

    #[test]
    fn a_second_key_is_refused_and_names_where_it_belongs() {
        let err = Settings::parse(
            &format!(
                "config_root = \"{}\"\nverbose = true",
                absolute("srv/linix")
            ),
            &at("settings.toml"),
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("verbose"));
        assert!(
            msg.contains("preferences.toml"),
            "the refusal must say where the key does belong"
        );
    }

    #[test]
    fn a_relative_root_is_refused() {
        let err = Settings::parse("config_root = \"../linix\"", &at("settings.toml")).unwrap_err();
        assert!(err.to_string().contains("absolute"));
    }

    #[test]
    fn an_empty_settings_file_is_valid_and_means_the_default() {
        let s = Settings::parse("", &at("settings.toml")).unwrap();
        assert!(s.config_root.is_none());
    }

    #[test]
    fn the_flag_outranks_everything_stored() {
        let settings = Settings {
            config_root: Some(at(&absolute("stored"))),
        };
        let flagged = at(&absolute("flagged"));
        let resolved = resolve_root(Some(&flagged), &settings).unwrap();
        assert_eq!(resolved.path, flagged);
        assert_eq!(resolved.source, RootSource::Flag);
    }

    #[test]
    fn the_settings_file_outranks_the_default() {
        let stored = at(&absolute("stored"));
        let settings = Settings {
            config_root: Some(stored.clone()),
        };
        let resolved = resolve_root(None, &settings).unwrap();
        assert_eq!(resolved.path, stored);
        assert_eq!(resolved.source, RootSource::SettingsFile);
    }

    #[test]
    fn with_nothing_set_the_source_is_the_default() {
        let resolved = resolve_root(None, &Settings::default()).unwrap();
        assert_eq!(resolved.source, RootSource::Default);
    }

    #[test]
    fn every_source_can_say_what_it_was() {
        for source in [
            RootSource::Flag,
            RootSource::Environment,
            RootSource::SettingsFile,
            RootSource::Default,
        ] {
            assert!(!source.describe().is_empty());
        }
    }
}
