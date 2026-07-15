use std::hash::{Hash, Hasher};
use std::sync::Arc;

use hive_router_internal::telemetry::traces::spans::graphql::{
    GraphQLNormalizeSpan, GraphQLSpanOperationIdentity,
};
use hive_router_plan_executor::hooks::on_graphql_params::GraphQLParams;
use hive_router_plan_executor::hooks::on_supergraph_load::SupergraphData;
use hive_router_plan_executor::introspection::partition::partition_operation;
use hive_router_plan_executor::projection::plan::FieldProjectionPlan;
use hive_router_query_planner::ast::normalization::error::NormalizationError;
use hive_router_query_planner::ast::normalization::normalize_operation;
use hive_router_query_planner::ast::operation::OperationDefinition;
use hive_router_query_planner::state::supergraph_state::OperationKind;
use xxhash_rust::xxh3::Xxh3;

use crate::cache_state::{CacheHitMiss, EntryResultHitMissExt};
use crate::pipeline::error::PipelineError;
use crate::pipeline::nullify::rebuilder::{
    rebuild_nulled_operation, rebuild_nulled_projection_plan,
};
use crate::pipeline::parser::GraphQLParserPayload;
use crate::pipeline::trie::Trie;
use crate::schema_state::SchemaState;
use hive_router_plan_executor::operation_filter::OperationFilterOutput;
use hive_router_plan_executor::response::graphql_error::GraphQLError;
use tracing::{trace, Instrument};

#[derive(Debug, Clone)]
pub struct GraphQLNormalizationPayload {
    /// The operation to execute, without introspection fields.
    pub operation_for_plan: Arc<OperationDefinition>,
    pub operation_for_plan_hash: u64,
    pub operation_for_introspection: Option<Arc<OperationDefinition>>,
    pub operation_for_introspection_hash: Option<u64>,
    pub normalized_operation_hash: u64,
    pub root_type_name: String,
    pub operation_kind: OperationKind,
    pub projection_plan: Arc<Vec<FieldProjectionPlan>>,
    pub operation_identity: OperationIdentity,
}

#[derive(Debug, Clone)]
pub struct OperationIdentity {
    pub name: Option<String>,
    pub operation_type: OperationKind,
    /// Hash of the original document sent to the router, by the client.
    pub client_document_hash: String,
}

impl<'a> From<&'a OperationIdentity> for GraphQLSpanOperationIdentity<'a> {
    fn from(op_id: &'a OperationIdentity) -> Self {
        GraphQLSpanOperationIdentity {
            name: op_id.name.as_deref(),
            operation_type: op_id.operation_type.as_str(),
            client_document_hash: &op_id.client_document_hash,
        }
    }
}

impl GraphQLNormalizationPayload {
    pub(crate) fn with_operation(
        &self,
        new_operation: OperationDefinition,
        new_projection_plan: Vec<FieldProjectionPlan>,
    ) -> Arc<GraphQLNormalizationPayload> {
        let hashes =
            hash_normalized_operation(&new_operation, self.operation_for_introspection.as_deref());

        Arc::new(GraphQLNormalizationPayload {
            operation_for_plan: Arc::new(new_operation),
            operation_for_plan_hash: hashes.operation_for_plan_hash,
            // These are cheap Arc clones
            operation_for_introspection: self.operation_for_introspection.clone(),
            operation_for_introspection_hash: hashes.operation_for_introspection_hash,
            normalized_operation_hash: hashes.combined_operation_hash,
            root_type_name: self.root_type_name.clone(),
            operation_kind: self.operation_kind.clone(),
            projection_plan: Arc::new(new_projection_plan),
            operation_identity: self.operation_identity.clone(),
        })
    }
}

/// OperationFilterOutput is defined in the executor crate,
/// so we extend it, to have a nice API to produce a new GraphQLNormalizationPayload,
/// based on the filter output and the original payload.
pub(crate) trait FilterOutputExt<'exec> {
    fn apply_to(
        self,
        payload: &GraphQLNormalizationPayload,
    ) -> (Arc<GraphQLNormalizationPayload>, Vec<GraphQLError>);
}

impl<'exec> FilterOutputExt<'exec> for OperationFilterOutput<'exec> {
    fn apply_to(
        self,
        payload: &GraphQLNormalizationPayload,
    ) -> (Arc<GraphQLNormalizationPayload>, Vec<GraphQLError>) {
        let trie = Trie::from_paths(&self.rejected_paths);
        let new_op = rebuild_nulled_operation(&payload.operation_for_plan, &trie);
        let new_projection = rebuild_nulled_projection_plan(&payload.projection_plan, &trie);
        (payload.with_operation(new_op, new_projection), self.errors)
    }
}

pub fn hash_normalized_operation(
    operation_for_plan: &OperationDefinition,
    operation_for_introspection: Option<&OperationDefinition>,
) -> NormalizedOperationHashes {
    let operation_for_plan_hash = operation_for_plan.hash();
    let operation_for_introspection_hash =
        operation_for_introspection.map(OperationDefinition::hash);

    let mut hasher = Xxh3::new();
    operation_for_plan_hash.hash(&mut hasher);
    operation_for_introspection_hash.is_some().hash(&mut hasher);
    if let Some(hash) = operation_for_introspection_hash {
        hash.hash(&mut hasher);
    }

    NormalizedOperationHashes {
        operation_for_plan_hash,
        operation_for_introspection_hash,
        combined_operation_hash: hasher.finish(),
    }
}

pub struct NormalizedOperationHashes {
    pub operation_for_plan_hash: u64,
    pub operation_for_introspection_hash: Option<u64>,
    pub combined_operation_hash: u64,
}

#[inline]
pub async fn normalize_request_with_cache(
    supergraph: &SupergraphData,
    schema_state: &SchemaState,
    graphql_params: &GraphQLParams,
    parser_payload: &GraphQLParserPayload,
) -> Result<Arc<GraphQLNormalizationPayload>, PipelineError> {
    let metrics = &schema_state.telemetry_context.metrics;
    let normalize_cache_capture = metrics.cache.normalize.capture_request();
    let normalize_span = GraphQLNormalizeSpan::new();
    async {
        let cache_key = match &graphql_params.operation_name {
            Some(operation_name) => {
                let mut hasher = Xxh3::new();
                graphql_params.query.hash(&mut hasher);
                operation_name.hash(&mut hasher);
                hasher.finish()
            }
            None => parser_payload.cache_key,
        };

        schema_state
            .normalize_cache
            .entry(cache_key)
            .or_try_insert_with::<_, NormalizationError>(async {
                let doc = normalize_operation(
                    &supergraph.planner.supergraph,
                    &parser_payload.parsed_operation,
                    graphql_params.operation_name.as_deref(),
                )?;

                trace!(
                    "Successfully normalized GraphQL operation (operation name={:?}): {}",
                    doc.operation_name,
                    doc.operation
                );

                let operation = doc.operation;
                let operation_kind = operation
                    .operation_kind
                    .clone()
                    .unwrap_or(OperationKind::Query);
                let (root_type_name, projection_plan) =
                    FieldProjectionPlan::from_operation(&operation, &supergraph.metadata);
                let root_type_name = root_type_name.to_string();
                let partitioned_operation = partition_operation(operation);

                let operation_for_plan = Arc::new(partitioned_operation.downstream_operation);
                let operation_for_introspection =
                    partitioned_operation.introspection_operation.map(Arc::new);

                let hashes = hash_normalized_operation(
                    &operation_for_plan,
                    operation_for_introspection.as_deref(),
                );

                let payload = GraphQLNormalizationPayload {
                    root_type_name,
                    operation_kind,
                    projection_plan: Arc::new(projection_plan),
                    operation_for_plan,
                    operation_for_plan_hash: hashes.operation_for_plan_hash,
                    operation_for_introspection,
                    operation_for_introspection_hash: hashes.operation_for_introspection_hash,
                    normalized_operation_hash: hashes.combined_operation_hash,
                    operation_identity: OperationIdentity {
                        name: doc.operation_name.clone(),
                        operation_type: parser_payload.operation_type.clone(),
                        client_document_hash: parser_payload.cache_key_string.clone(),
                    },
                };

                Ok(Arc::new(payload))
            })
            .await
            .map_err(PipelineError::from)
            .into_result_with_hit_miss(|hit_miss| match hit_miss {
                CacheHitMiss::Hit => {
                    normalize_span.record_cache_hit(true);
                    normalize_cache_capture.finish_hit();
                }
                CacheHitMiss::Miss | CacheHitMiss::Error => {
                    normalize_span.record_cache_hit(false);
                    normalize_cache_capture.finish_miss();
                }
            })
    }
    .instrument(normalize_span.clone())
    .await
}
