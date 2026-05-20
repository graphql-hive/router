#[derive(Debug, thiserror::Error)]
pub enum RequestContextError {
    #[error("request context is missing")]
    Missing,
    #[error("request context lock is poisoned")]
    LockPoison,
    #[error("unknown reserved request-context key: {key}")]
    UnknownReservedKey { key: String },
    #[error("request-context key '{key}' cannot be mutated externally")]
    ForbiddenReservedMutation { key: String },
    #[error("reserved request-context key '{key}' has an invalid type: expected {expected}")]
    ReservedKeyTypeMismatch { key: String, expected: &'static str },
    #[error("invalid operation kind: {value}")]
    InvalidOperationKind { value: String },
    #[error("reserved prefix in custom key: {key}")]
    ReservedPrefixInCustomKey { key: String },
    #[error("json error: {0}")]
    Json(sonic_rs::Error),
}
