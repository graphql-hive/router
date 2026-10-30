use super::context::{collect_context_entities, unalias_response, AliasedRequests, ContextGroup};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::vec;

use crate::query_planner::ast::operation::{ContextAliasTemplate, SubgraphFetchOperation};
use crate::query_planner::planner::plan_nodes::{
    BatchFetchNode, CustomScalarPaths, FetchContext, FetchNode, FlattenNode, ResponsePathRef,
};
use crate::query_planner::planner::query_plan::QUERY_PLAN_KIND;
use crate::query_planner::{
    ast::operation::OperationDefinition,
    planner::plan_nodes::{
        ConditionNode, EntityBatch, EntityBatchAlias, FetchRewrite, FlattenNodePath, PlanNode,
        QueryPlan, SequenceNode,
    },
    state::supergraph_state::OperationKind,
};
use crate::telemetry::logging::{summary, targets};
use crate::telemetry::metrics::graphql_metrics::GraphQLErrorMetricsRecorder;
use crate::telemetry::traces::spans::graphql::{
    GraphQLOperationSpan, GraphQLSpanOperationIdentity, GraphQLSubgraphOperationSpan,
};
use crate::telemetry::traces::spans::observed_error::ObservedError;
use ahash::{HashMap as AHashMap, HashMapExt, HashSet as AHashSet};
use futures::TryFutureExt;
use futures::{
    future::BoxFuture,
    stream::{BoxStream, FuturesUnordered},
    FutureExt, StreamExt,
};
use http::{HeaderMap, StatusCode};
use serde::Serialize;
use sonic_rs::ValueRef;
use tracing::{debug, error, warn, Instrument};

use crate::executor::execution::client_request_details::OperationDetails;
use crate::executor::execution::demand_control::DemandControlExecutionContext;
use crate::executor::execution::error_masking::ErrorMaskingRuntime;
use crate::executor::execution::operation_name::OperationNameFactory;
use crate::executor::executors::common::ConnectionFingerprint;
use crate::executor::headers::cache_control;
use crate::executor::{
    execution::{
        client_request_details::ClientRequestDetails,
        error::{IntoPlanExecutionError, LazyPlanContext, PlanExecutionError},
        jwt_forward::JwtAuthForwardingPlan,
        rewrites::FetchRewriteExt,
    },
    execution_context::ExecutionContext,
    executors::{common::SubgraphExecutionRequest, map::SubgraphExecutorMap},
    extensions::{aggregator::apply_subgraph_extensions, plan::ExtensionsPlan},
    headers::{
        plan::HeaderRulesPlan,
        request::modify_subgraph_request_headers,
        response::{apply_subgraph_response_headers, ResponseHeaderAggregator, ResponseHeaderSink},
    },
    hooks::{
        on_execute::{
            DemandControlCost, DemandControlEstimatedCost, OnExecuteEndHookPayload,
            OnExecuteResponse, OnExecuteStartHookPayload,
        },
        on_graphql_error::handle_graphql_errors_with_plugins,
    },
    introspection::{
        resolve::{resolve_introspection, IntrospectionContext},
        schema::SchemaMetadata,
    },
    plugin_context::PluginRequestState,
    plugin_trait::{EarlyHTTPResponse, EndControlFlow, StartControlFlow},
    plugins::hooks,
    projection::{plan::ProjectionPlan, response::project_by_operation},
    response::{
        graphql_error::{GraphQLError, GraphQLErrorPath, GraphQLErrorPathSegment},
        merge::{deep_merge, deep_merge_from},
        subgraph_response::SubgraphResponse,
        value::Value,
    },
    utils::traverse::traverse_and_callback_mut,
};
use crate::query_planner::state::entity_keys::KeysByType;

pub type VariablesMap = HashMap<String, sonic_rs::Value>;

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionResultExtensions<'exec> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_plan: Option<&'exec QueryPlan>,

    #[serde(flatten)]
    pub extensions: HashMap<String, sonic_rs::Value>,
}

impl ExecutionResultExtensions<'_> {
    pub fn is_empty(&self) -> bool {
        self.query_plan.is_none() && self.extensions.is_empty()
    }
}

#[derive(Clone, Debug, Default)]
pub struct CoerceVariablesPayload {
    pub variables_map: Option<VariablesMap>,
}

impl CoerceVariablesPayload {
    /// All coerced variables, including any defaults applied during coercion.
    pub fn variables(&self) -> Option<&VariablesMap> {
        self.variables_map.as_ref()
    }

    /// The coerced value of `name`, including any default applied during coercion.
    pub fn variable(&self, name: &str) -> Option<&sonic_rs::Value> {
        self.variables().and_then(|vars| vars.get(name))
    }

    pub fn variable_equals_true(&self, name: &str) -> bool {
        self.variable_is_true(name).unwrap_or(false)
    }

    /// Like `variable_equals_true`, but `None` when it's missing, see `variable_is_true`.
    pub fn variable_is_true(&self, name: &str) -> Option<bool> {
        variable_is_true(&self.variables_map, name)
    }
}

pub struct QueryPlanExecutionOpts<'exec> {
    pub query_plan: &'exec QueryPlan,
    pub operation_for_plan: Arc<OperationDefinition>,
    pub projection_plan: Arc<ProjectionPlan>,
    pub headers_plan: Arc<HeaderRulesPlan>,
    pub extensions_plan: Arc<ExtensionsPlan>,
    pub variable_values: Arc<CoerceVariablesPayload>,
    pub extensions: ExecutionResultExtensions<'exec>,
    pub client_request: Arc<ClientRequestDetails<'exec>>,
    pub introspection_context: Arc<IntrospectionContext>,
    pub operation_type_name: String,
    pub operation_kind: OperationKind,
    pub executors: Arc<SubgraphExecutorMap>,
    pub jwt_auth_forwarding: Option<Arc<JwtAuthForwardingPlan>>,
    pub graphql_error_recorder: Option<GraphQLErrorMetricsRecorder>,
    pub demand_control_context: Option<Arc<DemandControlExecutionContext>>,
    pub initial_errors: Vec<GraphQLError>,
    pub span: GraphQLOperationSpan,
    pub plugin_req_state: Option<PluginRequestState<'exec>>,
    pub operation_name_factory: OperationNameFactory,
    pub response_header_sink: ResponseHeaderSink,
    pub error_masking_runtime: Arc<Option<ErrorMaskingRuntime>>,
    pub connection_fingerprint: Option<ConnectionFingerprint>,
}

pub struct PlanSubscriptionOutput {
    pub body: BoxStream<'static, Vec<u8>>,
    pub error_count: usize,
}

pub enum QueryPlanExecutionResult {
    Single(PlanExecutionOutput),
    Stream(PlanSubscriptionOutput),
}

#[derive(Default)]
pub struct PlanExecutionOutput {
    pub body: Vec<u8>,
    pub error_count: usize,
    pub status_code: StatusCode,
}

#[derive(Serialize)]
pub struct FailedExecutionResult {
    pub errors: Vec<GraphQLError>,
}

impl FailedExecutionResult {
    pub fn serialize(&self) -> Vec<u8> {
        sonic_rs::to_vec(&self).unwrap_or_else(|err| {
            // should never happen. result should always serialize - but hey, no unwraps
            error!(target: targets::CORE, error = ?err, "Failed to serialize pipeline error to response");

            sonic_rs::to_vec(&FailedExecutionResult {
                errors: vec![GraphQLError::from_message_and_code(
                    "Failed to serialize error response",
                    "INTERNAL_SERVER_ERROR",
                )],
            })
            .unwrap()
        })
    }
}

#[inline]
fn serialize_masked_failure(
    mut error: GraphQLError,
    error_masking_runtime: &Option<ErrorMaskingRuntime>,
) -> Vec<u8> {
    if let Some(runtime) = error_masking_runtime {
        runtime.apply(&mut error);
    }
    FailedExecutionResult {
        errors: vec![error],
    }
    .serialize()
}

fn early_http_response_into_execution_output(
    response: EarlyHTTPResponse,
    response_header_sink: &ResponseHeaderSink,
) -> PlanExecutionOutput {
    if !response.headers.is_empty() {
        response_header_sink.store(ResponseHeaderAggregator::from_http_headers(
            &response.headers,
        ));
    }

    PlanExecutionOutput {
        body: response.body,
        error_count: 0,
        status_code: response.status_code,
    }
}

#[inline]
fn log_plan_execution_error(error: &PlanExecutionError) {
    if let Some(subgraph_name) = error.subgraph_name() {
        error!(
            target: targets::GRAPHQL_EXECUTION,
            subgraph = ?subgraph_name,
            error = ?error,
            "Error executing step",
        );
    } else {
        error!(
            target: targets::GRAPHQL_EXECUTION,
            error = ?error,
            "Error executing step",
        );
    }
}

pub async fn execute_query_plan<'exec>(
    opts: QueryPlanExecutionOpts<'exec>,
) -> Result<QueryPlanExecutionResult, PlanExecutionError> {
    let (subscription_node, remaining_nodes) = match &opts.query_plan.node {
        // a subscription to a subgraph that contains all data and doesn't need entity resolution
        Some(PlanNode::Subscription(sub)) => (Some(sub), None),
        // a subscription that needs entity resolution. after emitting, it needs to execute the
        // remaining plan nodes in the sequence
        Some(PlanNode::Sequence(seq)) => match seq.nodes.first() {
            Some(PlanNode::Subscription(sub)) => {
                let remaining = if seq.nodes.len() > 1 {
                    // TODO: why to_vec()? is it wasteful? it's actually a slice, we dont need it as Vec
                    Some(seq.nodes[1..].to_vec())
                } else {
                    None
                };
                (Some(sub), remaining)
            }
            _ => (None, None),
        },
        _ => (None, None),
    };

    // subscription
    if let Some(sub) = subscription_node {
        // subscription

        // the primary (fetch node) of the subscription is the
        // subscription destination, we execute it first and it
        // would give us back a stream of results
        let fetch_node = &sub.primary;

        // we assemble a synthetic query plan for entity resolution from remaining nodes
        // because we might need entity resolution after receiving each subscription event
        let query_plan: Arc<QueryPlan> = Arc::new(QueryPlan {
            kind: QUERY_PLAN_KIND,
            node: remaining_nodes.map(|nodes| {
                if nodes.len() == 1 {
                    nodes.into_iter().next().unwrap()
                } else {
                    PlanNode::Sequence(SequenceNode { nodes })
                }
            }),
        });

        // we perform a regular subgraph request to the subscription subgraph
        // the only difference is that we get back a stream of results
        let mut headers_map = HeaderMap::new();
        modify_subgraph_request_headers(
            &opts.headers_plan,
            &fetch_node.service_name,
            &opts.client_request,
            &mut headers_map,
        )
        .with_plan_context(LazyPlanContext {
            subgraph_name: || Some(fetch_node.service_name.to_string()),
            affected_path: || None,
        })?;
        let variable_refs = select_fetch_variables(
            &opts.variable_values.variables_map,
            fetch_node.variable_usages.as_ref(),
        );

        let mut subgraph_request = SubgraphExecutionRequest {
            query: &fetch_node.operation.document_str,
            document_name_write_pos: fetch_node.operation.name_write_position,
            dedupe: false,
            operation_name: opts
                .operation_name_factory
                .generate(&fetch_node.service_name, fetch_node.id),
            variables: variable_refs,
            headers: headers_map,
            raw_variable_values: None,
            extensions: None,
            custom_scalar_paths: fetch_node.custom_scalar_paths.as_ref(),
            connection_fingerprint: opts.connection_fingerprint,
        };

        // TODO: otel instrumentation and stuff
        // let subgraph_operation_span = GraphQLSubgraphOperationSpan::new(
        //     fetch_node.service_name.as_str(),
        //     &fetch_node.operation.document_str,
        // );
        // subgraph_operation_span.record_operation_identity(GraphQLSpanOperationIdentity {
        //     name: subgraph_request.operation_name,
        //     operation_type: match fetch_node.operation_kind {
        //         Some(OperationKind::Query) | None => "query",
        //         Some(OperationKind::Mutation) => "mutation",
        //         Some(OperationKind::Subscription) => "subscription",
        //     },
        //     client_document_hash: fetch_node.operation.hash.to_string().as_str(),
        // });

        if let Some(jwt_forwarding_plan) = &opts.jwt_auth_forwarding {
            subgraph_request.add_request_extensions_field(
                jwt_forwarding_plan.extension_field_name.clone(),
                jwt_forwarding_plan.extension_field_value.clone(),
            );
        }

        let mut response_stream = opts
            .executors
            .subscribe(
                &fetch_node.service_name,
                subgraph_request,
                &opts.client_request,
            )
            .await
            .with_plan_context(LazyPlanContext {
                subgraph_name: || Some(fetch_node.service_name.to_string()),
                affected_path: || None,
            })?;
        // clone all necessary data from the context for usage in the stream.
        // the stream will move all of these values inside its closure
        let subgraph_name: String = fetch_node.service_name.clone();
        let client_method = opts.client_request.method.clone();
        let client_url = opts.client_request.url.clone();
        let client_headers = opts.client_request.headers.clone();
        let client_operation_name = opts.client_request.operation.name.map(|s| s.to_string());
        let client_operation_query = opts.client_request.operation.query.to_string();
        let client_operation_kind = opts.client_request.operation.kind;
        let client_jwt = opts.client_request.jwt.clone();
        let client_path_params = opts.client_request.path_params.into_owned();
        let response_header_sink = opts.response_header_sink.clone();
        let error_masking_runtime = opts.error_masking_runtime.clone();

        let operation_name_factory = opts.operation_name_factory.clone();

        let body_stream = Box::pin(async_stream::stream! {
            while let Some(stream_result) = response_stream.next().await {
                let response = match stream_result.with_plan_context(LazyPlanContext {
                                subgraph_name: || Some(subgraph_name.to_string()),
                                affected_path: || None,
                            }) {
                    Ok(response) => response,
                    // NOTE: I thought about going one way up and having the
                    // PlanSubscriptionOutput.body be a `Result<Vec<u8>, SubgraphExecutorError>`
                    // but that would put the burden on the caller to handle errors that are
                    // internal to the execution of the query plan (subgraph executor errors).
                    // furthermore, we want to always act on those errors the same way (stream and stop,
                    // read below) and not allow the caller to decide and potentially decide wrong
                    Err(ref err) => {
                        // not a fatal error, but stream it and stop.
                        // it's not fatal because the subgraph might recover and send more
                        // events if the subgraph error is a network error. but we fail and stop
                        // just to be on the safe side and avoid infinite error streaming because
                        // we cannot guarantee that the subgraph will recover and clients might
                        // simply ignore errors wasting the router's resources
                        log_plan_execution_error(err);
                        yield serialize_masked_failure(err.into(), error_masking_runtime.as_ref());
                        return;
                    }
                };
                let mut initial_errors = opts.initial_errors.clone();
                if let Some(new_errors) = response.errors {
                    initial_errors.extend(new_errors);
                }
                let opts = QueryPlanExecutionOpts {
                    query_plan: &query_plan,
                    operation_for_plan: opts.operation_for_plan.clone(),
                    projection_plan: opts.projection_plan.clone(),
                    headers_plan: opts.headers_plan.clone(),
                    extensions_plan: opts.extensions_plan.clone(),
                    variable_values: opts.variable_values.clone(),
                    extensions: ExecutionResultExtensions::default(),
                    client_request: ClientRequestDetails {
                        method: &client_method,
                        url: &client_url,
                        headers: client_headers.clone(),
                        operation: OperationDetails {
                            query: &client_operation_query,
                            name: client_operation_name.as_deref(),
                            kind: client_operation_kind,
                        },
                        jwt: client_jwt.clone(),
                        path_params: client_path_params.clone(),
                    }.into(),
                    introspection_context: opts.introspection_context.clone(),
                    operation_type_name: opts.operation_type_name.clone(),
                    operation_kind: opts.operation_kind.clone(),
                    executors: opts.executors.clone(),
                    jwt_auth_forwarding: opts.jwt_auth_forwarding.clone(),
                    initial_errors,
                    span: GraphQLOperationSpan { span: opts.span.clone() },
                    // TODO: plugins for subscriptions are not yet supported
                    plugin_req_state: None,
                    graphql_error_recorder: None,
                    operation_name_factory: operation_name_factory.clone(),
                    demand_control_context: opts.demand_control_context.clone(),
                    response_header_sink: response_header_sink.clone(),
                    error_masking_runtime: opts.error_masking_runtime.clone(),
                    connection_fingerprint: opts.connection_fingerprint,
                };
                match execute_query_plan_with_data(response.data, opts).await {
                    Ok(result) => yield result.body,
                    Err(ref err) => {
                        // fatal error, stream it and stop
                        log_plan_execution_error(err);
                        yield serialize_masked_failure(err.into(), error_masking_runtime.as_ref());
                        return;
                    }
                }
            }
        });

        return Ok(QueryPlanExecutionResult::Stream(PlanSubscriptionOutput {
            body: body_stream,
            error_count: 0, // NOTE: errors can only happen before streaming started
        }));
    }

    // query or mutation

    let introspection_context_clone = Arc::clone(&opts.introspection_context);
    // An operation without fields, like `{ topProducts @skip(if: true) { name } }`,
    // still has `{}` as its data.
    let data = if let Some(introspection_query) = &introspection_context_clone.query {
        resolve_introspection(introspection_query, &introspection_context_clone)
    } else {
        Value::Object(Vec::new())
    };

    let output = execute_query_plan_with_data(data, opts).await?;

    Ok(QueryPlanExecutionResult::Single(output))
}

async fn execute_query_plan_with_data<'exec>(
    mut data: Value<'exec>,
    mut opts: QueryPlanExecutionOpts<'exec>,
) -> Result<PlanExecutionOutput, PlanExecutionError> {
    // declared first so it outlives everything that borrows from it
    let aliased_requests = AliasedRequests::default();
    let mut errors = opts.initial_errors;

    let dedupe_subgraph_requests = opts.operation_kind.is_query();

    let mut on_end_callbacks = vec![];

    // TODO: coprocessor.on_execution_request
    if let Some(plugin_req_state) = opts.plugin_req_state.as_ref() {
        let mut start_payload = OnExecuteStartHookPayload {
            router_http_request: &plugin_req_state.router_http_request,
            context: &plugin_req_state.context,
            request_context: plugin_req_state
                .request_context
                .for_plugin::<hooks::OnExecute>(),
            query_plan: opts.query_plan,
            operation_for_plan: &opts.operation_for_plan,
            data,
            errors,
            extensions: opts.extensions.extensions,
            variable_values: &opts.variable_values.variables_map,
            dedupe_subgraph_requests,
            demand_control_estimate: opts.demand_control_context.as_ref().map(|dc| {
                DemandControlEstimatedCost {
                    estimated: dc.evaluation.estimated_cost,
                    max: dc.operation.operation_max_cost,
                }
            }),
        };

        for plugin in plugin_req_state.plugins.as_ref() {
            let result = plugin.on_execute(start_payload).await;
            start_payload = result.payload;
            match result.control_flow {
                StartControlFlow::Proceed => { /* continue to next plugin */ }
                StartControlFlow::EndWithResponse(response) => match response {
                    OnExecuteResponse::Output(response) => return Ok(response),
                    OnExecuteResponse::EarlyResponse(response) => {
                        return Ok(early_http_response_into_execution_output(
                            response,
                            &opts.response_header_sink,
                        ));
                    }
                },
                StartControlFlow::OnEnd(callback) => {
                    on_end_callbacks.push(callback);
                }
            }
        }

        // Give the ownership back to variables
        data = start_payload.data;
        errors = start_payload.errors;
        opts.extensions.extensions = start_payload.extensions;
    }

    let mut exec_ctx = ExecutionContext::new(data, errors);
    // No need for `new`, it has too many parameters
    // We can directly create `Executor` instance here
    let executor = Executor {
        variable_values: &opts.variable_values.variables_map,
        schema_metadata: &opts.introspection_context.metadata,
        executors: &opts.executors,
        client_request: &opts.client_request,
        headers_plan: &opts.headers_plan,
        extensions_plan: &opts.extensions_plan,
        jwt_forwarding_plan: opts.jwt_auth_forwarding,
        dedupe_subgraph_requests,
        demand_control_context: opts.demand_control_context.clone(),
        plugin_req_state: opts.plugin_req_state.as_ref(),
        operation_name_factory: &opts.operation_name_factory,
        connection_fingerprint: opts.connection_fingerprint,
        aliased_requests: &aliased_requests,
    };

    if let Some(node) = &opts.query_plan.node {
        executor.execute_plan_node(&mut exec_ctx, node).await;
    }

    let error_count = exec_ctx.errors.len(); // Added for usage reporting

    if error_count > 0 {
        opts.span.record_error_count(error_count);
        opts.span
            .record_errors(|| exec_ctx.errors.iter().map(|e| e.into()).collect());

        if let Some(error_recorder) = opts.graphql_error_recorder.as_ref() {
            error_recorder.record_errors(|| {
                exec_ctx
                    .errors
                    .iter()
                    .map(|err| err.extensions.code.as_deref())
            });
        }
    }

    let mut data = exec_ctx.data;
    let mut errors = exec_ctx.errors;

    summary::record(|s| s.set_partial_response(error_count > 0 && !data.is_null()));
    let mut response_size_estimate = exec_ctx.response_storage.estimate_final_response_size();

    let mut demand_control_cost = None;
    if let Some(demand_control) = executor.demand_control_context {
        let actual = demand_control.calculate_actual_cost(
            &data,
            &opts.variable_values.variables_map,
            &exec_ctx.subgraph_response_cost_tracker,
        );

        demand_control_cost = Some(DemandControlCost {
            estimated: demand_control.evaluation.estimated_cost,
            max: demand_control.operation.operation_max_cost,
            actual,
        });

        if actual > demand_control.operation.operation_max_cost {
            warn!(
                target: targets::DEMAND_CONTROL,
                operation_name = ?opts.operation_for_plan.name.as_deref(),
                actual_cost = actual,
                estimated_cost = demand_control.evaluation.estimated_cost,
                max_cost = demand_control.operation.operation_max_cost,
                "actual cost exceeds max cost (not enforced)"
            );
        }

        demand_control.report_telemetry(
            actual,
            opts.operation_for_plan.name.as_deref(),
            &opts.span,
        );
        demand_control.apply_expose_headers(&mut exec_ctx.response_headers_aggregator, actual);
    }

    cache_control::finalize(
        &mut exec_ctx.response_headers_aggregator,
        // force no-store for mutations and errors (execution or graphql errors)
        opts.operation_kind.is_mutation() || !errors.is_empty(),
        // response_storage.len() counts subgraphs that returned bytes, which is
        // exactly what we need: a plugin hook can short-circuit with a bytes-less
        // SubgraphResponse, but in that case it also produces no cache-control header,
        // so both sides of the comparison in finalize stay in sync - that fetch is
        // invisible to both counters and cancels out (is safe to ignore)
        exec_ctx.response_storage.len(),
    );

    opts.response_header_sink
        .store(std::mem::take(&mut exec_ctx.response_headers_aggregator));

    // merge_into takes ownership of the aggregator to drain it in one pass; mem::take
    // gives us ownership while leaving a default (empty) aggregator in place, since
    // exec_ctx is still borrowed and we cannot move out of it directly
    std::mem::take(&mut exec_ctx.extensions_aggregator).merge_into(&mut opts.extensions.extensions);

    // TODO: coprocessor.on_execution_response
    if !on_end_callbacks.is_empty() {
        let mut end_payload = OnExecuteEndHookPayload {
            data,
            errors,
            extensions: opts.extensions.extensions,
            response_size_estimate,
            request_context: opts
                .plugin_req_state
                .as_ref()
                .map(|state| state.request_context.for_plugin::<hooks::OnExecute>())
                .expect("plugin state not available, but on_end_callbacks are present"),
            demand_control_cost,
        };

        for callback in on_end_callbacks {
            let result = callback(end_payload);
            end_payload = result.payload;
            match result.control_flow {
                EndControlFlow::Proceed => { /* continue to next callback */ }
                EndControlFlow::EndWithResponse(response) => match response {
                    OnExecuteResponse::Output(response) => return Ok(response),
                    OnExecuteResponse::EarlyResponse(response) => {
                        return Ok(early_http_response_into_execution_output(
                            response,
                            &opts.response_header_sink,
                        ));
                    }
                },
            }
        }

        // Give the ownership back to variables
        data = end_payload.data;
        errors = end_payload.errors;
        opts.extensions.extensions = end_payload.extensions;
        response_size_estimate = end_payload.response_size_estimate;
    }

    let mut status_code = StatusCode::OK;

    if !errors.is_empty() {
        if let Some(plugin_req_state) = opts.plugin_req_state.as_ref() {
            let (new_errors, new_status_code) = handle_graphql_errors_with_plugins(
                plugin_req_state.plugins.as_ref(),
                plugin_req_state.context.as_ref(),
                &plugin_req_state.request_context,
                errors,
                status_code,
            );

            errors = new_errors;
            status_code = new_status_code;
        }
    }

    if let Some(error_masking_runtime) = opts.error_masking_runtime.as_ref() {
        for error in &mut errors {
            error_masking_runtime.apply(error);
        }
    }

    let body = project_by_operation(
        &data,
        errors,
        &opts.extensions,
        opts.operation_type_name.as_str(),
        &opts.projection_plan,
        &opts.variable_values.variables_map,
        response_size_estimate,
        &opts.introspection_context.metadata,
    )
    .with_plan_context(LazyPlanContext {
        subgraph_name: || None,
        affected_path: || None,
    })?;

    Ok(PlanExecutionOutput {
        body,
        error_count,
        status_code,
    })
}

pub struct Executor<'exec> {
    pub variable_values: &'exec Option<VariablesMap>,
    pub schema_metadata: &'exec SchemaMetadata,
    pub executors: &'exec SubgraphExecutorMap,
    pub client_request: &'exec ClientRequestDetails<'exec>,
    pub headers_plan: &'exec HeaderRulesPlan,
    pub extensions_plan: &'exec ExtensionsPlan,
    pub jwt_forwarding_plan: Option<Arc<JwtAuthForwardingPlan>>,
    pub dedupe_subgraph_requests: bool,
    pub demand_control_context: Option<Arc<DemandControlExecutionContext>>,
    pub plugin_req_state: Option<&'exec PluginRequestState<'exec>>,
    pub operation_name_factory: &'exec OperationNameFactory,
    pub connection_fingerprint: Option<ConnectionFingerprint>,
    pub aliased_requests: &'exec AliasedRequests,
}

pub enum ExecutionJob<'exec> {
    Fetch {
        subgraph_name: &'exec str,
        operation: &'exec SubgraphFetchOperation,
        response: SubgraphResponse<'exec>,
        output_rewrites: Option<&'exec [FetchRewrite]>,
        // By default, this job is merged at the root. When this is set, we merge at
        // a specific path instead of the root (used for root re-entry fetches)
        merge_path: Option<ResponsePathRef<'exec>>,
    },
    FlattenFetch {
        subgraph_name: &'exec str,
        operation: &'exec SubgraphFetchOperation,
        response: SubgraphResponse<'exec>,
        flatten_node_path: &'exec FlattenNodePath,
        representation_hashes: Arc<Vec<Option<u64>>>,
        representation_hash_to_index: AHashMap<u64, usize>,
        output_rewrites: Option<&'exec [FetchRewrite]>,
    },
    BatchFetch {
        subgraph_name: &'exec str,
        operation: &'exec SubgraphFetchOperation,
        response: SubgraphResponse<'exec>,
        aliases: Vec<AliasBatchState<'exec>>,
    },
}

/// One subgraph request of a Flatten or BatchFetch, ready to be sent. `merge` is what its
/// response gets merged with: the representation indexes of a Flatten, the alias states of a
/// BatchFetch.
struct PreparedRequest<'exec, M> {
    operation: &'exec SubgraphFetchOperation,
    variables: Vec<(&'exec str, Vec<u8>)>,
    custom_scalar_paths: Option<&'exec CustomScalarPaths>,
    merge: M,
    /// Set when several groups went in one aliased request
    unalias: Option<Unalias<'exec>>,
}

/// Turns an aliased response back into the response of the original operation.
struct Unalias<'exec> {
    template: &'exec ContextAliasTemplate,
    groups: usize,
    /// Representations sent per response key of `template` and group
    counts: Vec<Vec<usize>>,
}

impl<'exec> Unalias<'exec> {
    fn apply(&self, response: &mut SubgraphResponse<'exec>) {
        unalias_response(response, self.template, self.groups, |key, group| {
            self.counts[key][group]
        });
    }
}

/// The variables of one wave of a BatchFetch, and what each alias sends in it.
type BatchWave<'exec> = (Vec<(&'exec str, Vec<u8>)>, Vec<AliasBatchState<'exec>>);

pub struct AliasBatchState<'exec> {
    alias_spec: &'exec EntityBatchAlias,
    /// Shared by the aliases that share a representations variable.
    representation_hash_to_index: Arc<AHashMap<u64, usize>>,
    paths: Vec<AliasPathState<'exec>>,
}

#[derive(Clone)]
struct AliasPathState<'exec> {
    merge_path: ResponsePathRef<'exec>,
    representation_hashes: Arc<Vec<Option<u64>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct AliasIndex(usize);

#[derive(Default)]
struct BatchFetchErrors {
    by_alias_index: AHashMap<AliasIndex, Vec<GraphQLError>>,
    unmatched: Vec<GraphQLError>,
}

impl<'exec> ExecutionJob<'exec> {
    fn response(self) -> SubgraphResponse<'exec> {
        match self {
            ExecutionJob::Fetch { response, .. } => response,
            ExecutionJob::FlattenFetch { response, .. } => response,
            ExecutionJob::BatchFetch { response, .. } => response,
        }
    }

    pub fn response_ref(&self) -> &SubgraphResponse<'exec> {
        match self {
            ExecutionJob::Fetch { response, .. } => response,
            ExecutionJob::FlattenFetch { response, .. } => response,
            ExecutionJob::BatchFetch { response, .. } => response,
        }
    }

    pub fn subgraph_name(&self) -> &'exec str {
        match self {
            ExecutionJob::Fetch { subgraph_name, .. } => subgraph_name,
            ExecutionJob::FlattenFetch { subgraph_name, .. } => subgraph_name,
            ExecutionJob::BatchFetch { subgraph_name, .. } => subgraph_name,
        }
    }

    pub fn operation(&self) -> &'exec SubgraphFetchOperation {
        match self {
            ExecutionJob::Fetch { operation, .. } => operation,
            ExecutionJob::FlattenFetch { operation, .. } => operation,
            ExecutionJob::BatchFetch { operation, .. } => operation,
        }
    }

    fn affected_path(&self) -> Option<ResponsePathRef<'exec>> {
        match self {
            ExecutionJob::Fetch { merge_path, .. } => *merge_path,
            ExecutionJob::FlattenFetch {
                flatten_node_path, ..
            } => Some(flatten_node_path.as_ref()),
            ExecutionJob::BatchFetch { .. } => None,
        }
    }
}

struct PrepareExecutionJobOpts<'exec> {
    // The name of the subgraph
    subgraph_name: &'exec str,
    // Variable usages
    variable_usages: Option<&'exec BTreeSet<String>>,
    // Operation name
    operation_name: Option<String>,
    // Operation Kind
    operation_kind: Option<&'exec OperationKind>,
    // Operation
    operation: &'exec SubgraphFetchOperation,
    // Output rewrites
    output_rewrites: Option<&'exec [FetchRewrite]>,
    // Response paths whose values should stay raw JSON in `data`
    custom_scalar_paths: Option<&'exec CustomScalarPaths>,
    // If the fetch job is for a flatten node, we pass the filtered representations,
    raw_variable_values: Option<Vec<(&'exec str, Vec<u8>)>>,
    // and the path to the representations in the original response for error handling and normalization
    affected_path: Option<ResponsePathRef<'exec>>,
}

impl<'exec> Executor<'exec> {
    async fn execute_plan_node(&self, ctx: &mut ExecutionContext<'exec>, node: &'exec PlanNode) {
        match node {
            PlanNode::Parallel(_) => {
                let mut scope = FuturesUnordered::new();
                // children that aren't a fetch, like the `Sequence` in
                // `Parallel { Include(if: $a) { Parallel { Sequence { .. }, Fetch } } }`, they run
                // once the fetches are done. Siblings don't depend on each other, so that's fine.
                // They run one after another though, two `Sequence`s don't overlap.
                let mut after = Vec::new();

                // We borrow `ctx.data` only for sync preparation of the job future,
                // and the actual execution of the job future is done without the borrow of `ctx.data`
                self.prepare_parallel_child(node, &ctx.data, &mut scope, &mut after);

                while let Some(job) = scope.next().await {
                    self.process_job_result(ctx, job);
                }

                for node in after {
                    Box::pin(self.execute_plan_node(ctx, node)).await;
                }
            }
            PlanNode::Sequence(sequence_node) => {
                for child in &sequence_node.nodes {
                    // We use `Box.pin` here to avoid the compiler error about recursive future,
                    // as `execute_plan_node` is calling itself recursively for sequence nodes
                    Box::pin(self.execute_plan_node(ctx, child)).await;
                }
            }
            PlanNode::Condition(condition_node) => {
                if let Some(next_node) =
                    condition_node_by_variables(condition_node, self.variable_values)
                {
                    // We use `Box.pin` here to avoid the compiler error about recursive future,
                    // as `execute_plan_node` is calling itself recursively for condition nodes
                    Box::pin(self.execute_plan_node(ctx, next_node)).await;
                }
            }
            node => {
                let mut scope: FuturesUnordered<_> = self
                    .prepare_job_future(node, &ctx.data)
                    .into_iter()
                    .collect();
                while let Some(job) = scope.next().await {
                    self.process_job_result(ctx, job);
                }
            }
        }
    }

    fn fetch_runs(&self, fetch_node: &FetchNode) -> bool {
        fetch_node
            .run_if
            .as_ref()
            .is_none_or(|run_if| run_if.holds(|name| variable_is_true(self.variable_values, name)))
    }

    fn prepare_root_reentry_fetch_job<'wave>(
        &'wave self,
        flatten_node: &'exec FlattenNode,
        fetch_node: &'exec FetchNode,
    ) -> BoxFuture<'wave, Result<ExecutionJob<'exec>, PlanExecutionError>> {
        self.prepare_execution_job(PrepareExecutionJobOpts {
            subgraph_name: &fetch_node.service_name,
            variable_usages: fetch_node.variable_usages.as_ref(),
            operation_name: self
                .operation_name_factory
                .generate(&fetch_node.service_name, fetch_node.id),
            operation_kind: fetch_node.operation_kind.as_ref(),
            operation: &fetch_node.operation,
            output_rewrites: fetch_node.output_rewrites.as_deref(),
            custom_scalar_paths: fetch_node.custom_scalar_paths.as_ref(),
            raw_variable_values: None,
            affected_path: Some(flatten_node.path.as_ref()),
        })
        .boxed()
    }

    /// Splits the waves of a BatchFetch into subgraph requests. When the operation allows it,
    /// several waves go in one aliased request, the rest are sent one by one.
    fn prepare_batch_requests(
        &self,
        batch_fetch_node: &'exec BatchFetchNode,
        waves: Vec<BatchWave<'exec>>,
    ) -> Vec<PreparedRequest<'exec, Vec<AliasBatchState<'exec>>>> {
        let waves_per_request = batch_fetch_node.operation.max_context_groups();
        let mut requests = Vec::new();
        let mut waves = waves.into_iter().peekable();
        while waves.peek().is_some() {
            let chunk: Vec<_> = waves.by_ref().take(waves_per_request).collect();
            let template = if chunk.len() < 2 {
                None
            } else {
                batch_fetch_node.operation.context_alias_template()
            };
            let Some(template) = template else {
                requests.extend(
                    chunk
                        .into_iter()
                        .map(|(variables, aliases)| PreparedRequest {
                            operation: &batch_fetch_node.operation,
                            variables,
                            custom_scalar_paths: batch_fetch_node.custom_scalar_paths.as_ref(),
                            merge: aliases,
                            unalias: None,
                        }),
                );
                continue;
            };
            let wave_count = chunk.len();
            // representations each alias sends over all the waves, so its map is sized once
            let mut totals = vec![0; chunk[0].1.len()];
            for (_, aliases) in &chunk {
                for (total, alias) in totals.iter_mut().zip(aliases) {
                    *total += alias.representation_hash_to_index.len();
                }
            }
            // the aliases of the first wave take in the entities of all the others
            let mut chunk = chunk.into_iter();
            let Some((values, mut merged_aliases)) = chunk.next() else {
                continue;
            };
            let mut values_per_wave = Vec::with_capacity(wave_count);
            values_per_wave.push(values);
            // representations sent per alias and wave
            let mut counts: Vec<Vec<usize>> = Vec::with_capacity(merged_aliases.len());
            // representations sent per alias so far, so where the next wave starts
            let mut offsets: Vec<usize> = Vec::with_capacity(merged_aliases.len());
            for (alias, total) in merged_aliases.iter_mut().zip(&totals) {
                let count = alias.representation_hash_to_index.len();
                counts.push(vec![count]);
                offsets.push(count);
                // aliases that share a representations variable share the map, each one gets
                // its own copy here since every alias takes in its own waves
                Arc::make_mut(&mut alias.representation_hash_to_index).reserve(total - count);
            }
            for (values, aliases) in chunk {
                values_per_wave.push(values);
                for (((merged, counts), offset), alias) in merged_aliases
                    .iter_mut()
                    .zip(&mut counts)
                    .zip(&mut offsets)
                    .zip(aliases)
                {
                    let count = alias.representation_hash_to_index.len();
                    counts.push(count);
                    Arc::make_mut(&mut merged.representation_hash_to_index).extend(
                        alias
                            .representation_hash_to_index
                            .iter()
                            .map(|(hash, index)| (*hash, index + *offset)),
                    );
                    *offset += count;
                }
            }
            let (operation, variables) = self.aliased_requests.write(template, values_per_wave);
            // the batch alias behind every response key, looked up once
            let batch_aliases = &batch_fetch_node.entity_batch.aliases;
            let counts = template
                .response_keys
                .iter()
                .map(|key| {
                    batch_aliases
                        .iter()
                        .position(|alias| alias.alias == *key)
                        .map_or_else(
                            || vec![0; wave_count],
                            |alias| std::mem::take(&mut counts[alias]),
                        )
                })
                .collect();
            requests.push(PreparedRequest {
                operation,
                variables,
                custom_scalar_paths: template.custom_scalar_paths(),
                merge: merged_aliases,
                unalias: Some(Unalias {
                    template,
                    groups: wave_count,
                    counts,
                }),
            });
        }

        requests
    }

    /// Splits the context groups of a Flatten into subgraph requests, like
    /// [`Self::prepare_batch_requests`] does with waves.
    fn prepare_flatten_requests(
        &self,
        fetch_node: &'exec FetchNode,
        groups: Vec<ContextGroup<'exec>>,
    ) -> Vec<PreparedRequest<'exec, AHashMap<u64, usize>>> {
        let groups_per_request = fetch_node.operation.max_context_groups();
        let mut requests = Vec::new();
        let mut groups = groups
            .into_iter()
            .filter(|group| !group.hash_to_index.is_empty())
            .peekable();
        while groups.peek().is_some() {
            let chunk: Vec<_> = groups.by_ref().take(groups_per_request).collect();
            let template = if chunk.len() < 2 {
                None
            } else {
                fetch_node.operation.context_alias_template()
            };
            let Some(template) = template else {
                requests.extend(chunk.into_iter().map(|group| {
                    let mut variables = group.variables;
                    variables.push(("representations", group.representations));
                    PreparedRequest {
                        operation: &fetch_node.operation,
                        variables,
                        custom_scalar_paths: fetch_node.custom_scalar_paths.as_ref(),
                        merge: group.hash_to_index,
                        unalias: None,
                    }
                }));
                continue;
            };
            let mut values_per_group = Vec::with_capacity(chunk.len());
            let mut hash_to_index =
                AHashMap::with_capacity(chunk.iter().map(|group| group.hash_to_index.len()).sum());
            // representations sent per group
            let mut counts = Vec::with_capacity(chunk.len());
            let mut offset = 0;
            for group in chunk {
                let count = group.hash_to_index.len();
                counts.push(count);
                hash_to_index.extend(
                    group
                        .hash_to_index
                        .into_iter()
                        .map(|(hash, entity_index)| (hash, entity_index + offset)),
                );
                offset += count;
                let mut values = group.variables;
                values.push(("representations", group.representations));
                values_per_group.push(values);
            }
            let (operation, variables) = self.aliased_requests.write(template, values_per_group);
            requests.push(PreparedRequest {
                operation,
                variables,
                custom_scalar_paths: template.custom_scalar_paths(),
                merge: hash_to_index,
                unalias: Some(Unalias {
                    template,
                    groups: counts.len(),
                    // every response key sent the same representations
                    counts: vec![counts; template.response_keys.len()],
                }),
            });
        }

        requests
    }

    /// Adds the fetches of a `Parallel` node to `scope`, looking through the conditions that hold
    /// and nested `Parallel`s. Anything else goes to `after`.
    fn prepare_parallel_child<'wave>(
        &'wave self,
        node: &'exec PlanNode,
        data: &Value<'exec>,
        scope: &mut FuturesUnordered<
            BoxFuture<'wave, Result<ExecutionJob<'exec>, PlanExecutionError>>,
        >,
        after: &mut Vec<&'exec PlanNode>,
    ) {
        match node {
            PlanNode::Condition(condition_node) => {
                if let Some(next_node) =
                    condition_node_by_variables(condition_node, self.variable_values)
                {
                    self.prepare_parallel_child(next_node, data, scope, after);
                }
            }
            PlanNode::Parallel(parallel_node) => {
                for child in &parallel_node.nodes {
                    self.prepare_parallel_child(child, data, scope, after);
                }
            }
            PlanNode::Fetch(_) | PlanNode::Flatten(_) | PlanNode::BatchFetch(_) => {
                scope.extend(self.prepare_job_future(node, data));
            }
            node => after.push(node),
        }
    }

    /**
     * This function is sync, because we only need the immutable borrow of `ctx.data` to prepare the subgraph request,
     * and the actual execution of the subgraph request is done in `prepare_fetch_job` which is async.
     * So we do everything in sync with `ctx.data` and return futures for the actual execution of the subgraph requests.
     *
     * A node can produce no futures (a flatten node with no data) or many (one per chunk of
     * groups of contextual argument values, see `ContextAliasTemplate`).
     */
    fn prepare_job_future<'wave>(
        &'wave self,
        node: &'exec PlanNode,
        data: &Value<'exec>,
    ) -> Vec<BoxFuture<'wave, Result<ExecutionJob<'exec>, PlanExecutionError>>> {
        match node {
            PlanNode::Fetch(fetch_node) if !self.fetch_runs(fetch_node) => Vec::new(),
            PlanNode::Fetch(fetch_node) => vec![self
                .prepare_execution_job(PrepareExecutionJobOpts {
                    subgraph_name: &fetch_node.service_name,
                    variable_usages: fetch_node.variable_usages.as_ref(),
                    operation_name: self
                        .operation_name_factory
                        .generate(&fetch_node.service_name, fetch_node.id),
                    operation_kind: fetch_node.operation_kind.as_ref(),
                    operation: &fetch_node.operation,
                    output_rewrites: fetch_node.output_rewrites.as_deref(),
                    custom_scalar_paths: fetch_node.custom_scalar_paths.as_ref(),
                    raw_variable_values: None,
                    affected_path: None,
                })
                .boxed()],
            PlanNode::BatchFetch(batch_fetch_node) => {
                let waves = self.prepare_batch_fetch_job_state(
                    &batch_fetch_node.entity_batch,
                    data,
                    self.schema_metadata
                        .entity_keys
                        .of_subgraph(&batch_fetch_node.service_name),
                );

                if waves.is_empty() {
                    // All alias lists are empty, so nothing to fetch.
                    // We skip the network call to save time.
                    debug!(
                        target: targets::GRAPHQL_EXECUTION,
                        alias_count = batch_fetch_node.entity_batch.aliases.len(),
                        "Skipping batched entity fetch with no representations"
                    );
                }

                self.prepare_batch_requests(batch_fetch_node, waves)
                    .into_iter()
                    .map(|request| {
                        let PreparedRequest {
                            operation,
                            variables,
                            custom_scalar_paths,
                            merge: aliases,
                            unalias,
                        } = request;
                        self.prepare_execution_job(PrepareExecutionJobOpts {
                            subgraph_name: &batch_fetch_node.service_name,
                            variable_usages: batch_fetch_node.variable_usages.as_ref(),
                            operation_name: self
                                .operation_name_factory
                                .generate(&batch_fetch_node.service_name, batch_fetch_node.id),
                            operation_kind: batch_fetch_node.operation_kind.as_ref(),
                            operation,
                            output_rewrites: None,
                            custom_scalar_paths,
                            raw_variable_values: Some(variables),
                            affected_path: None,
                        })
                        .map_ok(move |fetch_job| {
                            let mut response = fetch_job.response();
                            if let Some(unalias) = unalias {
                                unalias.apply(&mut response);
                            }
                            ExecutionJob::BatchFetch {
                                operation: &batch_fetch_node.operation,
                                subgraph_name: &batch_fetch_node.service_name,
                                response,
                                aliases,
                            }
                        })
                        .boxed()
                    })
                    .collect()
            }
            PlanNode::Flatten(flatten_node) => {
                let fetch_node = match flatten_node.node.as_ref() {
                    PlanNode::Fetch(fetch_node) => fetch_node,
                    _ => return Vec::new(),
                };
                if !self.fetch_runs(fetch_node) {
                    return Vec::new();
                }

                // If there are no requirements in the node (no _entities call), then we only need to make
                // a regular call. This happens when we perform subgraph re-entry.
                // So we can just create the fetch step future with the info we have
                let Some(requires_nodes) = fetch_node.requires.as_ref() else {
                    return vec![self.prepare_root_reentry_fetch_job(flatten_node, fetch_node)];
                };

                let (groups, mut path_hashes) = collect_context_entities(
                    data,
                    [flatten_node.path.as_ref()],
                    requires_nodes,
                    fetch_node.input_rewrites.as_deref(),
                    &fetch_node.context,
                    &self.schema_metadata.possible_types,
                    self.schema_metadata
                        .entity_keys
                        .of_subgraph(&fetch_node.service_name),
                );
                let representation_hashes = path_hashes.pop().unwrap_or_default();

                self.prepare_flatten_requests(fetch_node, groups)
                    .into_iter()
                    .map(|request| {
                        let PreparedRequest {
                            operation,
                            variables,
                            custom_scalar_paths,
                            merge: representation_hash_to_index,
                            unalias,
                        } = request;
                        let representation_hashes = Arc::clone(&representation_hashes);
                        self.prepare_execution_job(PrepareExecutionJobOpts {
                            subgraph_name: &fetch_node.service_name,
                            variable_usages: fetch_node.variable_usages.as_ref(),
                            operation_name: self
                                .operation_name_factory
                                .generate(&fetch_node.service_name, fetch_node.id),
                            operation_kind: fetch_node.operation_kind.as_ref(),
                            operation,
                            output_rewrites: fetch_node.output_rewrites.as_deref(),
                            custom_scalar_paths,
                            raw_variable_values: Some(variables),
                            affected_path: Some(flatten_node.path.as_ref()),
                        })
                        .map_ok(move |fetch_job| {
                            let mut response = fetch_job.response();
                            if let Some(unalias) = unalias {
                                unalias.apply(&mut response);
                            }
                            ExecutionJob::FlattenFetch {
                                operation: &fetch_node.operation,
                                flatten_node_path: &flatten_node.path,
                                response,
                                subgraph_name: fetch_node.service_name.as_str(),
                                representation_hashes,
                                representation_hash_to_index,
                                output_rewrites: fetch_node.output_rewrites.as_deref(),
                            }
                        })
                        .boxed()
                    })
                    .collect()
            }
            // callers look through conditions and `Parallel`s and run `Sequence`s themselves (see
            // `prepare_parallel_child`). `Subscription` and `Defer` don't run here.
            _ => Vec::new(),
        }
    }

    // We handle `Result` instead of passing `PlanExecutionError` directly
    // as PipelineError so the first occurrence of an error does not stop the whole execution
    // But those errors are added to the final GraphQL response in `errors` field
    // of the GraphQL response
    // For example, if a subgraph is down, the rest of the plan can still be executed
    // See `error_handling_e2e_tests` for reproduction
    fn process_job_result(
        &self,
        ctx: &mut ExecutionContext<'exec>,
        job: Result<ExecutionJob<'exec>, PlanExecutionError>,
    ) {
        match job {
            Err(ref err) => {
                if let (Some(subgraph_name), Some(subgraph_headers)) = (
                    err.subgraph_name().as_deref(),
                    err.subgraph_response_headers(),
                ) {
                    if let Err(ref propagation_err) = apply_subgraph_response_headers(
                        self.headers_plan,
                        subgraph_name,
                        subgraph_headers,
                        self.client_request,
                        &mut ctx.response_headers_aggregator,
                    )
                    .with_plan_context(LazyPlanContext {
                        subgraph_name: || Some(subgraph_name.to_string()),
                        affected_path: || err.affected_path().clone(),
                    }) {
                        self.log_error(propagation_err);
                        ctx.errors.push(propagation_err.into());
                    }
                }

                self.log_error(err);
                ctx.errors.push(err.into());
            }
            Ok(job) => {
                let subgraph_name = job.subgraph_name();
                let affected_path = job.affected_path();

                if let Some(demand_control) = &self.demand_control_context {
                    demand_control.record_subgraph_response_cost(
                        &mut ctx.subgraph_response_cost_tracker,
                        &job,
                        self.variable_values,
                    );
                }

                if let Some(ref subgraph_headers) = job.response_ref().headers {
                    if let Err(ref err) = apply_subgraph_response_headers(
                        self.headers_plan,
                        job.subgraph_name(),
                        subgraph_headers,
                        self.client_request,
                        &mut ctx.response_headers_aggregator,
                    )
                    .with_plan_context(LazyPlanContext {
                        subgraph_name: || Some(subgraph_name.to_string()),
                        affected_path: || affected_path.map(|p| p.to_string()),
                    }) {
                        self.log_error(err);
                        ctx.errors.push(err.into());
                    }
                }

                if let Some(ref subgraph_extensions) = job.response_ref().extensions {
                    apply_subgraph_extensions(
                        self.extensions_plan,
                        subgraph_extensions,
                        &mut ctx.extensions_aggregator,
                    );
                }

                match job {
                    ExecutionJob::Fetch {
                        mut response,
                        output_rewrites,
                        merge_path,
                        ..
                    } => {
                        if let Some(response_bytes) = response.bytes {
                            ctx.response_storage.add_response(response_bytes);
                        }
                        if let Some(output_rewrites) = output_rewrites {
                            for output_rewrite in output_rewrites {
                                output_rewrite.rewrite(
                                    &self.schema_metadata.possible_types,
                                    &mut response.data,
                                );
                            }
                        }

                        match merge_path {
                            // Root fetch
                            None => deep_merge(&mut ctx.data, response.data),
                            // Root re-entry
                            Some(merge_path) => {
                                let source = response.data;
                                traverse_and_callback_mut(
                                    &mut ctx.data,
                                    merge_path.as_slice(),
                                    self.schema_metadata,
                                    None,
                                    &mut |target, _error_path| {
                                        deep_merge_from(target, &source);
                                    },
                                );
                            }
                        }

                        ctx.handle_errors(subgraph_name, affected_path, response.errors, None);
                    }
                    ExecutionJob::FlattenFetch {
                        mut response,
                        flatten_node_path,
                        representation_hashes,
                        ref representation_hash_to_index,
                        output_rewrites,
                        ..
                    } => {
                        if let Some(response_bytes) = response.bytes {
                            ctx.response_storage.add_response(response_bytes);
                        }
                        if let Some(mut entities) = response.data.take_entities() {
                            if let Some(output_rewrites) = output_rewrites {
                                for output_rewrite in output_rewrites {
                                    for entity in &mut entities {
                                        output_rewrite
                                            .rewrite(&self.schema_metadata.possible_types, entity);
                                    }
                                }
                            }

                            let mut index = 0;
                            let normalized_path = flatten_node_path.as_slice();
                            // If there is an error in the response, then collect the paths for normalizing the error
                            let initial_error_path = response.errors.as_ref().map(|_| {
                                GraphQLErrorPath::with_capacity(normalized_path.len() + 2)
                            });
                            let mut entity_index_error_map = response
                                .errors
                                .as_ref()
                                .map(|_| HashMap::with_capacity(entities.len()));
                            let mut merger = EntityMerger::new(
                                &mut entities,
                                representation_hashes.iter(),
                                representation_hash_to_index,
                            );
                            traverse_and_callback_mut(
                                &mut ctx.data,
                                normalized_path,
                                self.schema_metadata,
                                initial_error_path,
                                &mut |target, error_path| {
                                    let hash = representation_hashes.get(index).copied();
                                    index += 1;

                                    let Some(Some(hash)) = hash else {
                                        return;
                                    };

                                    if let Some(entity_index) =
                                        representation_hash_to_index.get(&hash)
                                    {
                                        if let (Some(error_path), Some(entity_index_error_map)) =
                                            (error_path, entity_index_error_map.as_mut())
                                        {
                                            let error_paths = entity_index_error_map
                                                .entry(entity_index)
                                                .or_insert_with(Vec::new);
                                            error_paths.push(error_path);
                                        }
                                        merger.merge_into(target, *entity_index);
                                    }
                                },
                            );

                            ctx.handle_errors(
                                subgraph_name,
                                affected_path,
                                response.errors,
                                entity_index_error_map,
                            );
                        } else {
                            ctx.handle_errors(subgraph_name, affected_path, response.errors, None);
                        }
                    }
                    ExecutionJob::BatchFetch {
                        mut response,
                        aliases,
                        ..
                    } => {
                        if let Some(response_bytes) = response.bytes {
                            ctx.response_storage.add_response(response_bytes);
                        }

                        // Split errors by alias
                        let mut errors =
                            self.partition_batch_errors_by_alias(&aliases, response.errors.take());
                        // Take returned entities per alias
                        let mut entities_by_alias =
                            Self::collect_batched_entities_by_alias(&mut response.data, &aliases);

                        for (alias_index, alias_state) in aliases.iter().enumerate() {
                            let alias_index = AliasIndex(alias_index);
                            let mut alias_errors = errors.by_alias_index.remove(&alias_index);
                            // Merge entities back into execution context (final data)
                            // and attach alias errors and unmatched errors
                            let entity_index_error_map = self.merge_batch_alias_entities(
                                ctx,
                                alias_state,
                                entities_by_alias.get_mut(&alias_index),
                                alias_errors.as_deref(),
                            );

                            let affected_path = if alias_state.paths.len() == 1 {
                                Some(alias_state.paths[0].merge_path)
                            } else {
                                None
                            };

                            // Attach alias errors
                            ctx.handle_errors(
                                subgraph_name,
                                affected_path,
                                alias_errors.take(),
                                entity_index_error_map,
                            );
                        }

                        // Attach errors that do not point to any known alias.
                        if !errors.unmatched.is_empty() {
                            ctx.handle_errors(subgraph_name, None, Some(errors.unmatched), None);
                        }

                        tracing::trace!(
                            target: targets::GRAPHQL_EXECUTION,
                            alias_count = aliases.len(),
                            "Patched entity batch alias results"
                        );
                    }
                }
            }
        }
    }

    fn log_error(&self, error: &PlanExecutionError) {
        log_plan_execution_error(error);
    }

    fn partition_batch_errors_by_alias(
        &self,
        aliases: &[AliasBatchState<'exec>],
        response_errors: Option<Vec<GraphQLError>>,
    ) -> BatchFetchErrors {
        // Split subgraph errors into:
        // - errors that belong to a known alias
        // - errors that do not match any alias
        let mut alias_index_by_name: AHashMap<&str, AliasIndex> =
            AHashMap::with_capacity(aliases.len());
        for (alias_index, alias_state) in aliases.iter().enumerate() {
            alias_index_by_name.insert(
                alias_state.alias_spec.alias.as_str(),
                AliasIndex(alias_index),
            );
        }

        let mut errors_by_alias_index: AHashMap<AliasIndex, Vec<GraphQLError>> = AHashMap::new();
        let mut unmatched_errors: Vec<GraphQLError> = Vec::new();

        let Some(response_errors) = response_errors else {
            return BatchFetchErrors::default();
        };

        for mut error in response_errors {
            let maybe_alias = error.path.as_ref().and_then(|path| {
                path.segments.first().and_then(|segment| match segment {
                    GraphQLErrorPathSegment::String(alias) => Some(alias.as_str()),
                    _ => None,
                })
            });

            let Some(alias) = maybe_alias else {
                unmatched_errors.push(error);
                continue;
            };

            let Some(alias_index) = alias_index_by_name.get(alias) else {
                unmatched_errors.push(error);
                continue;
            };

            if let Some(path) = error.path.as_mut() {
                // Subgraph batch errors use alias names like "_e0".
                // Our error normalizer (GraphQLError::normalize_entity_error)
                // expects paths like ["_entities", index, ...].
                // So we replace the first path segment with "_entities".
                // Before: ["_e0", 2, "price"]
                // After : ["_entities", 2, "price"]
                if let Some(GraphQLErrorPathSegment::String(first)) = path.segments.first_mut() {
                    *first = "_entities".to_string();
                }
            }

            errors_by_alias_index
                .entry(*alias_index)
                .or_default()
                .push(error);
        }

        BatchFetchErrors {
            by_alias_index: errors_by_alias_index,
            unmatched: unmatched_errors,
        }
    }

    fn collect_batched_entities_by_alias(
        response_data: &mut Value<'exec>,
        aliases: &[AliasBatchState<'exec>],
    ) -> AHashMap<AliasIndex, Vec<Value<'exec>>> {
        // Take entity arrays from response data once per alias.
        // This avoids repeated lookups/mutations on response data.
        let mut entities_by_alias: AHashMap<AliasIndex, Vec<Value<'exec>>> =
            AHashMap::with_capacity(aliases.len());

        for (alias_index, alias_state) in aliases.iter().enumerate() {
            let Some(entities) =
                response_data.take_entities_by_key(alias_state.alias_spec.alias.as_str())
            else {
                continue;
            };
            entities_by_alias.insert(AliasIndex(alias_index), entities);
        }

        entities_by_alias
    }

    /// Merge one alias's returned entities back into `ctx.data`.
    fn merge_batch_alias_entities<'alias>(
        &self,
        ctx: &mut ExecutionContext<'exec>,
        alias_state: &'alias AliasBatchState<'exec>,
        entities: Option<&mut Vec<Value<'exec>>>,
        alias_errors: Option<&[GraphQLError]>,
    ) -> Option<HashMap<&'alias usize, Vec<GraphQLErrorPath>>> {
        let has_alias_errors = alias_errors.is_some();
        let mut entity_index_error_map = has_alias_errors.then(HashMap::new);
        let Some(entities) = entities else {
            return entity_index_error_map;
        };

        if let Some(output_rewrites) = alias_state.alias_spec.output_rewrites.as_ref() {
            for output_rewrite in output_rewrites {
                for entity in entities.iter_mut() {
                    output_rewrite.rewrite(&self.schema_metadata.possible_types, entity);
                }
            }
        }

        if alias_state.representation_hash_to_index.is_empty() {
            return entity_index_error_map;
        }

        let mut merger = EntityMerger::new(
            entities,
            alias_state
                .paths
                .iter()
                .flat_map(|path_state| path_state.representation_hashes.iter()),
            &alias_state.representation_hash_to_index,
        );

        // We walk each merge path
        for path_state in &alias_state.paths {
            let mut index = 0;
            let normalized_path = path_state.merge_path;
            let initial_error_path = has_alias_errors
                // Small extra capacity for path segments that will be appended later.
                .then(|| GraphQLErrorPath::with_capacity(normalized_path.len() + 2));

            // For each visited target:
            traverse_and_callback_mut(
                &mut ctx.data,
                normalized_path.as_slice(),
                self.schema_metadata,
                initial_error_path,
                &mut |target_data, error_path| {
                    let hash = path_state.representation_hashes.get(index).copied();
                    index += 1;

                    let Some(Some(hash)) = hash else {
                        return;
                    };

                    // Find matching entity index from hash->index map
                    if let Some(entity_index) = alias_state.representation_hash_to_index.get(&hash)
                    {
                        // If this alias has errors, we also collect target paths
                        // so one subgraph error can be copied to all matching targets.
                        if let (Some(error_path), Some(entity_index_error_map)) =
                            (error_path, entity_index_error_map.as_mut())
                        {
                            let error_paths = entity_index_error_map
                                .entry(entity_index)
                                .or_insert_with(Vec::new);
                            error_paths.push(error_path);
                        }
                        merger.merge_into(target_data, *entity_index);
                    }
                },
            );
        }

        entity_index_error_map
    }

    // The preperation includes:
    // - building one `_entities` input list for each representations variable
    // - remembering where each item came from (so we can put results back)
    // - telling the subgraph which aliases run
    //
    // Returns one request ("wave") per group of contextual argument values, without contexts
    // that's at most one.
    #[allow(clippy::type_complexity)]
    fn prepare_batch_fetch_job_state(
        &self,
        entity_batch: &'exec EntityBatch,
        data: &Value<'exec>,
        subgraph_keys: Option<&KeysByType>,
    ) -> Vec<BatchWave<'exec>> {
        // What each representations variable collected, by the first alias that runs with it.
        // An alias collects from the paths of the groups that hold, and with none of them
        // holding it doesn't run and collects nothing. Aliases that share a variable have the
        // same `requires`, context and paths, and either one group each or the same groups
        // (maybe in another order), so whenever two of them run they collect from the same
        // paths, and the others reuse what the first one collected. A variable is only ever
        // sent once that way.
        let variable_is_true = |name: &str| variable_is_true(self.variable_values, name);
        let mut collected = Vec::new();
        let mut collected_by: AHashMap<&'exec str, usize> =
            AHashMap::with_capacity(entity_batch.aliases.len());
        let mut sources: Vec<Option<usize>> = Vec::with_capacity(entity_batch.aliases.len());
        let mut active_groups: Vec<Vec<usize>> = Vec::with_capacity(entity_batch.aliases.len());
        for alias_spec in &entity_batch.aliases {
            let active: Vec<usize> = alias_spec
                .path_groups
                .iter()
                .enumerate()
                .filter(|(_, group)| group.is_active(variable_is_true))
                .map(|(index, _)| index)
                .collect();
            if active.is_empty() {
                sources.push(None);
                active_groups.push(active);
                continue;
            }
            let source = *collected_by
                .entry(alias_spec.representations_variable_name.as_str())
                .or_insert_with(|| {
                    collected.push(collect_context_entities(
                        data,
                        active_paths(alias_spec, &active),
                        &alias_spec.requires,
                        alias_spec.input_rewrites.as_deref(),
                        &alias_spec.context,
                        &self.schema_metadata.possible_types,
                        subgraph_keys,
                    ));
                    collected.len() - 1
                });
            sources.push(Some(source));
            active_groups.push(active);
        }

        // Everything the operation declares, for what no running alias sends: an empty list for
        // representations, null for context values.
        let mut declared: Vec<(&'exec str, &'static [u8])> = Vec::new();
        for alias_spec in &entity_batch.aliases {
            for name in FetchContext::group_variables(&alias_spec.context) {
                declared.push((name, b"null"));
            }
            declared.push((alias_spec.representations_variable_name.as_str(), b"[]"));
        }

        let wave_count = collected
            .iter()
            .map(|(groups, _)| groups.len())
            .max()
            .unwrap_or_default();
        let mut waves = Vec::with_capacity(wave_count);
        for wave in 0..wave_count {
            let mut raw_variable_values: Vec<(&'exec str, Vec<u8>)> =
                Vec::with_capacity(entity_batch.aliases.len());
            let mut aliases: Vec<AliasBatchState<'exec>> =
                Vec::with_capacity(entity_batch.aliases.len());
            // where the alias that sends each collected variable sits in `aliases`
            let mut sent_by: Vec<Option<usize>> = vec![None; collected.len()];
            for ((alias_spec, source), active) in entity_batch
                .aliases
                .iter()
                .zip(&sources)
                .zip(&active_groups)
            {
                if let Some(run_variable_name) = &alias_spec.run_variable_name {
                    let run: &[u8] = if source.is_some() { b"true" } else { b"false" };
                    raw_variable_values.push((run_variable_name.as_str(), run.to_vec()));
                }

                // The alias won't run, so it gets nothing back.
                let Some(source) = *source else {
                    aliases.push(AliasBatchState {
                        alias_spec,
                        representation_hash_to_index: Default::default(),
                        paths: Vec::new(),
                    });
                    continue;
                };

                if let Some(sender) = sent_by[source] {
                    let sender = &aliases[sender];
                    debug_assert!(
                        active_paths(alias_spec, active).count() == sender.paths.len()
                            && active_paths(alias_spec, active).all(|path| sender
                                .paths
                                .iter()
                                .any(|sent| sent.merge_path == path)),
                        "aliases that share representations must collect them from the same paths"
                    );
                    aliases.push(AliasBatchState {
                        alias_spec,
                        representation_hash_to_index: Arc::clone(
                            &sender.representation_hash_to_index,
                        ),
                        paths: sender.paths.clone(),
                    });
                    continue;
                }
                sent_by[source] = Some(aliases.len());

                let (groups, path_hashes) = &mut collected[source];
                // a variable with fewer groups still needs its context variables, it just sends
                // no representations. it borrows them from its first group, so that one is
                // copied, every other group is read once and can be moved out.
                let has_fewer_groups = groups.len() < wave_count;
                if let Some(group) = groups.get_mut(wave) {
                    if wave == 0 && has_fewer_groups {
                        raw_variable_values.extend(group.variables.iter().cloned());
                    } else {
                        raw_variable_values.extend(std::mem::take(&mut group.variables));
                    }
                } else if let Some(first) = groups.first() {
                    raw_variable_values.extend(first.variables.iter().cloned());
                }
                let mut group = groups.get_mut(wave);
                let representations = group
                    .as_mut()
                    .map(|group| std::mem::take(&mut group.representations))
                    .unwrap_or_else(|| b"[]".to_vec());
                raw_variable_values.push((
                    alias_spec.representations_variable_name.as_str(),
                    representations,
                ));
                aliases.push(AliasBatchState {
                    alias_spec,
                    representation_hash_to_index: Arc::new(
                        group
                            .map(|group| std::mem::take(&mut group.hash_to_index))
                            .unwrap_or_default(),
                    ),
                    paths: active_paths(alias_spec, active)
                        .zip(path_hashes.iter())
                        .map(|(merge_path, representation_hashes)| AliasPathState {
                            merge_path,
                            representation_hashes: Arc::clone(representation_hashes),
                        })
                        .collect(),
                });
            }

            let mut sent: AHashSet<&str> =
                raw_variable_values.iter().map(|(name, _)| *name).collect();
            for (name, value) in &declared {
                if sent.insert(name) {
                    raw_variable_values.push((name, value.to_vec()));
                }
            }

            if aliases
                .iter()
                .any(|alias| !alias.representation_hash_to_index.is_empty())
            {
                waves.push((raw_variable_values, aliases));
            }
        }

        waves
    }

    async fn prepare_execution_job(
        &self,
        opts: PrepareExecutionJobOpts<'exec>,
    ) -> Result<ExecutionJob<'exec>, PlanExecutionError> {
        let subgraph_operation_span =
            GraphQLSubgraphOperationSpan::new(opts.subgraph_name, &opts.operation.document_str);

        async {
            // TODO: We could optimize header map creation by caching them per service name
            let mut headers_map = HeaderMap::new();
            let subgraph_name_factory = || Some(opts.subgraph_name.to_string());
            let affected_path_factory = || opts.affected_path.map(|p| p.to_string());
            modify_subgraph_request_headers(
                self.headers_plan,
                opts.subgraph_name,
                self.client_request,
                &mut headers_map,
            )
            .with_plan_context(LazyPlanContext {
                subgraph_name: subgraph_name_factory,
                affected_path: affected_path_factory,
            })?;
            let variable_refs = select_fetch_variables(self.variable_values, opts.variable_usages);

            let mut subgraph_request = SubgraphExecutionRequest {
                query: &opts.operation.document_str,
                document_name_write_pos: opts.operation.name_write_position,
                dedupe: self.dedupe_subgraph_requests,
                operation_name: opts.operation_name,
                variables: variable_refs,
                raw_variable_values: opts.raw_variable_values,
                headers: headers_map,
                extensions: None,
                custom_scalar_paths: opts.custom_scalar_paths,
                connection_fingerprint: self.connection_fingerprint,
            };

            let client_document_hash_str = opts.operation.hash.to_string();
            subgraph_operation_span.record_operation_identity(GraphQLSpanOperationIdentity {
                name: subgraph_request.operation_name.as_deref(),
                operation_type: match opts.operation_kind {
                    Some(OperationKind::Query) | None => "query",
                    Some(OperationKind::Mutation) => "mutation",
                    Some(OperationKind::Subscription) => "subscription",
                },
                client_document_hash: &client_document_hash_str,
            });

            if let Some(jwt_forwarding_plan) = &self.jwt_forwarding_plan {
                subgraph_request.add_request_extensions_field(
                    jwt_forwarding_plan.extension_field_name.clone(),
                    jwt_forwarding_plan.extension_field_value.clone(),
                );
            }

            let mut response = self
                .executors
                .execute(
                    opts.subgraph_name,
                    subgraph_request,
                    self.client_request,
                    self.plugin_req_state,
                    self.demand_control_context.as_deref(),
                )
                .await
                .with_plan_context(LazyPlanContext {
                    subgraph_name: subgraph_name_factory,
                    affected_path: affected_path_factory,
                })?;

            if let Some(errors) = &response.errors {
                if !errors.is_empty() {
                    subgraph_operation_span.record_error_count(errors.len());
                    subgraph_operation_span
                        .record_errors(|| errors.iter().map(|e| e.into()).collect());
                }
            }

            if let Some(status) = &response.status {
                if !status.is_success() {
                    response.append_error(GraphQLError::create_from_status_code(
                        status,
                        opts.subgraph_name,
                    ));
                }
            }

            Ok(ExecutionJob::Fetch {
                subgraph_name: opts.subgraph_name,
                operation: opts.operation,
                response,
                output_rewrites: opts.output_rewrites,
                merge_path: opts.affected_path,
            })
        }
        .inspect_err(|err: &PlanExecutionError| {
            subgraph_operation_span.record_error_count(1);
            subgraph_operation_span
                .record_errors(|| vec![ObservedError::from(&GraphQLError::from(err))]);
        })
        .instrument(subgraph_operation_span.clone())
        .await
    }
}

/// Merges entities from an `_entities` response into the spots that asked for them.
/// Most entities land in more than one spot, so every spot gets a clone
/// except the last one, which gets the entity itself.
struct EntityMerger<'e, 'exec> {
    entities: &'e mut [Value<'exec>],
    remaining_uses: Vec<usize>,
}

impl<'e, 'exec> EntityMerger<'e, 'exec> {
    fn new<'h>(
        entities: &'e mut [Value<'exec>],
        representation_hashes: impl IntoIterator<Item = &'h Option<u64>>,
        representation_hash_to_index: &AHashMap<u64, usize>,
    ) -> Self {
        let mut remaining_uses = vec![0; entities.len()];
        for hash in representation_hashes.into_iter().flatten() {
            if let Some(remaining) = representation_hash_to_index
                .get(hash)
                .and_then(|entity_index| remaining_uses.get_mut(*entity_index))
            {
                *remaining += 1;
            }
        }
        Self {
            entities,
            remaining_uses,
        }
    }

    fn merge_into<'target>(&mut self, target: &mut Value<'target>, entity_index: usize)
    where
        'exec: 'target,
    {
        let (Some(entity), Some(remaining)) = (
            self.entities.get_mut(entity_index),
            self.remaining_uses.get_mut(entity_index),
        ) else {
            return;
        };
        let last_use = remaining.checked_sub(1).is_some_and(|left| {
            *remaining = left;
            left == 0
        });
        if last_use {
            deep_merge(target, std::mem::take(entity));
        } else {
            deep_merge_from(target, entity);
        }
    }
}

fn condition_node_by_variables<'a>(
    condition_node: &'a ConditionNode,
    variable_values: &'a Option<VariablesMap>,
) -> Option<&'a PlanNode> {
    if variable_is_true(variable_values, &condition_node.condition)? {
        condition_node.if_clause.as_deref()
    } else {
        condition_node.else_clause.as_deref()
    }
}

/// `Some(true)` for a `true` variable, `Some(false)` for any other value (`null` too) and `None`
/// when it's missing. Conditions read variables through this, so they all agree: a missing
/// variable runs neither side of a `Condition` node, and fails every condition group.
fn variable_is_true(variable_values: &Option<VariablesMap>, name: &str) -> Option<bool> {
    variable_values
        .as_ref()?
        .get(name)
        .map(|value| matches!(value.as_ref(), ValueRef::Bool(true)))
}

/// The paths of the groups in `active`.
fn active_paths<'a: 'b, 'b>(
    alias: &'a EntityBatchAlias,
    active: &'b [usize],
) -> impl Iterator<Item = ResponsePathRef<'a>> + 'b {
    active
        .iter()
        .flat_map(|&group| alias.path_groups[group].paths.iter())
}

fn select_fetch_variables<'a>(
    variable_values: &'a Option<VariablesMap>,
    variable_usages: Option<&BTreeSet<String>>,
) -> Option<HashMap<&'a str, &'a sonic_rs::Value>> {
    let values = variable_values.as_ref()?;

    variable_usages.map(|variable_usages| {
        variable_usages
            .iter()
            .filter_map(|var_name| {
                values
                    .get_key_value(var_name.as_str())
                    .map(|(key, value)| (key.as_str(), value))
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use crate::executor::{
        execution::{
            client_request_details::{ClientRequestDetails, JwtRequestDetails, OperationDetails},
            operation_name::OperationNameFactory,
            plan::Executor,
        },
        execution_context::ExecutionContext,
        extensions::plan::ExtensionsPlan,
        headers::plan::HeaderRulesPlan,
        introspection::schema::SchemaMetadata,
        response::{
            graphql_error::{GraphQLErrorExtensions, GraphQLErrorPath},
            value::Value as ResponseValue,
        },
        SubgraphExecutorMap,
    };

    use super::select_fetch_variables;
    use crate::query_planner::{
        ast::{
            document::Document, operation::PlanningFetchOperation, requires::RequiresSelectionSet,
        },
        planner::plan_nodes::{
            ConditionGroup, EntityBatch, EntityBatchAlias, FetchNode, FlattenNodePath,
            ParallelNode, PathGroup, PathSegment, PlanNode, RunIf, TypeCondition,
        },
        utils::parsing::parse_operation,
    };
    use crate::telemetry::TelemetryContext;
    use dashmap::DashMap;
    use graphql_tools::parser::query::{self, Definition};
    use ntex::http::HeaderMap;
    use sonic_rs::Value;
    use std::{
        collections::{BTreeSet, HashMap},
        sync::{mpsc::channel, Arc},
        time::Duration,
        vec,
    };

    fn value_from_number(n: i32) -> Value {
        sonic_rs::from_str(&n.to_string()).unwrap()
    }

    fn parse_document(query: &str) -> Document {
        let document = parse_operation(query);
        let mut operation = None;
        let mut fragments = Vec::new();

        for definition in document.definitions {
            match definition {
                Definition::Operation(current_operation) => {
                    if operation.is_none() {
                        operation = Some(current_operation.into());
                    }
                }
                Definition::Fragment(fragment) => fragments.push(fragment.into()),
            }
        }

        Document {
            operation: operation.expect("operation definition should exist"),
            fragments,
        }
    }

    #[test]
    fn select_fetch_variables_only_used_variables() {
        let mut variable_values_map = HashMap::new();
        variable_values_map.insert("used".to_string(), value_from_number(1));
        variable_values_map.insert("unused".to_string(), value_from_number(2));
        let variable_values = Some(variable_values_map);

        let mut usages = BTreeSet::new();
        usages.insert("used".to_string());

        let selected = select_fetch_variables(&variable_values, Some(&usages)).unwrap();

        assert_eq!(selected.len(), 1);
        assert!(selected.contains_key("used"));
        assert!(!selected.contains_key("unused"));
    }

    #[test]
    fn select_fetch_variables_ignores_missing_usage_entries() {
        let mut variable_values_map = HashMap::new();
        variable_values_map.insert("present".to_string(), value_from_number(3));
        let variable_values = Some(variable_values_map);

        let mut usages = BTreeSet::new();
        usages.insert("present".to_string());
        usages.insert("missing".to_string());

        let selected = select_fetch_variables(&variable_values, Some(&usages)).unwrap();

        assert_eq!(selected.len(), 1);
        assert!(selected.contains_key("present"));
        assert!(!selected.contains_key("missing"));
    }

    #[test]
    fn select_fetch_variables_for_no_usage_entries() {
        let mut variable_values_map = HashMap::new();
        variable_values_map.insert("unused_1".to_string(), value_from_number(1));
        variable_values_map.insert("unused_2".to_string(), value_from_number(2));

        let variable_values = Some(variable_values_map);

        let selected = select_fetch_variables(&variable_values, None);

        assert!(selected.is_none());
    }
    #[test]
    /**
     * We have the same entity in two different paths ["a", 0] and ["b", 1],
     * and the subgraph response has an error for this entity.
     * So we should duplicate the error for both paths.
     */
    fn normalize_entity_errors_correctly() {
        use crate::executor::response::graphql_error::{GraphQLError, GraphQLErrorPathSegment};
        use std::collections::HashMap;
        let mut ctx = ExecutionContext::default();
        let mut entity_index_error_map: HashMap<&usize, Vec<GraphQLErrorPath>> = HashMap::new();
        entity_index_error_map.insert(
            &0,
            vec![
                GraphQLErrorPath {
                    segments: vec![
                        GraphQLErrorPathSegment::String("a".to_string()),
                        GraphQLErrorPathSegment::Index(0),
                    ],
                },
                GraphQLErrorPath {
                    segments: vec![
                        GraphQLErrorPathSegment::String("b".to_string()),
                        GraphQLErrorPathSegment::Index(1),
                    ],
                },
            ],
        );
        let response_errors = vec![GraphQLError {
            message: "Error 1".to_string(),
            locations: None,
            path: Some(GraphQLErrorPath {
                segments: vec![
                    GraphQLErrorPathSegment::String("_entities".to_string()),
                    GraphQLErrorPathSegment::Index(0),
                    GraphQLErrorPathSegment::String("field1".to_string()),
                ],
            }),
            extensions: GraphQLErrorExtensions::default(),
        }];
        ctx.handle_errors(
            "subgraph_a",
            None,
            Some(response_errors),
            Some(entity_index_error_map),
        );
        assert_eq!(ctx.errors.len(), 2);
        assert_eq!(ctx.errors[0].message, "Error 1");
        assert_eq!(
            ctx.errors[0].path.as_ref().unwrap().segments,
            vec![
                GraphQLErrorPathSegment::String("a".to_string()),
                GraphQLErrorPathSegment::Index(0),
                GraphQLErrorPathSegment::String("field1".to_string())
            ]
        );
        assert_eq!(ctx.errors[1].message, "Error 1");
        assert_eq!(
            ctx.errors[1].path.as_ref().unwrap().segments,
            vec![
                GraphQLErrorPathSegment::String("b".to_string()),
                GraphQLErrorPathSegment::Index(1),
                GraphQLErrorPathSegment::String("field1".to_string())
            ]
        );
    }

    /// Runs `f` with an executor for a single `inventory` subgraph.
    fn with_test_executor(
        variable_values: &Option<super::VariablesMap>,
        f: impl FnOnce(&Executor),
    ) {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let subgraph_endpoint_map = HashMap::from([(
            "inventory".to_string(),
            "http://example.com/graphql".parse().unwrap(),
        )]);

        let executors = SubgraphExecutorMap::from_http_endpoint_map(
            &subgraph_endpoint_map,
            Default::default(),
            Default::default(),
            Default::default(),
            None,
            Arc::new(TelemetryContext::from_propagation_config(
                &Default::default(),
                &Default::default(),
            )),
            Arc::new(DashMap::new()),
        )
        .unwrap();

        let executor = Executor {
            aliased_requests: &crate::executor::execution::context::AliasedRequests::default(),
            variable_values,
            schema_metadata: &SchemaMetadata::default(),
            executors: &executors,
            client_request: &ClientRequestDetails {
                method: &http::Method::POST,
                url: &"http://example.com".parse().unwrap(),
                headers: HeaderMap::new().into(),
                operation: OperationDetails {
                    name: None,
                    query: "{ products { upc } }",
                    kind: "query",
                },
                jwt: JwtRequestDetails::Unauthenticated.into(),
                path_params: Default::default(),
            },
            headers_plan: &HeaderRulesPlan::default(),
            extensions_plan: &ExtensionsPlan::default(),
            jwt_forwarding_plan: None,
            dedupe_subgraph_requests: false,
            demand_control_context: None,
            plugin_req_state: None,
            operation_name_factory: &OperationNameFactory::default(),
            connection_fingerprint: None,
        };

        f(&executor);
    }

    /// The `requires` of an entity call, converted the same way the planner converts it.
    fn requires_from(query: &str) -> RequiresSelectionSet {
        let document = parse_operation(query);
        let selection = document
            .definitions
            .iter()
            .find_map(|def| match def {
                query::Definition::Operation(query::OperationDefinition::SelectionSet(sel)) => {
                    Some(sel.clone())
                }
                _ => None,
            })
            .unwrap();
        let set: crate::query_planner::ast::selection_set::SelectionSet = selection.into();
        RequiresSelectionSet::from(&set)
    }

    /// An alias that reads `Product`s by `upc` at `merge_paths`.
    fn product_alias(
        alias: &str,
        representations_variable_name: &str,
        merge_paths: Vec<FlattenNodePath>,
    ) -> EntityBatchAlias {
        EntityBatchAlias {
            alias: alias.to_string(),
            representations_variable_name: representations_variable_name.to_string(),
            path_groups: vec![PathGroup {
                run_if: None,
                paths: merge_paths.into(),
            }],
            requires: requires_from("{ ... on Product { upc } }"),
            input_rewrites: None,
            output_rewrites: None,
            context: None,
            run_variable_name: None,
        }
    }

    /// One group with these variables, `true` for an include and `false` for a skip.
    fn run_if(literals: &[(&str, bool)]) -> Option<RunIf> {
        RunIf::any_of(ConditionGroup::from_literals(literals.iter().copied()))
    }

    /// `@` is a list step, `|A|B` a type condition, anything else a field.
    fn path(steps: &[&str]) -> FlattenNodePath {
        steps
            .iter()
            .map(|step| match *step {
                "@" => PathSegment::List,
                s if s.starts_with('|') => PathSegment::TypeCondition(Box::new(
                    TypeCondition::from_names(s.trim_start_matches('|').split('|')),
                )),
                field => PathSegment::Field(field.into()),
            })
            .collect::<Vec<_>>()
            .into()
    }

    #[test]
    fn prepare_batch_fetch_job_state_deduplicates_shared_variable_payloads() {
        let data: ResponseValue = sonic_rs::from_str(
            r#"{
                "products": [
                    {"__typename": "Product", "upc": "1"},
                    {"__typename": "Product", "upc": "2"}
                ]
            }"#,
        )
        .unwrap();
        let products = || vec![path(&["products", "@"])];
        let mut entity_batch = EntityBatch {
            aliases: vec![
                product_alias("_e0", "__batch_reps_0", products()),
                product_alias("_e1", "__batch_reps_0", products()),
            ],
        };

        with_test_executor(&None, |executor| {
            let waves = executor.prepare_batch_fetch_job_state(&entity_batch, &data, None);

            assert_eq!(waves.len(), 1);
            let (raw_variable_values, aliases) = &waves[0];
            assert_eq!(aliases.len(), 2);
            assert_eq!(raw_variable_values.len(), 1);
            assert_eq!(raw_variable_values[0].0, "__batch_reps_0");
        });

        // aliases under other contexts never share a representations variable, the planner keys
        // the variable by the context too
        entity_batch.aliases[1].representations_variable_name = "__batch_reps_1".to_string();
        for (index, alias) in entity_batch.aliases.iter_mut().enumerate() {
            alias.context = crate::query_planner::planner::plan_nodes::FetchContext::new(
                vec![format!("__context_{index}")],
                Some(Vec::new()),
            );
        }
        with_test_executor(&None, |executor| {
            let waves = executor.prepare_batch_fetch_job_state(&entity_batch, &data, None);
            assert_eq!(waves.len(), 1);
            assert!(waves[0].0.contains(&("__context_0", b"null".to_vec())));
            assert!(waves[0].0.contains(&("__context_1", b"null".to_vec())));
        });

        // _e0 gets one group per product and _e1 a single group, so _e1 has fewer groups than
        // there are waves and must send its first group's values in every wave
        use crate::query_planner::planner::plan_nodes::{FetchNodePathSegment, KeyRenamer};
        let data: ResponseValue = sonic_rs::from_str(
            r#"{
                "currency": "USD",
                "products": [
                    {"__typename": "Product", "upc": "1"},
                    {"__typename": "Product", "upc": "2"}
                ]
            }"#,
        )
        .unwrap();
        entity_batch.aliases[0].context =
            crate::query_planner::planner::plan_nodes::FetchContext::new(
                vec!["__context_0".into()],
                Some(vec![KeyRenamer {
                    path: vec![FetchNodePathSegment::Key("upc".into())],
                    rename_key_to: "__context_0".into(),
                }]),
            );
        entity_batch.aliases[1].context =
            crate::query_planner::planner::plan_nodes::FetchContext::new(
                vec!["__context_1".into()],
                Some(vec![KeyRenamer {
                    path: vec![
                        FetchNodePathSegment::Parent,
                        FetchNodePathSegment::Key("currency".into()),
                    ],
                    rename_key_to: "__context_1".into(),
                }]),
            );
        with_test_executor(&None, |executor| {
            let waves = executor.prepare_batch_fetch_job_state(&entity_batch, &data, None);
            assert_eq!(waves.len(), 2);
            for (wave, upc) in waves.iter().zip(["\"1\"", "\"2\""]) {
                assert!(wave.0.contains(&("__context_0", upc.as_bytes().to_vec())));
                assert!(wave.0.contains(&("__context_1", b"\"USD\"".to_vec())));
            }
        });
    }

    /// A folded path `items.@|[A|B].product` must collect exactly what `items.@|[A].product`
    /// and `items.@|[B].product` collect together: the products under `A` and `B`, each once,
    /// and nothing under `C`.
    #[test]
    fn folded_type_condition_collects_the_same_products_as_the_paths_it_replaces() {
        let data: ResponseValue = sonic_rs::from_str(
            r#"{
                "items": [
                    {"__typename": "A", "product": {"__typename": "Product", "upc": "1"}},
                    {"__typename": "B", "product": {"__typename": "Product", "upc": "2"}},
                    {"__typename": "C", "product": {"__typename": "Product", "upc": "3"}},
                    {"__typename": "A", "product": {"__typename": "Product", "upc": "1"}}
                ]
            }"#,
        )
        .unwrap();

        let folded = EntityBatch {
            aliases: vec![product_alias(
                "_e0",
                "__batch_reps_0",
                vec![path(&["items", "@", "|A|B", "product"])],
            )],
        };
        let unfolded = EntityBatch {
            aliases: vec![product_alias(
                "_e0",
                "__batch_reps_0",
                vec![
                    path(&["items", "@", "|A", "product"]),
                    path(&["items", "@", "|B", "product"]),
                ],
            )],
        };

        with_test_executor(&None, |executor| {
            let representations = |batch: &EntityBatch| {
                let waves = executor.prepare_batch_fetch_job_state(batch, &data, None);
                let (raw_variable_values, aliases) = &waves[0];
                let visited: usize = aliases[0]
                    .paths
                    .iter()
                    .map(|path| path.representation_hashes.len())
                    .sum();
                let raw = String::from_utf8(raw_variable_values[0].1.clone()).unwrap();
                (raw, visited)
            };

            let (folded_raw, folded_visited) = representations(&folded);
            let (unfolded_raw, unfolded_visited) = representations(&unfolded);

            assert_eq!(
                folded_raw,
                r#"[{"__typename":"Product","upc":"1"},{"__typename":"Product","upc":"2"}]"#
            );
            assert_eq!(folded_raw, unfolded_raw);
            // Three items match `A` or `B`, both ways visit each of them once.
            assert_eq!(folded_visited, 3);
            assert_eq!(unfolded_visited, 3);
        });
    }

    /// The variables a batch sends, as text.
    fn sent_variables(raw_variable_values: &[(&str, Vec<u8>)]) -> Vec<(String, String)> {
        let mut sent: Vec<(String, String)> = raw_variable_values
            .iter()
            .map(|(name, raw)| (name.to_string(), String::from_utf8(raw.clone()).unwrap()))
            .collect();
        sent.sort();
        sent
    }

    fn bool_variables(values: &[(&str, bool)]) -> Option<super::VariablesMap> {
        Some(
            values
                .iter()
                .map(|(name, value)| (name.to_string(), sonic_rs::to_value(value).unwrap()))
                .collect(),
        )
    }

    /// An alias whose condition is off collects nothing, and the subgraph is told to skip it.
    /// Its own representations variable gets an empty list. The alias next to it still collects
    /// its products.
    #[test]
    fn alias_with_condition_off_sends_an_empty_list() {
        let data: ResponseValue =
            sonic_rs::from_str(r#"{"products": [{"__typename": "Product", "upc": "1"}]}"#).unwrap();

        let mut conditional =
            product_alias("_e1", "__batch_reps_1", vec![path(&["products", "@"])]);
        conditional.path_groups[0].run_if = run_if(&[("show", true)]);
        conditional.run_variable_name = Some("__batch_run_0".to_string());
        let entity_batch = EntityBatch {
            aliases: vec![
                product_alias("_e0", "__batch_reps_0", vec![path(&["products", "@"])]),
                conditional,
            ],
        };

        with_test_executor(&bool_variables(&[("show", false)]), |executor| {
            let waves = executor.prepare_batch_fetch_job_state(&entity_batch, &data, None);
            assert_eq!(waves.len(), 1);
            let (raw_variable_values, aliases) = &waves[0];

            assert_eq!(
                sent_variables(raw_variable_values),
                [
                    (
                        "__batch_reps_0".to_string(),
                        r#"[{"__typename":"Product","upc":"1"}]"#.to_string()
                    ),
                    ("__batch_reps_1".to_string(), "[]".to_string()),
                    ("__batch_run_0".to_string(), "false".to_string()),
                ]
            );
            assert!(aliases[1].representation_hash_to_index.is_empty());
            assert!(aliases[1].paths.is_empty());
        });
    }

    /// `_e0` and `_e1` read the same products under different conditions, so they share one
    /// representations variable. It's collected once when any of them runs, and the run
    /// variables tell the subgraph which ones do. With both off there's no request at all.
    #[test]
    fn shared_representations_are_collected_once_for_the_aliases_that_run() {
        let data: ResponseValue = sonic_rs::from_str(
            r#"{"products": [{"__typename": "Product", "upc": "1"}, {"__typename": "Product", "upc": "2"}]}"#,
        )
        .unwrap();
        let products = r#"[{"__typename":"Product","upc":"1"},{"__typename":"Product","upc":"2"}]"#;

        let mut details = product_alias("_e0", "__batch_reps_0", vec![path(&["products", "@"])]);
        details.path_groups[0].run_if = run_if(&[("skipDetails", false)]);
        details.run_variable_name = Some("__batch_run_0".to_string());
        let mut inventory = product_alias("_e1", "__batch_reps_0", vec![path(&["products", "@"])]);
        inventory.path_groups[0].run_if = run_if(&[("withInventory", true)]);
        inventory.run_variable_name = Some("__batch_run_1".to_string());
        let entity_batch = EntityBatch {
            aliases: vec![details, inventory],
        };

        let cases = [
            // skipDetails, withInventory, _e0 runs, _e1 runs
            (false, true, true, true),
            (true, true, false, true),
            (false, false, true, false),
            (true, false, false, false),
        ];
        for (skip_details, with_inventory, details_runs, inventory_runs) in cases {
            let variables = bool_variables(&[
                ("skipDetails", skip_details),
                ("withInventory", with_inventory),
            ]);
            with_test_executor(&variables, |executor| {
                let waves = executor.prepare_batch_fetch_job_state(&entity_batch, &data, None);
                let context = format!("skipDetails={skip_details} withInventory={with_inventory}");

                if !details_runs && !inventory_runs {
                    assert!(waves.is_empty(), "{context}");
                    return;
                }
                assert_eq!(waves.len(), 1, "{context}");
                let (raw_variable_values, aliases) = &waves[0];

                assert_eq!(
                    sent_variables(raw_variable_values),
                    [
                        ("__batch_reps_0".to_string(), products.to_string()),
                        ("__batch_run_0".to_string(), details_runs.to_string()),
                        ("__batch_run_1".to_string(), inventory_runs.to_string()),
                    ],
                    "{context}"
                );
                // Only the aliases that run get the products back.
                for (alias, runs) in aliases.iter().zip([details_runs, inventory_runs]) {
                    assert_eq!(
                        alias.representation_hash_to_index.len(),
                        if runs { 2 } else { 0 }
                    );
                }
                // With both running, the second one reuses what the first one collected.
                if details_runs && inventory_runs {
                    assert!(Arc::ptr_eq(
                        &aliases[0].representation_hash_to_index,
                        &aliases[1].representation_hash_to_index
                    ));
                }
            });
        }
    }

    /// An alias collects only from the paths of its groups that hold, each product once, and
    /// an inactive path gets nothing merged back since it isn't walked at all. A `null`
    /// variable counts as `false` and a missing one fails its group, like `Condition` nodes.
    #[test]
    fn only_paths_of_groups_that_hold_are_collected() {
        let data: ResponseValue = sonic_rs::from_str(
            r#"{
                "products": [{"__typename": "Product", "upc": "1"}],
                "top": [{"__typename": "Product", "upc": "2"}],
                "extra": [{"__typename": "Product", "upc": "3"}, {"__typename": "Product", "upc": "1"}]
            }"#,
        )
        .unwrap();

        let mut alias = product_alias("_e0", "__batch_reps_0", vec![path(&["products", "@"])]);
        alias.path_groups.push(PathGroup {
            run_if: run_if(&[("a", true)]),
            paths: vec![path(&["top", "@"])].into(),
        });
        alias.path_groups.push(PathGroup {
            run_if: run_if(&[("b", false)]),
            paths: vec![path(&["extra", "@"])].into(),
        });
        let entity_batch = EntityBatch {
            aliases: vec![alias],
        };
        let product = |upc: &str| format!(r#"{{"__typename":"Product","upc":"{upc}"}}"#);

        let cases: [(&str, &[&str], usize); 5] = [
            // variables, upcs sent, paths walked
            (r#"{"a": false, "b": true}"#, &["1"], 1),
            (r#"{"a": true, "b": true}"#, &["1", "2"], 2),
            (r#"{"a": true, "b": false}"#, &["1", "2", "3"], 3),
            (r#"{"a": null, "b": null}"#, &["1", "3"], 2),
            (r#"{}"#, &["1"], 1),
        ];
        for (variables, upcs, walked) in cases {
            let variables = Some(sonic_rs::from_str(variables).unwrap());
            with_test_executor(&variables, |executor| {
                let waves = executor.prepare_batch_fetch_job_state(&entity_batch, &data, None);
                let (raw_variable_values, aliases) = &waves[0];
                let sent: Vec<String> = upcs.iter().map(|upc| product(upc)).collect();
                assert_eq!(
                    sent_variables(raw_variable_values),
                    [(
                        "__batch_reps_0".to_string(),
                        format!("[{}]", sent.join(","))
                    )],
                    "{variables:?}"
                );
                assert_eq!(aliases[0].paths.len(), walked, "{variables:?}");
            });
        }
    }

    /// Aliases with the same groups share a representations variable, here in another order.
    /// They hold together, so the second one reuses what the first one collected, and the
    /// variable is sent once.
    #[test]
    fn aliases_with_the_same_groups_reuse_one_collection() {
        let data: ResponseValue = sonic_rs::from_str(
            r#"{"products": [{"__typename": "Product", "upc": "1"}], "top": [{"__typename": "Product", "upc": "2"}]}"#,
        )
        .unwrap();
        let alias = |name: &str| {
            let mut alias = product_alias(name, "__batch_reps_0", vec![path(&["products", "@"])]);
            alias.path_groups[0].run_if = run_if(&[("a", true)]);
            alias.path_groups.push(PathGroup {
                run_if: run_if(&[("b", true)]),
                paths: vec![path(&["top", "@"])].into(),
            });
            alias.run_variable_name = Some(format!("__batch_run_{name}"));
            alias
        };
        let mut reversed = alias("_e1");
        reversed.path_groups.reverse();
        let entity_batch = EntityBatch {
            aliases: vec![alias("_e0"), reversed],
        };

        with_test_executor(&bool_variables(&[("a", false), ("b", true)]), |executor| {
            let waves = executor.prepare_batch_fetch_job_state(&entity_batch, &data, None);
            let (raw_variable_values, aliases) = &waves[0];
            assert_eq!(
                sent_variables(raw_variable_values),
                [
                    (
                        "__batch_reps_0".to_string(),
                        r#"[{"__typename":"Product","upc":"2"}]"#.to_string()
                    ),
                    ("__batch_run__e0".to_string(), "true".to_string()),
                    ("__batch_run__e1".to_string(), "true".to_string()),
                ]
            );
            assert!(Arc::ptr_eq(
                &aliases[0].representation_hash_to_index,
                &aliases[1].representation_hash_to_index
            ));
            assert_eq!(aliases[1].paths.len(), 1);
        });
    }

    /// A fetch whose `run_if` doesn't hold gets no job, so no request.
    #[test]
    fn fetch_with_run_if_off_has_no_job() {
        let fetch = |run_if| {
            PlanNode::Fetch(Box::new(FetchNode {
                id: 1,
                planner_requires: (),
                service_name: "inventory".to_string(),
                context: None,
                operation: PlanningFetchOperation::from_anonymous_operation(parse_document(
                    "query($a: Boolean!) { products @include(if: $a) { upc } }",
                ))
                .operation,
                custom_scalar_paths: None,
                requires: None,
                input_rewrites: None,
                output_rewrites: None,
                variable_usages: None,
                operation_kind: None,
                run_if,
            }))
        };
        let data = ResponseValue::Object(Vec::new());
        for (a, jobs) in [(true, 1), (false, 0)] {
            with_test_executor(&bool_variables(&[("a", a)]), |executor| {
                let node = fetch(run_if(&[("a", true)]));
                assert_eq!(
                    executor.prepare_job_future(&node, &data).len(),
                    jobs,
                    "a={a}"
                );
            });
        }
    }

    #[tokio::test]
    async fn runs_parallel_jobs_in_parallel() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let mut subgraph_a = mockito::Server::new_async().await;
        let mut subgraph_b = mockito::Server::new_async().await;
        let data = crate::executor::response::value::Value::Null;
        let subgraph_endpoint_map = HashMap::from([
            (
                "subgraph_a".to_string(),
                format!("http://{}/graphql", subgraph_a.host_with_port())
                    .parse()
                    .unwrap(),
            ),
            (
                "subgraph_b".to_string(),
                format!("http://{}/graphql", subgraph_b.host_with_port())
                    .parse()
                    .unwrap(),
            ),
        ]);
        let executor = Executor {
            aliased_requests: &crate::executor::execution::context::AliasedRequests::default(),
            variable_values: &None,
            schema_metadata: &SchemaMetadata::default(),
            executors: &SubgraphExecutorMap::from_http_endpoint_map(
                &subgraph_endpoint_map,
                Default::default(),
                Default::default(),
                Default::default(),
                None,
                Arc::new(TelemetryContext::from_propagation_config(
                    &Default::default(),
                    &Default::default(),
                )),
                Arc::new(DashMap::new()),
            )
            .unwrap(),
            client_request: &ClientRequestDetails {
                method: &http::Method::POST,
                url: &"http://example.com".parse().unwrap(),
                headers: HeaderMap::new().into(),
                operation: OperationDetails {
                    name: None,
                    query: "{ from_a from_b }",
                    kind: "query",
                },
                jwt: JwtRequestDetails::Unauthenticated.into(),
                path_params: Default::default(),
            },
            headers_plan: &HeaderRulesPlan::default(),
            extensions_plan: &ExtensionsPlan::default(),
            jwt_forwarding_plan: None,
            dedupe_subgraph_requests: false,
            demand_control_context: None,
            plugin_req_state: None,
            operation_name_factory: &OperationNameFactory::default(),
            connection_fingerprint: None,
        };

        let mock_a = subgraph_a
            .mock("POST", "/graphql")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"from_a":"value_a"}}"#)
            .create();

        let mut exec_ctx = ExecutionContext {
            data,
            ..Default::default()
        };

        // It is ok to have 'static lifetime here, because `data` is owned by `exec_ctx`, and `exec_ctx` lives for the entire duration of the test,
        // so the reference to `data` will never be dangling.
        let data_ref: &'static crate::executor::response::value::Value<'static> =
            unsafe { std::mem::transmute(&exec_ctx.data) };

        let (sender, receiver) = channel();

        let mock_b = subgraph_b
            .mock("POST", "/graphql")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_chunked_body(move |writer| {
                // We can add some delay here to make sure the parallel execution is actually working
                std::thread::sleep(Duration::from_millis(1000));
                // data should have `from_a` field from subgraph_a's response,
                // so data the merging process does not wait for subgraph_b's response to merge subgraph_a's response
                if let Some(data) = data_ref.as_object() {
                    let from_a_index = data.iter().position(|(k, _)| k == &"from_a");
                    let from_a_value = from_a_index
                        .and_then(|index| data.get(index))
                        .and_then(|(_, v)| v.as_str());
                    if let Some(from_a_value) = from_a_value {
                        sender
                            .send(from_a_value.to_string())
                            .expect("Failed to send from_a value through channel");
                    }
                }
                writer.write_fmt(format_args!(r#"{{"data":{{"from_b":"value_b"}}}}"#))
            })
            .create();

        executor
            .execute_plan_node(
                &mut exec_ctx,
                &PlanNode::Parallel(ParallelNode {
                    nodes: vec![
                        PlanNode::Fetch(Box::new(FetchNode {
                            id: 1,
                            planner_requires: (),
                            service_name: "subgraph_a".to_string(),
                            context: None,
                            operation: PlanningFetchOperation::from_anonymous_operation(
                                parse_document("{ from_a }"),
                            )
                            .operation,
                            custom_scalar_paths: None,
                            requires: None,
                            input_rewrites: None,
                            output_rewrites: None,
                            variable_usages: None,
                            operation_kind: None,
                            run_if: None,
                        })),
                        PlanNode::Fetch(Box::new(FetchNode {
                            id: 2,
                            planner_requires: (),
                            service_name: "subgraph_b".to_string(),
                            context: None,
                            operation: PlanningFetchOperation::from_anonymous_operation(
                                parse_document("{ from_b }"),
                            )
                            .operation,
                            custom_scalar_paths: None,
                            requires: None,
                            input_rewrites: None,
                            output_rewrites: None,
                            variable_usages: None,
                            operation_kind: None,
                            run_if: None,
                        })),
                    ],
                }),
            )
            .await;
        mock_a.assert();
        mock_b.assert();

        let from_a_value = receiver
            .recv()
            .expect("Failed to receive from_a value through channel");
        assert_eq!(from_a_value, "value_a");
    }
}
