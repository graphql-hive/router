use graphql_tools::parser::query::{
    self as query_ast, Definition, Mutation, OperationDefinition, Query, SelectionSet, Subscription,
};

use crate::query_planner::{
    ast::normalization::error::NormalizationError,
    state::supergraph_state::{SupergraphDefinition, SupergraphState},
};

pub struct RootTypes<'a> {
    pub query: Option<&'a str>,
    pub mutation: Option<&'a str>,
    pub subscription: Option<&'a str>,
}

impl<'a> RootTypes<'a> {
    pub fn query_type_name(&self) -> Result<&'a str, NormalizationError> {
        self.query
            .ok_or_else(|| NormalizationError::TypeForOperationNotFound {
                kind: "query".to_string(),
            })
    }

    pub fn mutation_type_name(&self) -> Option<&'a str> {
        self.mutation
    }

    pub fn subscription_type_name(&self) -> Option<&'a str> {
        self.subscription
    }

    /// The name of the root type `operation` starts from.
    fn of(
        &self,
        operation: &OperationDefinition<'static, String>,
    ) -> Result<&'a str, NormalizationError> {
        let (name, kind) = match operation {
            OperationDefinition::SelectionSet(_) | OperationDefinition::Query(_) => {
                return self.query_type_name()
            }
            OperationDefinition::Mutation(_) => (self.mutation, "mutation"),
            OperationDefinition::Subscription(_) => (self.subscription, "subscription"),
        };
        name.ok_or_else(|| NormalizationError::TypeForOperationNotFound {
            kind: kind.to_string(),
        })
    }
}

/// An operation's root type definition and its selection set.
pub type Operation<'a, 's> = (
    &'a SupergraphDefinition,
    &'s mut SelectionSet<'static, String>,
);

pub struct NormalizationContext<'a> {
    pub operation_name: Option<&'a str>,
    pub document: &'a mut query_ast::Document<'static, String>,
    pub supergraph: &'a SupergraphState,
    pub root_types: RootTypes<'a>,
    pub subgraph_name: Option<&'a str>,
}

impl<'a> NormalizationContext<'a> {
    /// The selection set of each operation, with the definition of its root type.
    /// Fragment definitions are skipped.
    pub fn operations_mut(
        &mut self,
    ) -> impl Iterator<Item = Result<Operation<'a, '_>, NormalizationError>> + '_ {
        let supergraph = self.supergraph;
        let root_types = &self.root_types;
        self.document
            .definitions
            .iter_mut()
            .filter_map(move |definition| {
                let Definition::Operation(operation) = definition else {
                    return None;
                };
                let root = root_types.of(operation).and_then(|name| {
                    supergraph.definitions.get(name).ok_or_else(|| {
                        NormalizationError::SchemaTypeNotFound {
                            type_name: name.to_string(),
                        }
                    })
                });
                Some(root.map(|root| (root, selection_set_mut(operation))))
            })
    }
}

/// The selection set of an operation, whatever kind it is.
pub fn selection_set_mut<'o>(
    operation: &'o mut OperationDefinition<'static, String>,
) -> &'o mut SelectionSet<'static, String> {
    match operation {
        OperationDefinition::SelectionSet(selection_set)
        | OperationDefinition::Query(Query { selection_set, .. })
        | OperationDefinition::Mutation(Mutation { selection_set, .. })
        | OperationDefinition::Subscription(Subscription { selection_set, .. }) => selection_set,
    }
}

impl<'a> From<&'a SupergraphState> for RootTypes<'a> {
    fn from(state: &'a SupergraphState) -> Self {
        RootTypes {
            query: Some(state.query_type.as_ref()),
            mutation: state.mutation_type.as_deref(),
            subscription: state.subscription_type.as_deref(),
        }
    }
}
