//! Library errors.

use std::path::PathBuf;

/// Everything that makes `grover` fail.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no config found, expected: {}", .0.display())]
    ConfigMissing(PathBuf),
    #[error("{}: {source}", .path.display())]
    ConfigRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{}: {source}", .path.display())]
    ConfigParse {
        path: PathBuf,
        #[source]
        source: serde_yaml_ng::Error,
    },
    #[error("invalid config: {0}")]
    ConfigInvalid(String),
    #[error("cannot render config as YAML: {0}")]
    ConfigWrite(#[source] serde_yaml_ng::Error),
    #[error("no repo selected: pass `--repo <name>` or `--all`")]
    NoSelection,
    #[error("no repos found: check `repo_dirs`, `repos` and `discover`")]
    NoRepos,
    #[error("unknown name \"{query}\"; similar: {suggestions}")]
    UnknownName { query: String, suggestions: String },
    #[error("ambiguous name \"{query}\", candidates: {candidates}")]
    AmbiguousName { query: String, candidates: String },
    #[error("ticket \"{0}\" yields no directory name")]
    InvalidTicket(String),
    #[error("no ticket folder above {} in {}", .dir.display(), .work_dir.display())]
    NotInWorkDir { dir: PathBuf, work_dir: PathBuf },
    #[error("repo {}: none of the base refs \"{bases}\" exists", .repo.display())]
    BaseMissing { repo: PathBuf, bases: String },
    #[error("{}: {source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot run git: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("git {command}: {stderr}")]
    Git { command: String, stderr: String },
}

/// Result carrying [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
