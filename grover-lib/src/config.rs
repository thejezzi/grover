//! Read, validate and print the config.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::repo::url_name;

/// Resolved config: every path absolute, `~` expanded.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Ticket directories are created here.
    pub work_dir: PathBuf,
    /// Branch name; `{ticket}` is replaced by the slug.
    #[serde(default = "default_branch_template")]
    pub branch_template: String,
    /// Base ref for new branches.
    #[serde(default = "default_base")]
    pub base: String,
    /// Directories whose child repos are discovered.
    #[serde(default)]
    pub repo_dirs: Vec<PathBuf>,
    /// Destination for clones from `repos`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clone_dir: Option<PathBuf>,
    /// Repos cloned only when selected.
    #[serde(default)]
    pub repos: Vec<RemoteRepo>,
    /// Files copied into the ticket directory.
    #[serde(default)]
    pub copy: Vec<CopySpec>,
}

/// One `repos` entry.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteRepo {
    pub url: String,
    /// Otherwise derived from the URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl RemoteRepo {
    /// Name for directory and fuzzy selection, `None` if unusable.
    pub fn resolved_name(&self) -> Option<String> {
        let name = self.name.clone().or_else(|| url_name(&self.url))?;
        (!name.trim().is_empty()).then_some(name)
    }
}

/// One `copy` entry; `to` is relative to the ticket directory, default is the file name.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CopySpec {
    pub from: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<PathBuf>,
}

fn default_branch_template() -> String {
    "ticket/{ticket}".to_string()
}

fn default_base() -> String {
    "HEAD".to_string()
}

impl Config {
    /// Location: `--config`, else `./grover.yaml`, else the XDG path.
    pub fn locate(explicit: Option<&Path>) -> Result<PathBuf> {
        let cwd =
            std::env::current_dir().map_err(|source| Error::Io { path: ".".into(), source })?;
        if let Some(path) = explicit {
            let path = absolute(path, &cwd);
            return if path.exists() { Ok(path) } else { Err(Error::ConfigMissing(path)) };
        }
        let local = cwd.join("grover.yaml");
        if local.exists() {
            return Ok(local);
        }
        let xdg = dirs::config_dir().map(|dir| dir.join("grover/config.yaml"));
        match &xdg {
            Some(path) if path.exists() => Ok(path.clone()),
            _ => Err(Error::ConfigMissing(xdg.unwrap_or(local))),
        }
    }

    /// Where `init` writes: `--config`, else an existing config, else the XDG path.
    pub fn target(explicit: Option<&Path>) -> Result<PathBuf> {
        if let Some(path) = explicit {
            let cwd =
                std::env::current_dir().map_err(|source| Error::Io { path: ".".into(), source })?;
            return Ok(absolute(path, &cwd));
        }
        if let Ok(path) = Config::locate(None) {
            return Ok(path);
        }
        match dirs::config_dir() {
            Some(dir) => Ok(dir.join("grover/config.yaml")),
            None => Err(Error::ConfigInvalid("no config directory (HOME unset)".into())),
        }
    }

    /// Read, resolve paths, validate.
    pub fn load(explicit: Option<&Path>) -> Result<Config> {
        let path = Config::locate(explicit)?;
        let text = fs::read_to_string(&path)
            .map_err(|source| Error::ConfigRead { path: path.clone(), source })?;
        let mut config: Config = serde_yaml_ng::from_str(&text)
            .map_err(|source| Error::ConfigParse { path: path.clone(), source })?;
        let base = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        config.resolve(&base)?;
        config.validate()?;
        Ok(config)
    }

    /// Write the resolved config as YAML, creating the parent directory.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|source| Error::Io { path: parent.to_path_buf(), source })?;
        }
        let yaml = self.to_yaml()?;
        fs::write(path, yaml).map_err(|source| Error::Io { path: path.to_path_buf(), source })
    }

    /// Resolved config as YAML.
    pub fn to_yaml(&self) -> Result<String> {
        serde_yaml_ng::to_string(self).map_err(Error::ConfigWrite)
    }

    /// Relative paths are relative to the config file, `~` is expanded.
    fn resolve(&mut self, base: &Path) -> Result<()> {
        self.work_dir = expand(&self.work_dir, base)?;
        self.repo_dirs =
            self.repo_dirs.iter().map(|dir| expand(dir, base)).collect::<Result<_>>()?;
        self.clone_dir = match self.clone_dir.take() {
            Some(dir) => Some(expand(&dir, base)?),
            None => dirs::home_dir().map(|home| home.join(".grover/repos")),
        };
        for copy in &mut self.copy {
            copy.from = expand(&copy.from, base)?;
        }
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        if !self.branch_template.contains("{ticket}") {
            return Err(Error::ConfigInvalid(format!(
                "branch_template {:?} has no {{ticket}}",
                self.branch_template
            )));
        }
        if self.base.split('|').all(|base| base.trim().is_empty()) {
            return Err(Error::ConfigInvalid("`base` has no ref".into()));
        }
        if self.repo_dirs.is_empty() && self.repos.is_empty() {
            return Err(Error::ConfigInvalid("neither `repo_dirs` nor `repos` set".into()));
        }
        if !self.repos.is_empty() && self.clone_dir.is_none() {
            return Err(Error::ConfigInvalid(
                "`clone_dir` missing and no home directory found".into(),
            ));
        }
        for repo in &self.repos {
            if repo.resolved_name().is_none() {
                return Err(Error::ConfigInvalid(format!(
                    "`repos`: cannot derive a name from {:?}",
                    repo.url
                )));
            }
        }
        for copy in &self.copy {
            if copy.from.as_os_str().is_empty() {
                return Err(Error::ConfigInvalid("`copy`: `from` is empty".into()));
            }
        }
        Ok(())
    }
}

fn absolute(path: &Path, cwd: &Path) -> PathBuf {
    if path.is_absolute() { path.to_path_buf() } else { cwd.join(path) }
}

/// Expand `~` and join relative paths onto `base`.
fn expand(raw: &Path, base: &Path) -> Result<PathBuf> {
    let expanded = if raw.starts_with("~") {
        let home = dirs::home_dir().ok_or_else(|| {
            Error::ConfigInvalid(format!("{}: no home directory for `~`", raw.display()))
        })?;
        let rest: PathBuf = raw.components().skip(1).collect();
        home.join(rest)
    } else {
        raw.to_path_buf()
    };
    Ok(if expanded.is_absolute() { expanded } else { base.join(expanded) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, text: &str) -> PathBuf {
        let path = dir.join("grover.yaml");
        fs::write(&path, text).expect("write config");
        path
    }

    #[test]
    fn branch_and_base_defaults() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = write(tmp.path(), "work_dir: tickets\nrepo_dirs: [code]\n");
        let config = Config::load(Some(&path)).expect("config");
        assert_eq!(config.branch_template, "ticket/{ticket}");
        assert_eq!(config.base, "HEAD");
        assert_eq!(config.work_dir, tmp.path().join("tickets"));
        assert_eq!(config.repo_dirs, vec![tmp.path().join("code")]);
    }

    #[test]
    fn base_without_ref_is_an_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = write(tmp.path(), "work_dir: tickets\nrepo_dirs: [code]\nbase: \" | \"\n");
        let err = Config::load(Some(&path)).expect_err("empty base");
        assert!(err.to_string().contains("`base`"), "{err}");
    }

    #[test]
    fn unknown_field_is_an_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = write(tmp.path(), "work_dir: tickets\nrepos_dirs: []\n");
        assert!(Config::load(Some(&path)).is_err());
    }

    #[test]
    fn template_without_ticket_token_is_an_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path =
            write(tmp.path(), "work_dir: tickets\nbranch_template: feature\nrepo_dirs: [code]\n");
        let err = Config::load(Some(&path)).expect_err("validation");
        assert!(err.to_string().contains("{ticket}"), "{err}");
    }

    #[test]
    fn tilde_becomes_home() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = write(tmp.path(), "work_dir: ~/tickets\nrepo_dirs: [~/code]\n");
        let config = Config::load(Some(&path)).expect("config");
        let home = dirs::home_dir().expect("HOME");
        assert_eq!(config.work_dir, home.join("tickets"));
        assert_eq!(config.clone_dir, Some(home.join(".grover/repos")));
    }

    #[test]
    fn missing_config_path_reports_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let missing = tmp.path().join("fehlt.yaml");
        let err = Config::load(Some(&missing)).expect_err("missing");
        assert!(err.to_string().contains("fehlt.yaml"), "{err}");
    }

    #[test]
    fn yaml_lists_are_read() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = write(
            tmp.path(),
            "work_dir: tickets\nrepos:\n  - url: git@github.com:org/api.git\n    name: api\n\
             copy:\n  - from: tpl/AGENTS.md\n    to: AGENTS.md\n",
        );
        let config = Config::load(Some(&path)).expect("config");
        assert_eq!(config.repos.len(), 1);
        assert_eq!(config.repos[0].resolved_name().as_deref(), Some("api"));
        assert_eq!(config.copy[0].from, tmp.path().join("tpl/AGENTS.md"));
    }

    #[test]
    fn example_config_loads() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = write(tmp.path(), include_str!("../../grover.yaml.example"));
        Config::load(Some(&path)).expect("example config");
    }

    #[test]
    fn save_and_load_round_trip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = write(tmp.path(), "work_dir: tickets\nrepo_dirs: [code]\n");
        let config = Config::load(Some(&path)).expect("config");
        let target = tmp.path().join("out/config.yaml");
        config.save(&target).expect("save");
        let reloaded = Config::load(Some(&target)).expect("reload");
        assert_eq!(reloaded.to_yaml().expect("yaml"), config.to_yaml().expect("yaml"));
    }

    #[test]
    fn init_target_keeps_an_explicit_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("fehlt.yaml");
        assert_eq!(Config::target(Some(&path)).expect("target"), path);
    }
}
