use thiserror::Error;

/// Workspace-wide error type for stage-0 scaffolding.
#[derive(Debug, Error)]
pub enum Error {
    #[error("configuration error: {0}")]
    Config(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_error_displays_message() {
        let err = Error::Config("missing listen".into());
        assert!(err.to_string().contains("missing listen"));
    }
}
