use graphql_parser::query::OperationDefinition;

use crate::{
    ast::selection_set::SelectionSet,
    graph::{edge::EdgeReference, Graph},
    state::supergraph_state::OperationKind,
};

use super::error::WalkOperationError;

pub fn get_entrypoints<'a>(
    graph: &'a Graph,
    operation_type: &OperationKind,
) -> Result<Vec<EdgeReference<'a>>, WalkOperationError> {
    let entrypoint_root = match operation_type {
        OperationKind::Query => Some(graph.query_root),
        OperationKind::Mutation => graph.mutation_root,
        OperationKind::Subscription => graph.subscription_root,
    }
    .ok_or(WalkOperationError::MissingRootType(operation_type.clone()))?;

    Ok(graph.edges_from(entrypoint_root).collect())
}

pub fn operation_to_parts(
    operation: &OperationDefinition<'static, String>,
) -> (OperationKind, SelectionSet) {
    match operation {
        OperationDefinition::Query(query) => (OperationKind::Query, (&query.selection_set).into()),
        OperationDefinition::SelectionSet(selection_set) => {
            (OperationKind::Query, selection_set.into())
        }
        OperationDefinition::Mutation(mutation) => {
            (OperationKind::Mutation, (&mutation.selection_set).into())
        }
        OperationDefinition::Subscription(subscription) => (
            OperationKind::Subscription,
            (&subscription.selection_set).into(),
        ),
    }
}
