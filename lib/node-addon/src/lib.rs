#![deny(clippy::all)]

mod query_plan;
use hive_router::executor::introspection::schema::{SchemaMetadata, SchemaWithMetadata};
use hive_router::query_planner::graph::PERCENTAGE_SCALE_FACTOR;
use hive_router::query_planner::planner::{Planner, QueryPlannerOptions};
use hive_router::query_planner::utils::cancellation::CancellationToken;
use hive_router::query_planner::utils::parsing::safe_parse_schema;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use crate::query_plan::{QueryPlanError, QueryPlanTask};

/// How the planner plans, like the router's planner options from its config. Every option is
/// off or unlimited when missing.
#[napi(object, js_name = "QueryPlannerOptions")]
pub struct JsQueryPlannerOptions {
    /// The most aliases one entity request to a subgraph may carry, like the router's
    /// `traffic_shaping.all.max_aliases_per_request`. An integer from 0 to 4294967295, where 0
    /// counts as 1. Batch fetches are split to stay within it, and it limits the
    /// `maxContextGroups` of fetches with context variables. No limit when missing.
    pub max_aliases_per_request: Option<f64>,
    /// `maxAliasesPerRequest` for single subgraphs, by subgraph name, like the router's
    /// `traffic_shaping.subgraphs.<name>.max_aliases_per_request`. Wins over
    /// `maxAliasesPerRequest` for its subgraph.
    pub subgraph_max_aliases_per_request: Option<HashMap<String, f64>>,
    /// Folds matching object type fragments into an interface fragment even when that interface
    /// isn't the field's type, like the router's
    /// `query_planner.experimental_abstract_type_folding`. Only the subgraph operations change.
    pub experimental_abstract_type_folding: Option<bool>,
}

#[napi]
pub struct QueryPlanner {
    planner: Planner,
    metadata: SchemaMetadata,
    pub consumer_schema: String,
    pub override_labels: Vec<String>,
    pub override_percentages: Vec<f64>,
    entity_keys: BTreeMap<String, BTreeMap<String, Vec<Vec<String>>>>,
    possible_types: BTreeMap<String, Vec<String>>,
}

// TODO: Did not find struct `QueryPlanner` parsed before expand #[napi] for impl?
//       fixed in vscode with `"rust-analyzer.procMacro.ignored": { "napi-derive": ["napi"] }`
#[napi]
impl QueryPlanner {
    #[napi(constructor)]
    pub fn new(supergraph_sdl: String, options: Option<JsQueryPlannerOptions>) -> Result<Self> {
        let parsed_supergraph =
            safe_parse_schema(&supergraph_sdl).map_err(QueryPlanError::SchemaParse)?;

        // capped at u32 so an emitted `maxContextGroups` above it can only mean no limit
        let alias_limit = |name: &str, value: f64| {
            if value.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(&value) {
                Ok(value as usize)
            } else {
                Err(napi::Error::from_reason(format!(
                    "{name} must be an integer from 0 to {}, got {value}",
                    u32::MAX
                )))
            }
        };
        let mut planner_options = QueryPlannerOptions::default();
        if let Some(options) = options {
            planner_options.experimental_abstract_type_folding =
                options.experimental_abstract_type_folding.unwrap_or(false);
            planner_options.max_aliases_per_request = options
                .max_aliases_per_request
                .map(|value| alias_limit("maxAliasesPerRequest", value))
                .transpose()?;
            planner_options.subgraph_max_aliases_per_request = options
                .subgraph_max_aliases_per_request
                .unwrap_or_default()
                .into_iter()
                .map(|(subgraph, value)| {
                    let limit =
                        alias_limit(&format!("subgraphMaxAliasesPerRequest.{subgraph}"), value)?;
                    Ok((subgraph, limit))
                })
                .collect::<Result<_>>()?;
        }

        let planner =
            Planner::new_from_supergraph(&parsed_supergraph, planner_options).map_err(|err| {
                napi::Error::from_reason(format!("Failed to create query planner: {}", err))
            })?;

        let consumer_schema = planner.consumer_schema.document.to_string();
        let override_labels = planner
            .supergraph
            .progressive_overrides
            .flags
            .iter()
            .map(|s| s.to_string())
            .collect();
        let override_percentages = planner
            .supergraph
            .progressive_overrides
            .percentages
            .iter()
            .map(|p| (*p as f64) / (PERCENTAGE_SCALE_FACTOR as f64))
            .collect();

        let entity_keys = planner
            .supergraph
            .known_subgraphs
            .values()
            .filter_map(|subgraph| {
                let keys = planner.supergraph.entity_keys.of_subgraph(subgraph)?;
                let keys = keys
                    .iter()
                    .map(|(type_name, keys)| (type_name.clone(), keys.clone()))
                    .collect();
                Some((subgraph.clone(), keys))
            })
            .collect();

        let metadata = planner.consumer_schema.schema_metadata();
        let possible_types = metadata
            .interface_types
            .iter()
            .chain(&metadata.union_types)
            .filter_map(|abstract_type| {
                let mut types: Vec<String> = metadata
                    .get_possible_types(abstract_type)?
                    .iter()
                    .cloned()
                    .collect();
                types.sort_unstable();
                Some((abstract_type.clone(), types))
            })
            .collect();

        Ok(QueryPlanner {
            planner,
            metadata,
            consumer_schema,
            override_labels,
            override_percentages,
            entity_keys,
            possible_types,
        })
    }

    /// Each subgraph's resolvable `@key`s, by subgraph name and then entity type. A key is its
    /// top-level fields without `__typename`, like `["id", "org"]` for
    /// `@key(fields: "id org { id }")`, and `[]` for a key of `__typename` alone. A representation
    /// goes to a subgraph only when it has every field of at least one of its type's keys there.
    /// Then a field it misses is sent as `null`. Without keys for the type, a representation that
    /// misses any field is left out.
    #[napi(getter)]
    pub fn entity_keys(&self) -> BTreeMap<String, BTreeMap<String, Vec<Vec<String>>>> {
        self.entity_keys.clone()
    }

    /// The types of each interface and union of the consumer schema that has any, sorted: the
    /// object types that implement an interface or belong to a union, and the interfaces that
    /// implement an interface. The router's executor checks type conditions with it. An object of
    /// type `T` satisfies the type condition `C` when `T` is `C` or `possibleTypes[C]` has `T`.
    /// For `requires` inline fragments it also accepts the other way around, `possibleTypes[T]`
    /// having `C`.
    #[napi(getter)]
    pub fn possible_types(&self) -> BTreeMap<String, Vec<String>> {
        self.possible_types.clone()
    }

    #[napi]
    pub fn compute_cache_key(
        &self,
        query: String,
        operation_name: Option<String>,
        active_labels: HashSet<String>,
        percentage_value: f64,
    ) -> Result<String> {
        let cache_key = query_plan::compute_cache_key(
            &self.planner,
            query.as_str(),
            operation_name.as_deref(),
            active_labels,
            percentage_value,
        )?;

        Ok(cache_key)
    }

    // queryplan located in query-plan.d.ts and will be merged with index.d.ts on build
    // because of napi-rs limitations, the queryplan from hive-query-planner cannot be used
    #[napi(ts_return_type = "QueryPlan")]
    pub fn plan<'a>(
        &self,
        query: String,
        operation_name: Option<String>,
        active_labels: HashSet<String>,
        percentage_value: f64,
        signal: Option<AbortSignal>,
        env: &'a Env,
    ) -> Result<Unknown<'a>> {
        let cancellation_token = create_cancellation_token(&signal);
        let query_plan = query_plan::query_plan(
            &self.planner,
            &self.metadata,
            query.as_str(),
            operation_name.as_deref(),
            active_labels,
            percentage_value,
            &cancellation_token,
        )?;

        env.to_js_value(&query_plan)
    }

    #[napi(ts_return_type = "Promise<QueryPlan>")]
    pub fn plan_async<'a>(
        &'a self,
        query: String,
        operation_name: Option<String>,
        active_labels: HashSet<String>,
        percentage_value: f64,
        signal: Option<AbortSignal>,
    ) -> AsyncTask<QueryPlanTask<'a>> {
        let cancellation_token = create_cancellation_token(&signal);
        AsyncTask::with_optional_signal(
            QueryPlanTask {
                planner: &self.planner,
                metadata: &self.metadata,
                query,
                operation_name,
                active_labels,
                percentage_value,
                cancellation_token,
            },
            signal,
        )
    }
}

fn create_cancellation_token(signal: &Option<AbortSignal>) -> Arc<CancellationToken> {
    let cancellation_token = Arc::new(CancellationToken::new());
    if let Some(signal) = signal {
        let cancellation_token_for_abort = Arc::clone(&cancellation_token);
        signal.on_abort(move || {
            cancellation_token_for_abort.cancel();
        });
    }
    cancellation_token
}
