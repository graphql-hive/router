use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use graphql_tools::static_graphql::schema::Document;
use hive_router_query_planner::planner::{Planner, PlannerError, QueryPlannerOptions};
use hive_router_query_planner::utils::parsing::safe_parse_schema;
use tokio_util::sync::CancellationToken;

use crate::{
    introspection::schema::{SchemaMetadata, SchemaWithMetadata},
    plugin_trait::{EndHookPayload, FromGraphQLErrorToResponse, StartHookPayload},
    response::graphql_error::GraphQLError,
};

pub struct PublicSchema {
    /// The AST of the public schema document exposed by the router.
    pub document: Arc<Document>,
    /// The SDL string of the public schema document exposed by the router.
    pub sdl: Arc<str>,
}

/// Errors that can occur while constructing a schema-only [`Supergraph`].
#[derive(Debug, thiserror::Error)]
pub enum SupergraphBuildError {
    #[error("Failed to parse supergraph SDL: {0}")]
    ParseError(#[from] graphql_tools::parser::schema::ParseError),
    #[error("Failed to build query planner: {0}")]
    PlannerBuilderError(#[from] PlannerError),
}

/// Monotonically increasing id allocated to each constructed supergraph.
static NEXT_SUPERGRAPH_DATA_ID: AtomicU64 = AtomicU64::new(0);

/// The schema-derived data shared by a [`Supergraph`] owner and every [`SupergraphSnapshot`]
/// cloned from it. Never constructed directly; always reached through the owner or a snapshot.
pub struct SupergraphData {
    /// Process-unique id allocated once per constructed supergraph. Never reused, so a later
    /// instance cannot reuse an earlier runtime or join its in-flight request deduplication,
    /// even when both have identical consumer schemas.
    pub cache_id: u64,
    pub metadata: Arc<SchemaMetadata>,
    pub planner: Planner,
    pub supergraph_schema: Arc<Document>,
    pub public_schema: PublicSchema,
}

/// A cheap, read-only snapshot of a [`Supergraph`] owner: the schema-derived data plus a clone
/// of the owner's retirement token. Holding a snapshot keeps the schema-derived data alive, but
/// never keeps the owner `Arc<Supergraph>` itself alive.
#[derive(Clone)]
pub struct SupergraphSnapshot {
    data: Arc<SupergraphData>,
    retirement: CancellationToken,
}

impl Deref for SupergraphSnapshot {
    type Target = SupergraphData;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl SupergraphSnapshot {
    /// Whether the owner this snapshot was taken from has since been retired (dropped or
    /// replaced). Ordinary in-flight requests holding a snapshot are unaffected by retirement;
    /// this is meant for subscription producers and long-lived connections deciding whether to
    /// keep accepting new work from this snapshot.
    #[inline]
    pub fn is_retired(&self) -> bool {
        self.retirement.is_cancelled()
    }

    /// Resolves once the owner this snapshot was taken from is retired. Resolves immediately if
    /// it already was.
    #[inline]
    pub async fn retired(&self) {
        self.retirement.cancelled().await
    }

    /// A clone of the retirement token, for callers (e.g. subscription pumps) that need to hold
    /// onto it independently of the rest of the snapshot.
    #[inline]
    pub fn retirement_token(&self) -> CancellationToken {
        self.retirement.clone()
    }
}

/// The owner handle for one constructed supergraph. Plugins and the router's configured source
/// retain `Arc<Supergraph>` while a supergraph remains selectable for new requests. It contains
/// only schema-derived data. Router runtime concerns such as subgraph executors and telemetry live
/// in the router and are built separately from a [`SupergraphSnapshot`].
///
/// When the last `Arc<Supergraph>` reference drops, [`Drop`] publishes retirement: every
/// [`SupergraphSnapshot`] taken from it observes this (immediately, or later via
/// [`SupergraphSnapshot::retired`]), without that observation delaying or being delayed by the
/// owner's drop.
pub struct Supergraph {
    data: Arc<SupergraphData>,
    retirement: CancellationToken,
}

impl Deref for Supergraph {
    type Target = SupergraphData;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl Drop for Supergraph {
    fn drop(&mut self) {
        self.retirement.cancel();
    }
}

impl From<&Supergraph> for SupergraphSnapshot {
    fn from(val: &Supergraph) -> Self {
        SupergraphSnapshot {
            data: val.data.clone(),
            retirement: val.retirement.clone(),
        }
    }
}

impl Supergraph {
    /// Same as [`Self::from_sdl`], but takes an already-parsed supergraph document.
    pub fn from_document(
        document: Document,
        options: QueryPlannerOptions,
    ) -> Result<Self, SupergraphBuildError> {
        let planner = Planner::new_from_supergraph(&document, options)?;
        let metadata = Arc::new(planner.consumer_schema.schema_metadata());

        let public_schema = PublicSchema {
            document: planner.consumer_schema.document.clone(),
            sdl: Arc::<str>::from(planner.consumer_schema.document.to_string()),
        };

        let cache_id = NEXT_SUPERGRAPH_DATA_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("supergraph id space exhausted");
        let data = SupergraphData {
            cache_id,
            metadata,
            planner,
            supergraph_schema: Arc::new(document),
            public_schema,
        };

        Ok(Self {
            data: Arc::new(data),
            retirement: CancellationToken::new(),
        })
    }

    /// Parses `sdl` into a supergraph document and builds a schema-only [`Supergraph`] from
    /// it. `options` may only carry query-planner options - router configuration, telemetry, and
    /// router runtime state do not belong here.
    pub fn from_sdl(sdl: &str, options: QueryPlannerOptions) -> Result<Self, SupergraphBuildError> {
        Self::from_document(safe_parse_schema(sdl)?, options)
    }

    /// Takes a cheap, read-only snapshot of this owner's schema-derived data plus a clone of its
    /// retirement token. Store this in request extensions and caches - never the owner itself.
    pub fn snapshot(&self) -> SupergraphSnapshot {
        self.into()
    }
}

pub type OnSupergraphLoadResult = Result<Supergraph, GraphQLError>;

pub struct OnSupergraphLoadStartHookPayload {
    /// A snapshot of the configured supergraph currently in use by the router, before loading
    /// the new one. `None` before the first supergraph has finished loading.
    pub current_supergraph_data: Option<SupergraphSnapshot>,
    /// The parsed AST of the new supergraph schema being loaded by the router.
    /// Plugins can modify this document before planner and runtime construction. The modified
    /// document is used for the rest of the configured load process.
    pub new_ast: Document,
}

impl StartHookPayload<OnSupergraphLoadEndHookPayload, OnSupergraphLoadResult>
    for OnSupergraphLoadStartHookPayload
{
}

pub type OnSupergraphLoadStartHookResult<'exec> = crate::plugin_trait::StartHookResult<
    'exec,
    OnSupergraphLoadStartHookPayload,
    OnSupergraphLoadEndHookPayload,
    OnSupergraphLoadResult,
>;

pub struct OnSupergraphLoadEndHookPayload {
    /// The new supergraph data that is generated from loading the new supergraph schema.
    pub new_supergraph_data: Supergraph,
}

impl EndHookPayload<OnSupergraphLoadResult> for OnSupergraphLoadEndHookPayload {}

pub type OnSupergraphLoadEndHookResult =
    crate::plugin_trait::EndHookResult<OnSupergraphLoadEndHookPayload, OnSupergraphLoadResult>;

impl FromGraphQLErrorToResponse for OnSupergraphLoadResult {
    fn from_graphql_error_to_response(error: GraphQLError, _status_code: http::StatusCode) -> Self {
        Err(error)
    }
}
