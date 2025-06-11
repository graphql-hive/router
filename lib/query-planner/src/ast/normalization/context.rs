use graphql_parser::query as query_ast;

use crate::state::supergraph_state::SupergraphState;

pub struct RootTypes<'a> {
    pub query: Option<&'a str>,
    pub mutation: Option<&'a str>,
    pub subscription: Option<&'a str>,
}

pub struct NormalizationContext<'a> {
    pub operation_name: Option<&'a str>,
    pub document: &'a mut query_ast::Document<'static, String>,
    pub supergraph: &'a SupergraphState,
    pub root_types: RootTypes<'a>,
    pub subgraph_name: Option<&'a String>,
}

impl<'a> NormalizationContext<'a> {
    pub fn query_type_name(&self) -> &'a str {
        self.root_type_name("Query")
    }

    pub fn mutation_type_name(&self) -> &'a str {
        self.root_type_name("Mutation")
    }

    pub fn subscription_type_name(&self) -> &'a str {
        self.root_type_name("Subscription")
    }

    fn root_type_name(&self, fallback: &'a str) -> &'a str {
        self.root_types.query.unwrap_or(fallback)
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
