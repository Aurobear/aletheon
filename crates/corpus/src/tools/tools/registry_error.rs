//! Corpus-owned tool registry error.

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("missing or invalid registry configuration: {0}")]
    Config(String),
    #[error("'{0}' already registered")]
    AlreadyExists(String),
    #[error("'{0}' not found")]
    NotFound(String),
}

impl RegistryError {
    pub fn config_missing(value: &str) -> Self {
        Self::Config(value.to_owned())
    }
    pub fn already_exists(value: &str) -> Self {
        Self::AlreadyExists(value.to_owned())
    }
    pub fn not_found(value: &str) -> Self {
        Self::NotFound(value.to_owned())
    }
}
