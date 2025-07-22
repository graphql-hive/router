use std::collections::HashMap;

use bytes::BytesMut;
use ouroboros::self_referencing;
use query_planner::planner::plan_nodes::{
    FetchNode, ParallelNode, PlanNode, QueryPlan, SequenceNode,
};
use simd_json::BorrowedValue;

use crate::{
    context::ExecutionContext,
    projection::{plan::ProjectionPlan, response::project_by_operation},
    schema::metadata::SchemaMetadata,
};

mod consts;
pub mod context;
mod json_writer;
pub mod projection;
pub mod response;
pub mod schema;

pub async fn execute_query_plan<'a>(
    query_plan: &QueryPlan,
    projection_plan: &ProjectionPlan<'a>,
    // subgraph_executor_map: &SubgraphExecutorMap,
    variable_values: &Option<HashMap<String, serde_json::Value>>,
    schema_metadata: &SchemaMetadata<'a>,
    operation_type_name: &str,
    _has_introspection: bool,
    // expose_query_plan: ExposeQueryPlanMode,
) -> BytesMut {
    let mut executor = Executor::new(
        schema_metadata,
        operation_type_name,
        projection_plan,
        variable_values,
    );

    executor.execute(query_plan.node.as_ref()).await;
    executor.finalize()
}

// Sequence(wave, wave, wave)
// Parallel(fetch, fetch)
// Fetch

pub struct Executor<'a> {
    execution_context: ExecutionContext<'a>,
    operation_type_name: &'a str,
    projection_plan: &'a ProjectionPlan<'a>,
    variable_values: &'a Option<HashMap<String, serde_json::Value>>,
}

impl<'a> Executor<'a> {
    pub fn new(
        schema_metadata: &'a SchemaMetadata<'a>,
        operation_type_name: &'a str,
        projection_plan: &'a ProjectionPlan<'a>,
        variable_values: &'a Option<HashMap<String, serde_json::Value>>,
    ) -> Self {
        Executor {
            execution_context: ExecutionContext::new(schema_metadata),
            operation_type_name,
            projection_plan,
            variable_values,
        }
    }

    pub async fn execute(&mut self, plan: Option<&PlanNode>) {
        match plan {
            Some(PlanNode::Fetch(node)) => self.execute_fetch_wave(node).await,
            Some(PlanNode::Parallel(node)) => self.execute_parallel_wave(node).await,
            Some(PlanNode::Sequence(node)) => self.execute_sequence_wave(node).await,
            Some(_) => panic!("Unsupported plan node type"),
            None => panic!("Empty plan"),
        }
    }

    pub fn finalize(&'a self) -> BytesMut {
        project_by_operation(
            &self.execution_context.response_storage.final_response,
            self.operation_type_name,
            &self.projection_plan.root_selections,
            self.variable_values,
        )
    }

    async fn execute_fetch_wave(&mut self, node: &FetchNode) {
        // no need to deep_merge,
        // we .into() and pass to final
        let result = self.get_result().await;

        self.execution_context.response_storage.add_response(result);
    }

    async fn execute_sequence_wave(&mut self, node: &SequenceNode) {
        // merge after every sequence
        // so the next step can read it.
        // It can hold a list of mix of Parallel blocks and Fetch and Flatten(Fetch)
    }

    async fn execute_parallel_wave(&mut self, node: &ParallelNode) {
        // we merge after all fetches
        // it can hold a list of Fetch or Flatten(Fetch)
    }

    async fn get_result(&self) -> ParsedResponse {
        // 1. Fetch data from the network.
        let response_body = br#"{"data": {"product": "super-fast-widget"}}"#.to_vec();
        let parsed_response =
            ParsedResponse::try_new(response_body, |buffer| simd_json::from_slice(buffer)).unwrap();

        parsed_response
    }
}

#[self_referencing]
pub struct ParsedResponse {
    buffer: Vec<u8>,

    #[borrows(mut buffer)]
    #[covariant]
    pub json: BorrowedValue<'this>,
}
