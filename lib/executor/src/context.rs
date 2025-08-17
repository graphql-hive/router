use std::collections::HashMap;

use query_planner::planner::plan_nodes::{FetchNode, FetchRewrite, QueryPlan};

use crate::response::{graphql_error::GraphQLError, storage::ResponsesStorage, value::Value};

pub struct QueryPlanExecutionContext<'a> {
    pub response_storage: ResponsesStorage,
    pub final_response: Value<'a>,
    pub errors: Vec<GraphQLError>,
    pub output_rewrites: OutputRewritesStorage,
}

impl<'a> Default for QueryPlanExecutionContext<'a> {
    fn default() -> Self {
        QueryPlanExecutionContext {
            response_storage: Default::default(),
            output_rewrites: Default::default(),
            errors: Vec::new(),
            final_response: Value::Null,
        }
    }
}

impl<'a> QueryPlanExecutionContext<'a> {
    pub fn new(query_plan: &QueryPlan, init_final_response: Value<'a>) -> Self {
        QueryPlanExecutionContext {
            response_storage: ResponsesStorage::new(),
            output_rewrites: OutputRewritesStorage::from_query_plan(query_plan),
            errors: Vec::new(),
            final_response: init_final_response,
        }
    }
}

#[derive(Default)]
pub struct OutputRewritesStorage {
    output_rewrites: HashMap<i64, Vec<FetchRewrite>>,
}

impl OutputRewritesStorage {
    pub fn from_query_plan(query_plan: &QueryPlan) -> OutputRewritesStorage {
        let mut output_rewrites = OutputRewritesStorage {
            output_rewrites: HashMap::new(),
        };

        for fetch_node in query_plan.fetch_nodes() {
            output_rewrites.add_maybe(fetch_node);
        }

        output_rewrites
    }

    fn add_maybe(&mut self, fetch_node: &FetchNode) {
        if let Some(rewrites) = &fetch_node.output_rewrites {
            self.output_rewrites.insert(fetch_node.id, rewrites.clone());
        }
    }

    pub fn get(&self, id: i64) -> Option<&Vec<FetchRewrite>> {
        self.output_rewrites.get(&id)
    }
}
