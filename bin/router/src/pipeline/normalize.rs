use crate::query_planner::ast::flat::plan_document::PlanDocument;
use crate::query_planner::ast::flat::selection_tree::PlanOperation;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use crate::executor::hooks::on_graphql_params::GraphQLParams;
use crate::executor::hooks::on_supergraph_load::SupergraphSnapshot;
use crate::executor::projection::plan::ProjectionPlan;
use crate::query_planner::ast::normalization::arguments_to_variables::{
    arguments_to_variables, ExtractedVariable,
};
use crate::query_planner::ast::normalization::error::NormalizationError;
use crate::query_planner::ast::normalization::normalize_flat_operation;
use crate::query_planner::ast::operation::OperationDefinition;
use crate::query_planner::state::supergraph_state::OperationKind;
use crate::telemetry::logging::targets;
use crate::telemetry::traces::spans::graphql::{
    GraphQLNormalizeSpan, GraphQLSpanOperationIdentity,
};
use xxhash_rust::xxh3::Xxh3;

use crate::cache_state::{CacheHitMiss, EntryResultHitMissExt};
use crate::executor::operation_filter::OperationFilterOutput;
use crate::executor::response::graphql_error::GraphQLError;
use crate::pipeline::error::PipelineError;
use crate::pipeline::nullify::rebuilder::rebuild_nulled_operation;
use crate::pipeline::parser::GraphQLParserPayload;
use crate::pipeline::trie::Trie;
use crate::schema_state::{RouterSupergraphRuntime, SchemaState};
use tracing::{debug, Instrument};

#[derive(Debug, Clone)]
pub struct GraphQLNormalizationPayload {
    /// `document_for_plan`, what the planner plans.
    pub plan_operation: Arc<PlanOperation>,
    /// The operation to execute, without introspection fields.
    pub document_for_plan: Arc<PlanDocument>,
    /// The introspection part, the router answers it itself.
    pub document_for_introspection: Option<Arc<PlanDocument>>,
    pub operation_for_plan_hash: u128,
    pub operation_for_introspection_hash: Option<u128>,
    /// `document_for_plan` as a tree, built the first time a plugin hook asks for it.
    operation_tree: OnceLock<Arc<OperationDefinition>>,
    pub normalized_operation_hash: u64,
    pub root_type_name: String,
    pub operation_kind: OperationKind,
    pub projection_plan: Arc<ProjectionPlan>,
    pub operation_identity: OperationIdentity,
    /// Values of the variables that hard-coded arguments were turned into.
    /// See `query_planner.arguments_to_variables` in the config.
    pub extracted_variables: Arc<Vec<ExtractedVariable>>,
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
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        plan_operation: Arc<PlanOperation>,
        document_for_plan: Arc<PlanDocument>,
        document_for_introspection: Option<Arc<PlanDocument>>,
        hashes: NormalizedOperationHashes,
        root_type_name: String,
        operation_kind: OperationKind,
        projection_plan: Arc<ProjectionPlan>,
        operation_identity: OperationIdentity,
        extracted_variables: Arc<Vec<ExtractedVariable>>,
    ) -> Self {
        GraphQLNormalizationPayload {
            plan_operation,
            document_for_plan,
            document_for_introspection,
            operation_for_plan_hash: hashes.operation_for_plan_hash,
            operation_for_introspection_hash: hashes.operation_for_introspection_hash,
            normalized_operation_hash: hashes.combined_operation_hash,
            root_type_name,
            operation_kind,
            projection_plan,
            operation_identity,
            operation_tree: OnceLock::new(),
            extracted_variables,
        }
    }

    /// `document_for_plan` as a tree, for the plugin hooks only. Built once, on the first ask,
    /// and kept with the cached payload. Same reference every time, so the pipeline can tell a
    /// plugin's own operation apart from it.
    pub fn operation_for_plan_tree(&self) -> &Arc<OperationDefinition> {
        self.operation_tree
            .get_or_init(|| Arc::new(self.document_for_plan.operation_tree()))
    }

    pub(crate) fn with_operation(
        &self,
        new_document: PlanDocument,
        new_projection_plan: Arc<ProjectionPlan>,
    ) -> Arc<GraphQLNormalizationPayload> {
        let hashes = hash_normalized_operation(
            &new_document,
            self.document_for_introspection.as_deref(),
            &self.extracted_variables,
        );
        Arc::new(GraphQLNormalizationPayload::new(
            Arc::new(PlanOperation::from_document(&new_document)),
            Arc::new(new_document),
            self.document_for_introspection.clone(),
            hashes,
            self.root_type_name.clone(),
            self.operation_kind.clone(),
            new_projection_plan,
            self.operation_identity.clone(),
            self.extracted_variables.clone(),
        ))
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
        let new_document = rebuild_nulled_operation(&payload.document_for_plan, &trie);
        let new_projection = payload.projection_plan.rewrite(&trie);
        (
            payload.with_operation(new_document, new_projection),
            self.errors,
        )
    }
}

pub fn hash_normalized_operation(
    document_for_plan: &PlanDocument,
    document_for_introspection: Option<&PlanDocument>,
    extracted_variables: &[ExtractedVariable],
) -> NormalizedOperationHashes {
    let operation_for_plan_hash = document_for_plan.hash();
    let operation_for_introspection_hash = document_for_introspection.map(PlanDocument::hash);

    let mut hasher = Xxh3::new();
    operation_for_plan_hash.hash(&mut hasher);
    operation_for_introspection_hash.is_some().hash(&mut hasher);
    if let Some(hash) = operation_for_introspection_hash {
        hash.hash(&mut hasher);
    }
    // `{ user(id: "1") }` and `{ user(id: "2") }` share an operation once the ids are extracted,
    // so the values have to be in here too, or request dedupe would treat them as one request.
    extracted_variables.hash(&mut hasher);

    NormalizedOperationHashes {
        operation_for_plan_hash,
        operation_for_introspection_hash,
        combined_operation_hash: hasher.finish(),
    }
}

pub struct NormalizedOperationHashes {
    pub operation_for_plan_hash: u128,
    pub operation_for_introspection_hash: Option<u128>,
    pub combined_operation_hash: u64,
}

#[inline]
pub async fn normalize_request_with_cache(
    supergraph: &SupergraphSnapshot,
    runtime: &RouterSupergraphRuntime,
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

        runtime
            .normalize_cache
            .entry(cache_key)
            .or_try_insert_with::<_, NormalizationError>(async {
                let doc = normalize_flat_operation(
                    &supergraph.planner.supergraph,
                    &parser_payload.parsed_operation.flat,
                    graphql_params.operation_name.as_deref(),
                )?;

                debug!(
                    target: targets::GRAPHQL_NORMALIZATION,
                    operation_name = ?doc.operation_name,
                    result = %doc.document,
                    "successfully normalized GraphQL operation"
                );

                let operation_kind = doc.document.kind.clone().unwrap_or(OperationKind::Query);
                let (root_type_name, projection_plan) =
                    ProjectionPlan::from_operation(&doc.document, &supergraph.metadata);
                let root_type_name = root_type_name.to_string();
                let projection_plan = Arc::new(projection_plan);
                let (mut document_for_plan, document_for_introspection, mut plan_operation) =
                    match doc.document.partition_introspection() {
                        Some((downstream, introspection)) => {
                            let plan_operation = PlanOperation::from_document(&downstream);
                            (
                                Arc::new(downstream),
                                Some(Arc::new(introspection)),
                                Arc::new(plan_operation),
                            )
                        }
                        // moved out, so the document isn't shared below
                        None => (doc.document, None, doc.plan_operation),
                    };
                let extracted_variables = if supergraph.options.query_planner.arguments_to_variables
                {
                    // clones only if something else still holds the document
                    let document = Arc::make_mut(&mut document_for_plan);
                    let extracted =
                        arguments_to_variables(document, &supergraph.planner.supergraph);
                    if !extracted.is_empty() {
                        plan_operation = Arc::new(PlanOperation::from_document(document));
                    }
                    extracted
                } else {
                    Vec::new()
                };

                let hashes = hash_normalized_operation(
                    &document_for_plan,
                    document_for_introspection.as_deref(),
                    &extracted_variables,
                );

                let payload = GraphQLNormalizationPayload::new(
                    plan_operation,
                    document_for_plan,
                    document_for_introspection,
                    hashes,
                    root_type_name,
                    operation_kind,
                    projection_plan,
                    OperationIdentity {
                        name: doc.operation_name.clone(),
                        operation_type: parser_payload.operation_type.clone(),
                        client_document_hash: parser_payload.cache_key_string.clone(),
                    },
                    Arc::new(extracted_variables),
                );

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
