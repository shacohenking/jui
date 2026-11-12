//! The core error type. See ticket 13 "Jira errors in the CLI and the TUI".

use std::collections::BTreeMap;
use std::path::PathBuf;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("JIRA_TOKEN is not set.")]
    NoToken,

    #[error("Jira did not accept the token (HTTP 401).")]
    Unauthorized,

    #[error("You do not have permission: {0}")]
    Forbidden(String),

    #[error("{0} not found.")]
    NotFound(String),

    #[error("{}", validation_message(.messages))]
    Validation {
        /// Field ID (or name, when known) to message.
        fields: BTreeMap<String, String>,
        /// General messages (`errorMessages`).
        messages: Vec<String>,
    },

    #[error("{0}")]
    Conflict(String),

    #[error("Jira limits the request rate (HTTP 429).")]
    RateLimited { retry_after_secs: Option<u64> },

    #[error("Jira server error (HTTP {status}): {body}")]
    Server { status: u16, body: String },

    #[error("Cannot connect to {host}: {cause}")]
    Network { host: String, cause: String },

    #[error("No response from Jira after {0} s.")]
    Timeout(u64),

    #[error("Unexpected response from {endpoint}: {message}")]
    Decode { endpoint: String, message: String },

    #[error("The config file {} is not valid: {message}", .path.display())]
    Settings { path: PathBuf, message: String },

    #[error("{message}")]
    Input { message: String, hint: Option<String> },

    #[error("Cancelled.")]
    Cancelled,
}

fn validation_message(messages: &[String]) -> String {
    if messages.is_empty() {
        "Jira did not accept the request.".to_string()
    } else {
        messages.join(" ")
    }
}

/// A stable, machine-readable error kind name (used in `--json` output).
impl Error {
    pub fn kind(&self) -> &'static str {
        match self {
            Error::NoToken => "no_token",
            Error::Unauthorized => "unauthorized",
            Error::Forbidden(_) => "forbidden",
            Error::NotFound(_) => "not_found",
            Error::Validation { .. } => "validation",
            Error::Conflict(_) => "conflict",
            Error::RateLimited { .. } => "rate_limited",
            Error::Server { .. } => "server",
            Error::Network { .. } => "network",
            Error::Timeout(_) => "timeout",
            Error::Decode { .. } => "decode",
            Error::Settings { .. } => "settings",
            Error::Input { .. } => "input",
            Error::Cancelled => "cancelled",
        }
    }

    /// The process exit code for this error kind.
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Cancelled => 0,
            Error::Input { .. } => 2,
            Error::NoToken | Error::Unauthorized | Error::Forbidden(_) => 3,
            Error::NotFound(_) => 4,
            Error::Validation { .. } => 5,
            Error::Network { .. } | Error::Timeout(_) => 6,
            Error::Settings { .. } => 7,
            Error::Server { .. } | Error::Decode { .. } | Error::RateLimited { .. } | Error::Conflict(_) => 1,
        }
    }

    /// A hint that tells the user what to do next.
    pub fn hint(&self) -> Option<String> {
        match self {
            Error::NoToken => Some(
                "Make a Personal Access Token in Jira (Profile > Personal Access Tokens) and export JIRA_TOKEN."
                    .into(),
            ),
            Error::Unauthorized => Some("The token can be expired. Make a new one.".into()),
            Error::NotFound(_) => Some("Check the key, or check that you can see it in Jira.".into()),
            Error::Timeout(_) => Some("Change jira.timeout_secs, or try again.".into()),
            Error::Network { .. } => Some("Check the VPN.".into()),
            Error::RateLimited { .. } => Some("Wait and try again.".into()),
            Error::Settings { .. } => Some("Use `jui config edit` to correct the file, or `jui config path` to find it.".into()),
            Error::Input { hint, .. } => hint.clone(),
            _ => None,
        }
    }

    pub fn input(message: impl Into<String>) -> Self {
        Error::Input { message: message.into(), hint: None }
    }

    pub fn input_hint(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Error::Input { message: message.into(), hint: Some(hint.into()) }
    }

    /// True when a GET request can be sent again after this error.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Error::Network { .. }
                | Error::Timeout(_)
                | Error::RateLimited { .. }
                | Error::Server { status: 502..=504, .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_fixed() {
        assert_eq!(Error::Cancelled.exit_code(), 0);
        assert_eq!(Error::input("x").exit_code(), 2);
        assert_eq!(Error::NoToken.exit_code(), 3);
        assert_eq!(Error::Unauthorized.exit_code(), 3);
        assert_eq!(Error::NotFound("Ticket X-1".into()).exit_code(), 4);
        assert_eq!(Error::Validation { fields: Default::default(), messages: vec![] }.exit_code(), 5);
        assert_eq!(Error::Timeout(30).exit_code(), 6);
        assert_eq!(Error::Settings { path: "x".into(), message: "y".into() }.exit_code(), 7);
        assert_eq!(Error::Server { status: 500, body: String::new() }.exit_code(), 1);
    }

    #[test]
    fn only_some_server_errors_are_retryable() {
        assert!(Error::Server { status: 503, body: String::new() }.is_retryable());
        assert!(!Error::Server { status: 500, body: String::new() }.is_retryable());
        assert!(!Error::Unauthorized.is_retryable());
    }
}
