//! A small error type and context helpers.
//!
//! The service deliberately has no error-handling crate: every failure that reaches a caller is a
//! message that can be put in a JSON `error` field or printed by the CLI.

use std::fmt;

/// The service error: a message, optionally given context by the call site.
#[derive(Debug, Clone)]
pub struct Error {
    message: String,
}

impl Error {
    /// Build an error from anything printable.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The message without the wrapper.
    #[cfg(test)]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::new(error.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::new(error.to_string())
    }
}

impl From<std::str::Utf8Error> for Error {
    fn from(error: std::str::Utf8Error) -> Self {
        Self::new(error.to_string())
    }
}

/// Convenient `Result` alias for the service.
pub type Result<T> = std::result::Result<T, Error>;

/// Add a human-readable context to any displayable error.
///
/// This is a blanket implementation so both `io::Error` and `serde_json::Error` (and our own
/// `Error`) get the same behavior without a third-party `anyhow`.
pub trait ResultExt<T> {
    /// Prefix the error with `context`.
    fn context(self, context: impl Into<String>) -> Result<T>;
}

impl<T, E: fmt::Display> ResultExt<T> for std::result::Result<T, E> {
    fn context(self, context: impl Into<String>) -> Result<T> {
        self.map_err(|error| Error::new(format!("{}: {error}", context.into())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_keeps_both_parts() {
        let error = Err::<(), _>(std::io::Error::other("disk"))
            .context("open records")
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert_eq!(error, "open records: disk");
    }
}
