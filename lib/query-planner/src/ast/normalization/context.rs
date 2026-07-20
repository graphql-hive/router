use graphql_tools::parser::query as query_ast;

use crate::{
    ast::normalization::error::NormalizationError, state::supergraph_state::SupergraphState,
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
}

pub struct NormalizationContext<'a> {
    pub operation_name: Option<&'a str>,
    pub document: &'a mut query_ast::Document<'static, String>,
    pub supergraph: &'a SupergraphState,
    pub root_types: RootTypes<'a>,
    pub subgraph_name: Option<&'a String>,
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
