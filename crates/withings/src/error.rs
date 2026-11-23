//! Client errors. A rejected session is distinct from a transport failure so the runner
//! can tell "re-seed me" from "try again later".

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The session is no longer accepted: Withings returned a non-zero status on a call
    /// that should have worked. Only a human re-seed fixes this.
    #[error("the Withings session was rejected (status {0})")]
    Rejected(i64),
    /// Rate-limited; back off and retry.
    #[error("rate limited by Withings")]
    Throttled,
    /// A non-success HTTP status.
    #[error("Withings returned HTTP {0}")]
    Http(u16),
    /// A response that did not match the shape the parser expects.
    #[error("unexpected response shape: {0}")]
    Shape(String),
    /// A request the allow-list does not name — a bug, never reached in production.
    #[error("refused a request outside the read allow-list: {0} {1}")]
    NotAllowed(&'static str, String),
    /// Network or TLS failure.
    #[error("transport error: {0}")]
    Transport(String),
}
