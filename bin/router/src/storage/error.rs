#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("failed to configure storage runtime: {0}")]
    Configuration(String),
    #[error("storage failure: {0}")]
    Store(#[from] object_store::Error),
    #[error("failed to format contents: {0}")]
    Format(#[from] std::string::FromUtf8Error),
}
