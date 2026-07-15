use std::sync::Arc;

use hive_router_query_planner::ast::operation::OperationDefinition;
use hive_router_query_planner::state::supergraph_state::OperationKind;

use crate::execution::client_request_details::ClientRequestDetails;
use crate::execution::plan::{CoerceVariablesPayload, PlanExecutionOutput};
use crate::hooks::on_graphql_params::GraphQLParams;
use crate::introspection::schema::SchemaMetadata;
use crate::operation_filter::{OperationFilter, OperationFilterOutput};
use crate::plugin_context::{PluginContext, RouterHttpRequest};
use crate::request_context::RequestContextPluginApi;

type RequestContextApi = RequestContextPluginApi<super::OnGraphqlAnalysis>;

pub use crate::operation_filter::{
    FieldInfo, FilterDecision, OperationFilterError, Selection, TypeConditionInfo,
};

type OperationFilterVisitor<'exec> =
    Box<dyn FnMut(&Selection<'exec>) -> FilterDecision + Send + 'exec>;

pub struct OnGraphqlAnalysisHookPayload<'exec> {
    pub router_http_request: &'exec RouterHttpRequest<'exec>,
    pub context: &'exec PluginContext,
    pub request_context: RequestContextApi,
    pub filtered_operation_for_plan: &'exec OperationDefinition,
    pub client_request_details: Arc<ClientRequestDetails<'exec>>,
    pub graphql_params: &'exec GraphQLParams,
    schema_metadata: &'exec SchemaMetadata,
    variable_payload: &'exec CoerceVariablesPayload,
    operation_filter_visitors: Vec<OperationFilterVisitor<'exec>>,
}

impl<'exec> OnGraphqlAnalysisHookPayload<'exec> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        router_http_request: &'exec RouterHttpRequest<'exec>,
        context: &'exec PluginContext,
        request_context: RequestContextApi,
        filtered_operation_for_plan: &'exec OperationDefinition,
        client_request_details: Arc<ClientRequestDetails<'exec>>,
        graphql_params: &'exec GraphQLParams,
        schema_metadata: &'exec SchemaMetadata,
        variable_payload: &'exec CoerceVariablesPayload,
    ) -> Self {
        Self {
            router_http_request,
            context,
            request_context,
            filtered_operation_for_plan,
            client_request_details,
            graphql_params,
            schema_metadata,
            variable_payload,
            operation_filter_visitors: Vec::new(),
        }
    }

    /// Register a visitor that decides per-field and per-inline-fragment
    /// whether to keep or reject it (with an error). A `Reject` on a
    /// non-null field bubbles up to the nearest nullable ancestor.
    ///
    /// The visitor is not run here - after every plugin has executed, the
    /// pipeline walks the original operation once.
    /// The first `Reject` wins.
    ///
    /// If the operation references a field that isn't declared in
    /// the schema metadata - which should never happen for an
    /// already-validated operation, but would otherwise be a bug - the
    /// pipeline aborts the request. Plugin authors never handle that error.
    pub fn filter_operation(
        &mut self,
        visitor: impl FnMut(&Selection<'exec>) -> FilterDecision + Send + 'exec,
    ) {
        self.operation_filter_visitors.push(Box::new(visitor));
    }

    pub fn run_operation_filters(
        self,
    ) -> Result<OperationFilterOutput<'exec>, OperationFilterError> {
        if self.operation_filter_visitors.is_empty() {
            return Ok(OperationFilterOutput::default());
        }

        let schema_metadata = self.schema_metadata;
        let operation = self.filtered_operation_for_plan;

        let root_type_name = match operation.operation_kind {
            None | Some(OperationKind::Query) => schema_metadata.query_type_name.as_deref(),
            Some(OperationKind::Mutation) => schema_metadata.mutation_type_name.as_deref(),
            Some(OperationKind::Subscription) => schema_metadata.subscription_type_name.as_deref(),
        }
        .expect("root type name not found in schema metadata");

        let mut visitors = self.operation_filter_visitors;
        OperationFilter::new(schema_metadata).filter(
            root_type_name,
            &operation.selection_set,
            self.variable_payload,
            |selection| {
                for visitor in visitors.iter_mut() {
                    if let FilterDecision::Reject { error } = visitor(selection) {
                        return FilterDecision::Reject { error };
                    }
                }
                FilterDecision::Keep
            },
        )
    }
}

pub enum OnGraphqlAnalysisHookResult {
    Proceed,
    EndWithResponse(PlanExecutionOutput),
}
