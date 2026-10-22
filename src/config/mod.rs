//! Configuration: typed model, persistence, migrations, settings patches, exchange files and
//! the legacy XML format (SPEC §10–11).

pub mod exchange;
pub mod legacy;
pub mod migrate;
pub mod model;
pub mod patch;
pub mod store;

pub use model::Config;
pub use store::ConfigStore;

use crate::i18n::{t, t_args};

/// Why a config operation failed (SPEC §4.5): a stable [`code`](Self::code) and a message for
/// the user in the UI's language; the `Display` text is English, for the log.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Reading or writing a file failed.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// Text that is not JSON, or not of the config's shape.
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    /// No sensor has this id (it was removed meanwhile).
    #[error("no sensor {0}")]
    NotFound(String),
    /// No sensor type of this version has this id.
    #[error("unknown sensor type {0}")]
    UnknownType(String),
    /// Settings paths that `Settings` does not have.
    #[error("unknown settings: {}", .0.join(", "))]
    UnknownSettings(Vec<String>),
    /// Props that a sensor of type `kind` has not, or values of the wrong type or shape.
    #[error("invalid props for {kind}: {}", .keys.join(", "))]
    InvalidProps { kind: String, keys: Vec<String> },
    /// Any other value of the wrong type or shape; the reason is for the log.
    #[error("{0}")]
    Invalid(String),
}

impl ConfigError {
    /// The stable code (SPEC §4.5): `io`, `json`, `notFound`, `unknownType`,
    /// `unknownSettings` or `invalid`.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io(_) => "io",
            Self::Json(_) => "json",
            Self::NotFound(_) => "notFound",
            Self::UnknownType(_) => "unknownType",
            Self::UnknownSettings(_) => "unknownSettings",
            Self::InvalidProps { .. } | Self::Invalid(_) => "invalid",
        }
    }

    /// What went wrong, for the user (SPEC §13).
    pub fn message(&self) -> String {
        match self {
            Self::Io(e) => t_args("config.error.io", &[("reason", &e.to_string())]),
            Self::Json(_) => t("config.error.json"),
            Self::NotFound(_) => t("config.error.notFound"),
            Self::UnknownType(kind) => t_args("config.error.unknownType", &[("type", kind)]),
            Self::UnknownSettings(paths) => t_args(
                "config.error.unknownSettings",
                &[("paths", &paths.join(", "))],
            ),
            Self::InvalidProps { keys, .. } => {
                t_args("config.error.invalidProps", &[("keys", &keys.join(", "))])
            }
            Self::Invalid(_) => t("config.error.invalid"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_have_codes_and_messages() {
        let cases = [
            (ConfigError::Io(std::io::Error::other("disk full")), "io"),
            (
                ConfigError::Json(serde_json::from_str::<u32>("x").expect_err("not JSON")),
                "json",
            ),
            (ConfigError::NotFound("s_1".into()), "notFound"),
            (ConfigError::UnknownType("mail".into()), "unknownType"),
            (
                ConfigError::UnknownSettings(vec!["bar.x".into()]),
                "unknownSettings",
            ),
            (
                ConfigError::InvalidProps {
                    kind: "image".into(),
                    keys: vec!["opacity".into()],
                },
                "invalid",
            ),
            (ConfigError::Invalid("expected u32".into()), "invalid"),
        ];
        for (error, code) in cases {
            assert_eq!(error.code(), code);
            let message = error.message();
            assert!(!message.starts_with("config."), "{code}: {message}");
        }
        assert_eq!(
            ConfigError::UnknownSettings(vec!["bar.x".into(), "y".into()]).message(),
            "Unknown settings: bar.x, y."
        );
        assert_eq!(
            ConfigError::Io(std::io::Error::other("disk full")).message(),
            "The settings could not be read or written: disk full"
        );
    }
}
