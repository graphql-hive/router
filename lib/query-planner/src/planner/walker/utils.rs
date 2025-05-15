use graphql_parser_hive_fork::query::OperationDefinition;

use crate::{
    ast::selection_set::SelectionSet,
    graph::{edge::EdgeReference, Graph},
    state::supergraph_state::RootOperationType,
};

use super::error::WalkOperationError;

pub fn get_entrypoints<'a>(
    graph: &'a Graph,
    operation_type: &RootOperationType,
) -> Result<Vec<EdgeReference<'a>>, WalkOperationError> {
    let entrypoint_root = match operation_type {
        RootOperationType::Query => Some(graph.query_root),
        RootOperationType::Mutation => graph.mutation_root,
        RootOperationType::Subscription => graph.subscription_root,
    }
    .ok_or(WalkOperationError::MissingRootType(*operation_type))?;

    Ok(graph.edges_from(entrypoint_root).collect())
}

pub fn operation_to_parts(
    operation: &OperationDefinition<'static, String>,
) -> (RootOperationType, SelectionSet) {
    match operation {
        OperationDefinition::Query(query) => {
            (RootOperationType::Query, (&query.selection_set).into())
        }
        OperationDefinition::SelectionSet(selection_set) => {
            (RootOperationType::Query, selection_set.into())
        }
        OperationDefinition::Mutation(mutation) => (
            RootOperationType::Mutation,
            (&mutation.selection_set).into(),
        ),
        OperationDefinition::Subscription(subscription) => (
            RootOperationType::Subscription,
            (&subscription.selection_set).into(),
        ),
    }
}
