//! Per-request schema selection for routers serving more than one schema.
//!
//! A plugin's `on_schema_resolve` hook overrides the default app-state schema for
//! a request by inserting a [`RequestSchema`] into its `PluginContext`, which the
//! router reads before the pipeline. The context (not the hook's return value)
//! carries it because the executor crate, where the hook lives, can't name
//! `SchemaState` / `RouterSharedState`.
//!
//! Cache note: the `normalize` / `validate` / `plan` caches are fields on
//! `SchemaState`, so a distinct `SchemaState` per schema partitions them. Don't
//! share one across schemas — the normalize and plan cache keys hash the operation,
//! not the supergraph.

use std::sync::Arc;

use crate::{schema_state::SchemaState, shared_state::RouterSharedState};

/// A schema selected for a request, inserted into the `PluginContext` by a
/// plugin's `on_schema_resolve` hook and read by the router before the pipeline.
pub struct RequestSchema {
    /// The schema/planner for this request, replacing the default app-state one.
    pub schema_state: Arc<SchemaState>,
    /// Optional per-request shared state (e.g. a per-tenant usage agent). `None`
    /// keeps the default; `Some` replaces it for the rest of the request, so it
    /// should carry the router's plugin set (or none).
    pub shared_state: Option<Arc<RouterSharedState>>,
}

impl RequestSchema {
    /// Select `schema_state`, keeping the router's default shared state.
    pub fn new(schema_state: Arc<SchemaState>) -> Self {
        Self {
            schema_state,
            shared_state: None,
        }
    }

    /// Select `schema_state` with a per-request `shared_state` override.
    pub fn with_shared_state(
        schema_state: Arc<SchemaState>,
        shared_state: Arc<RouterSharedState>,
    ) -> Self {
        Self {
            schema_state,
            shared_state: Some(shared_state),
        }
    }
}
