#[derive(Debug, Clone, thiserror::Error)]
pub enum FragmentExpansionError {
    #[error("Could not find fragment definition '{0}'")]
    MissingFragment(String),
    #[error("Detected cycle while expanding fragment spread '{0}'")]
    Cycle(String),
}
