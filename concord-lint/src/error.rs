use std::path::PathBuf;

/// Everything that can stop concord-lint before it reports violations.
#[derive(Debug, thiserror::Error)]
pub enum LintError {
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{}: {message}", path.display())]
    Parse { path: PathBuf, message: String },
    #[error(transparent)]
    Registry(#[from] concord_fields::ParseError),
    #[error("{0}")]
    BadArgs(String),
}
