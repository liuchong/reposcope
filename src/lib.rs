pub mod cli;
pub mod contributors;
pub mod github;
pub mod star_history;
pub mod svg;

use thiserror::Error;

/// Top-level error type. Maps to process exit codes in `main`.
#[derive(Debug, Error)]
pub enum ScopeError {
    /// Usage / configuration problems (bad flags, missing repo, invalid output).
    #[error("{0}")]
    Usage(String),
    /// GitHub API failures after retries.
    #[error("github api: {0}")]
    Api(String),
    /// Rendering / output-validation failures.
    #[error("render: {0}")]
    Render(String),
}

pub type Result<T> = std::result::Result<T, ScopeError>;
