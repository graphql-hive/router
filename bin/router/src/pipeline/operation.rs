//! The transport-independent part of serving a GraphQL operation.
//!
//! Every entry point (the HTTP endpoint, the WebSocket subscribe flow) handles its own
//! transport concerns - reading the body, CORS, connection handshakes, resolving the
//! operation's params - and then hands the prepared operation to [`execute_operation`],
//! which parses, validates, normalizes, plans and executes it.

use std::{ops::ControlFlow, sync::Arc, time::Instant};

use hive_console_sdk::agent::usage_agent::RequestDetails;
use http::Method;
use ntex::{http::HeaderMap, url::Url, web};
use tracing::{debug, warn};

use crate::{
    executor::{
        coprocessor::runtime::MutableRequestState, execution::client_request_details::PathParams,
        executors::inflight::InFlightRole, headers::response::ResponseHeaderSink,
        hooks::on_graphql_params::GraphQLParams, plugin_context::PluginRequestState,
        request_context::SharedRequestContext,
    },
    pipeline::{
        connection_fingerprint,
        error::{ClientPipelineError, PipelineError},
        execute_planned_request, hash_graphql_extensions, hash_graphql_variables,
        header::ResponseMode,
        inbound_request_fingerprint,
        long_lived_client_limit::{try_reserve_long_lived_client, LongLivedClientGuard},
        normalize::normalize_request_with_cache,
        parser::{parse_operation_with_cache, ParseResult},
        request_extensions::write_graphql_operation_metric_identity,
        usage_reporting,
        validation::validate_operation_with_cache,
    },
    query_planner::state::supergraph_state::OperationKind,
    schema_state::{SchemaState, SelectedSupergraph},
    shared_state::{RouterSharedState, SharedRouterResponse},
    telemetry::{
        logging::{summary, targets},
        traces::spans::graphql::GraphQLOperationSpan,
    },
};

/// A prepared operation, together with the request it arrived on.
///
/// `method`, `uri` and `headers` describe that request as the rest of the pipeline sees it
/// (JWT, header propagation, progressive override, dedupe, coprocessor stages). For
/// transports without a real HTTP request, the entry point fills them in synthetically.
pub(crate) struct OperationRequest<'exec> {
    pub method: &'exec Method,
    pub uri: &'exec Url,
    pub path_params: PathParams<'exec>,
    pub headers: HeaderMap,
    /// The operation's params, after the `on_graphql_params` hooks and persisted documents.
    pub graphql_params: GraphQLParams,
    pub resolved_document_id: Option<&'exec str>,
    pub client_name: Option<&'exec str>,
    pub client_version: Option<&'exec str>,
    pub supergraph: &'exec SelectedSupergraph,
    pub plugin_req_state: Option<PluginRequestState<'exec>>,
    pub request_context: &'exec SharedRequestContext,
    pub response_mode: &'exec ResponseMode,
    pub response_header_sink: ResponseHeaderSink,
    pub operation_span: GraphQLOperationSpan,
    /// When the operation started, for the usage report's duration.
    pub started_at: Instant,
    /// Sent with the usage report. Only needed when Hive usage reporting is enabled.
    pub usage_request_details: Option<RequestDetails>,
    /// Count subscriptions toward the long-lived client limit.
    pub reserve_long_lived_client: bool,
}

pub(crate) enum OperationOutcome {
    /// A plugin, a coprocessor or a limit ended the operation with its own response.
    EarlyResponse(web::HttpResponse),
    Executed {
        response: SharedRouterResponse,
        /// Held for as long as the client stream is open.
        long_lived_client_guard: Option<LongLivedClientGuard>,
    },
}

/// Learns the operation's identity as soon as it is known, so an entry point can attribute
/// metrics to it even when the operation fails later on.
pub(crate) trait OperationObserver {
    fn operation_identity(&self, name: Option<&str>, operation_type: Option<&'static str>);
}

impl OperationObserver for () {
    #[inline]
    fn operation_identity(&self, _: Option<&str>, _: Option<&'static str>) {}
}

impl OperationObserver for web::HttpRequest {
    #[inline]
    fn operation_identity(&self, name: Option<&str>, operation_type: Option<&'static str>) {
        write_graphql_operation_metric_identity(self, name.map(str::to_owned), operation_type);
    }
}

pub(crate) async fn execute_operation<'exec>(
    request: OperationRequest<'exec>,
    shared_state: &'exec Arc<RouterSharedState>,
    schema_state: &'exec Arc<SchemaState>,
    observer: &impl OperationObserver,
) -> Result<OperationOutcome, PipelineError> {
    let OperationRequest {
        method,
        uri,
        path_params,
        mut headers,
        mut graphql_params,
        resolved_document_id,
        client_name,
        client_version,
        supergraph,
        plugin_req_state,
        request_context,
        response_mode,
        response_header_sink,
        operation_span,
        started_at,
        usage_request_details,
        reserve_long_lived_client,
    } = request;

    observer.operation_identity(graphql_params.operation_name.as_deref(), None);

    let parser_result =
        parse_operation_with_cache(shared_state, &graphql_params, &plugin_req_state).await?;

    let mut parser_payload = match parser_result {
        ParseResult::Payload(payload) => payload,
        ParseResult::EarlyResponse(response) => {
            return Ok(OperationOutcome::EarlyResponse(response));
        }
    };

    summary::record(|s| {
        s.set_operation_name(parser_payload.operation_name.as_deref());
        s.set_operation_type(parser_payload.operation_type.as_str());
        s.set_operation_hash(Some(parser_payload.hive_operation_hash.as_str()));
    });

    operation_span.record_details(
        &parser_payload.minified_document,
        (&parser_payload).into(),
        client_name,
        client_version,
        &parser_payload.hive_operation_hash,
    );

    if let Some(response) = validate_operation_with_cache(
        supergraph,
        schema_state,
        shared_state,
        &parser_payload,
        &plugin_req_state,
    )
    .await?
    {
        return Ok(OperationOutcome::EarlyResponse(response));
    }

    request_context.update(|ctx| {
        ctx.operation.update(
            parser_payload.operation_name.clone(),
            Some(parser_payload.operation_type.clone()),
        );

        // Set initial state for progressive overrides in request context
        let progressive_overrides = &supergraph.snapshot.planner.supergraph.progressive_overrides;
        if !progressive_overrides.flags.is_empty() {
            ctx.progressive_override.unresolved_labels = Some(progressive_overrides.flags.clone());
        }
    })?;

    if let Some(coprocessor_runtime) = shared_state.coprocessor.as_ref() {
        let performed_mutations = match coprocessor_runtime
            .on_graphql_request(
                MutableRequestState {
                    method,
                    uri,
                    headers: &mut headers,
                },
                &mut graphql_params,
                request_context,
                || supergraph.snapshot.public_schema.sdl.clone(),
            )
            .await?
        {
            ControlFlow::Break(response) => return Ok(OperationOutcome::EarlyResponse(response)),
            ControlFlow::Continue(performed_mutations) => performed_mutations,
        };

        if performed_mutations.body {
            let parser_result =
                parse_operation_with_cache(shared_state, &graphql_params, &plugin_req_state)
                    .await?;

            parser_payload = match parser_result {
                ParseResult::Payload(payload) => payload,
                ParseResult::EarlyResponse(response) => {
                    return Ok(OperationOutcome::EarlyResponse(response));
                }
            };

            request_context.update(|ctx| {
                ctx.operation.update(
                    parser_payload.operation_name.clone(),
                    Some(parser_payload.operation_type.clone()),
                );
            })?;
        }
    }

    let normalize_payload = normalize_request_with_cache(
        &supergraph.snapshot,
        &supergraph.runtime,
        schema_state,
        &graphql_params,
        &parser_payload,
    )
    .await?;

    observer.operation_identity(
        normalize_payload.operation_identity.name.as_deref(),
        Some(normalize_payload.operation_identity.operation_type.as_str()),
    );

    // Update the request context if the operation name or type has changed
    if parser_payload.operation_name.as_ref() != normalize_payload.operation_identity.name.as_ref()
        || parser_payload.operation_type != normalize_payload.operation_identity.operation_type
    {
        request_context.update(|ctx| {
            ctx.operation.update(
                normalize_payload.operation_identity.name.clone(),
                Some(normalize_payload.operation_identity.operation_type.clone()),
            );
        })?;
    }

    if method == Method::GET {
        if let Some(OperationKind::Mutation) = normalize_payload.operation_for_plan.operation_kind {
            warn!(
                target: targets::GRAPHQL_VALIDATION,
                "Mutation is not allowed over GET, stopping"
            );

            return Err(ClientPipelineError::MutationNotAllowedOverHttpGet.into());
        }
    }

    let is_subscription = matches!(
        normalize_payload.operation_for_plan.operation_kind,
        Some(OperationKind::Subscription)
    );

    if is_subscription
        && (!shared_state.router_config.subscriptions.enabled || !response_mode.can_stream())
    {
        // check early, even though we check again planned execution below
        return Err(ClientPipelineError::SubscriptionsNotSupported.into());
    }

    // subscriptions over HTTP hold a streaming response open, so they count
    // toward the long-lived client limit. the slot is reserved here - before
    // any planning or execution work - and released when the client stream
    // ends. the parsed operation kind is used instead of the `Accept` header
    // because clients like urql advertise streaming support on every
    // operation, and regular queries must not count toward the limit.
    // TODO: SharedRouterResponse::Stream should be used for long-lived connection count
    // once we add support for things like @stream and @defer
    let long_lived_client_guard = if reserve_long_lived_client && is_subscription {
        match try_reserve_long_lived_client(shared_state) {
            Ok(guard) => guard,
            Err(error) => {
                debug!(
                    target: targets::HTTP_SERVER,
                    "rejecting subscription, long-lived client limit reached"
                );
                return Ok(OperationOutcome::EarlyResponse(error.into()));
            }
        }
    } else {
        None
    };

    let usage_variables = usage_reporting::usage_report_variables(
        supergraph.runtime.hive_usage_agent.as_ref(),
        &graphql_params.variables,
    );

    let request_dedupe_enabled = shared_state
        .router_config
        .traffic_shaping
        .router
        .dedupe
        .enabled;
    let websocket_reuse_enabled = supergraph
        .snapshot
        .options
        .traffic_shaping
        .any_websocket_connection_reuse_enabled();

    // establish a connection fingerprint only if dedupe or multiplexing is enabled
    let connection_fingerprint = (request_dedupe_enabled || websocket_reuse_enabled).then(|| {
        connection_fingerprint(
            method,
            uri.path().as_str(),
            &headers,
            &shared_state.in_flight_requests_header_policy,
            supergraph.snapshot.cache_id,
        )
    });

    let fingerprint = if request_dedupe_enabled
        && matches!(
            normalize_payload.operation_for_plan.operation_kind,
            // same deduplication applies for queries and subscriptions
            None | Some(OperationKind::Query) | Some(OperationKind::Subscription)
        ) {
        let variables_hash = hash_graphql_variables(&graphql_params.variables);
        let extensions_hash = graphql_params
            .extensions
            .as_ref()
            .map_or(0, hash_graphql_extensions);
        let partition = plugin_req_state
            .as_ref()
            .and_then(|state| state.context.inbound_dedupe_partition());

        Some(inbound_request_fingerprint(
            // let chains are only allowed in Rust 2024 or later... so we assert
            // connection_fingerprint because it will be present when request_dedupe_enabled
            connection_fingerprint.expect("dedupe computes a connection fingerprint"),
            normalize_payload.normalized_operation_hash,
            variables_hash,
            extensions_hash,
            partition,
        ))
    } else {
        None
    };

    let exec = |guard| {
        execute_planned_request(
            method,
            uri,
            headers,
            path_params,
            graphql_params,
            &normalize_payload,
            supergraph,
            shared_state,
            schema_state,
            operation_span,
            plugin_req_state,
            request_context,
            response_mode,
            guard,
            response_header_sink.clone(),
            connection_fingerprint,
        )
    };

    let shared_response = if let Some(fp) = fingerprint {
        let (shared_response, role) = if is_subscription {
            shared_state
                .in_flight_requests
                .claim(fp)
                .get_or_try_init_with_guard(|guard| exec(Some(guard)))
                .await?
        } else {
            shared_state
                .in_flight_requests
                .claim(fp)
                .get_or_try_init(|| exec(None))
                .await?
        };
        if role == InFlightRole::Joiner {
            shared_state
                .telemetry_context
                .metrics
                .request_dedupe
                .record_joined(normalize_payload.operation_kind.as_str());
        }
        Arc::unwrap_or_clone(shared_response)
    } else {
        exec(None).await?
    };

    if let Some(hive_usage_agent) = &supergraph.runtime.hive_usage_agent {
        usage_reporting::collect_usage_report(
            supergraph.snapshot.supergraph_schema.clone(),
            started_at.elapsed(),
            client_name,
            client_version,
            normalize_payload.operation_for_plan.name.as_deref(),
            normalize_payload.operation_for_plan.operation_kind.as_ref(),
            &parser_payload.minified_document,
            hive_usage_agent,
            shared_response.error_count(),
            usage_request_details,
            resolved_document_id,
            usage_variables,
        )
        .await;
    }

    debug!(target: targets::GRAPHQL_EXECUTION, error_count = shared_response.error_count(), "graphql request completed");

    summary::record(|s| {
        s.set_error_count(shared_response.error_count() as u32);
    });

    Ok(OperationOutcome::Executed {
        response: shared_response,
        long_lived_client_guard,
    })
}
