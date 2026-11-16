use crate::query_planner::{
    ast::fragment_expansion::FragmentExpansionError, graph::error::GraphError,
    utils::cancellation::CancellationError,
};

use super::{fetch::error::FetchGraphError, plan_nodes::FetchId};

#[derive(Debug, Clone, thiserror::Error)]
pub enum QueryPlanError {
    #[error("FetchGraph error: {0}")]
    FetchGraphFailure(#[from] FetchGraphError),
    #[error("Graph error: {0}")]
    GraphFailure(#[from] GraphError),
    #[error("Root fetch is missing")]
    NoRoot,
    #[error("Failed to build a plan")]
    EmptyPlan,
    #[error("Query planning finished with unresolved pending steps")]
    UnexpectedPendingState,
    #[error("Internal Error: {0}")]
    Internal(String),
    #[error("Fetch {fetch} depends on fetch {dependency}, which doesn't come before it")]
    DependsOnLaterFetch { fetch: FetchId, dependency: FetchId },
    #[error("Two fetches have id {id}")]
    DuplicateFetchId { id: FetchId },
    #[error("Fragment expansion error: {0}")]
    FragmentExpansionFailure(#[from] FragmentExpansionError),
    #[error(transparent)]
    CancellationError(#[from] CancellationError),
}
