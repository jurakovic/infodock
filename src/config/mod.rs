//! Configuration: typed model, persistence, migrations and settings patches (SPEC §10–11).

pub mod migrate;
pub mod model;
pub mod patch;
pub mod store;

pub use model::Config;
pub use store::ConfigStore;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
}
