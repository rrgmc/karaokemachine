//! What can go wrong, and what the process exits with when it does.
//!
//! The exit codes are part of the interface, because `verify` is meant to run in CI: a threshold
//! change that makes a shipped background unreadable has to fail a build, and a build needs to tell
//! "the pack is wrong" apart from "the tool could not run".

use std::path::PathBuf;

/// The result type used throughout the tool.
pub type Result<T> = std::result::Result<T, Error>;

/// Anything that stops the tool.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The config file says something impossible.
    #[error("configuration: {0}")]
    Config(String),

    /// A file could not be read or written.
    #[error("{path}: {source}")]
    Io {
        /// What was being read or written.
        path: String,
        /// The underlying failure.
        source: std::io::Error,
    },

    /// A provider could not be reached, or answered with something unusable.
    #[error("{provider}: {message}")]
    Provider {
        /// Which provider.
        provider: &'static str,
        /// What happened.
        message: String,
    },

    /// An API key is not in the environment.
    #[error(
        "{var} is not set. A key is free from {provider}, and this tool never reads one from a config file."
    )]
    MissingKey {
        /// The environment variable.
        var: &'static str,
        /// Where to get a key.
        provider: &'static str,
    },

    /// An image could not be decoded or encoded.
    #[error("{path}: {message}")]
    Image {
        /// The file.
        path: PathBuf,
        /// What the codec said.
        message: String,
    },

    /// A built pack failed its own contrast gate.
    #[error("{count} image(s) in the pack fall below the contrast gate")]
    ContrastFailed {
        /// How many.
        count: usize,
    },

    /// The manifest and the files on disk disagree.
    #[error("the manifest and the pack disagree: {0}")]
    PackMismatch(String),
}

impl Error {
    /// A configuration complaint.
    pub fn config(message: impl Into<String>) -> Self {
        Self::Config(message.into())
    }

    /// A provider complaint.
    pub fn provider(provider: &'static str, message: impl Into<String>) -> Self {
        Self::Provider {
            provider,
            message: message.into(),
        }
    }

    /// The process exit code for this failure.
    ///
    /// `2` and `3` are the two a CI job wants to tell apart: an unreadable background is a *finding*
    /// about the pack, while a missing file is a broken run. Everything else is 1.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::ContrastFailed { .. } => 2,
            Self::PackMismatch(_) => 3,
            _ => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_findings_ci_cares_about_have_codes_of_their_own() {
        assert_eq!(Error::ContrastFailed { count: 3 }.exit_code(), 2);
        assert_eq!(Error::PackMismatch("gone".to_owned()).exit_code(), 3);
        assert_eq!(Error::config("nonsense").exit_code(), 1);
    }

    #[test]
    fn a_missing_key_says_where_to_get_one_and_never_suggests_the_config_file() {
        let error = Error::MissingKey {
            var: "PIXABAY_API_KEY",
            provider: "pixabay.com",
        };
        let message = error.to_string();
        assert!(message.contains("PIXABAY_API_KEY"), "{message}");
        assert!(message.contains("pixabay.com"), "{message}");
        assert!(
            message.contains("never reads one from a config file"),
            "{message}"
        );
    }
}
