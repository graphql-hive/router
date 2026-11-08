//! The slot pipeline: subgraph bodies to client bytes without building `Value`s.
//!
//! A query plan is compiled once into a [`compile::SlotPlan`]: a layout for each position in the
//! response and, for each fetch, the shape its response is read with. A request then parses
//! every subgraph body into slots of a pooled [`store::Arena`], writes representations from those
//! slots, merges entities by copying slots, and writes the client response from them.
//! See `pipeline-plan.md` and `plan-log.md` at the repository root.

pub mod compile;
pub mod parse;
pub mod store;
pub mod walk;
pub mod write;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, OnceLock, RwLock};

use crate::executor::introspection::schema::SchemaMetadata;
use crate::executor::projection::plan::ProjectionPlan;
use crate::query_planner::ast::operation::OperationDefinition;
use crate::query_planner::planner::plan_nodes::{FetchNode, QueryPlan};
use compile::{Seed, SlotPlan};

/// Whether requests may use the slot pipeline, from the router config. Off by default.
static ENABLED: AtomicBool = AtomicBool::new(false);

/// `HIVE_SLOT_PIPELINE=1` (or `true`) turns it on whatever the config says.
static FROM_ENV: LazyLock<bool> = LazyLock::new(|| {
    std::env::var("HIVE_SLOT_PIPELINE").is_ok_and(|value| value == "1" || value == "true")
});

/// Turns the slot pipeline on or off for the process (`query_planner.experimental_slot_pipeline`).
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed) || *FROM_ENV
}

/// Whether entity fetches may copy entities that are already in the response
/// (`traffic_shaping.experimental_entity_reuse`). Off by default. Slot plans compiled while it's
/// off have no reuse programs, so it costs nothing then.
static ENTITY_REUSE: AtomicBool = AtomicBool::new(false);

pub fn set_entity_reuse(enabled: bool) {
    ENTITY_REUSE.store(enabled, Ordering::Relaxed);
}

pub fn entity_reuse() -> bool {
    ENTITY_REUSE.load(Ordering::Relaxed)
}

/// The slot plans of a cached query plan, compiled by the first request that needs each.
///
/// A slot plan is compiled for one projection plan (and the introspection part of the
/// operation that goes with it). Most requests of a query plan have the
/// same one; an authorization rewrite (fields the client may not see, forced to null) gives a
/// request its own copy, and different client operations can share a query plan. Those get a
/// slot plan of their own, found by the projection plan's content.
#[derive(Default)]
pub struct SlotPlanCell {
    first: OnceLock<Option<Arc<SlotPlan>>>,
    others: RwLock<Vec<Arc<SlotPlan>>>,
}

/// Slot plans kept for projection plans other than the first one. Past this, a request with
/// yet another projection plan compiles one for itself.
const MAX_OTHERS: usize = 16;

impl SlotPlanCell {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The cell of a subscription's stream: the plan that runs for each event (`plan`, the
    /// nodes after the subscription's own fetch) with each event read in like a fetch's
    /// response, compiled now for the whole stream.
    pub fn for_subscription(
        plan: &QueryPlan,
        projection: &Arc<ProjectionPlan>,
        schema: &SchemaMetadata,
        root_type: &str,
        subscription: &FetchNode,
    ) -> Arc<Self> {
        let cell = Self::default();
        let compiled = compile_or_log(
            plan,
            projection,
            schema,
            root_type,
            Seed::Subscription(subscription),
        );
        let _ = cell.first.set(compiled);
        Arc::new(cell)
    }

    /// Whether the first slot plan compiled: the slot pipeline runs the requests that use it
    pub fn compiled(&self) -> bool {
        self.first.get().is_some_and(Option::is_some)
    }

    /// The slot plan for this projection plan, `None` when the query plan isn't supported.
    pub fn get(
        &self,
        plan: &QueryPlan,
        projection: &Arc<ProjectionPlan>,
        schema: &SchemaMetadata,
        root_type: &str,
        introspection: Option<&OperationDefinition>,
    ) -> Option<Arc<SlotPlan>> {
        let seed = introspection.map_or(Seed::None, Seed::Operation);
        let compile = || compile_or_log(plan, projection, schema, root_type, seed);
        let first = self.first.get_or_init(compile).as_ref()?;
        if first.projection.same_as(projection) {
            return Some(Arc::clone(first));
        }
        let read = self
            .others
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(found) = read
            .iter()
            .find(|other| other.projection.same_as(projection))
        {
            return Some(Arc::clone(found));
        }
        drop(read);
        let compiled = compile()?;
        let mut others = self
            .others
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if others.len() < MAX_OTHERS
            && !others
                .iter()
                .any(|other| other.projection.same_as(projection))
        {
            others.push(Arc::clone(&compiled));
        }
        Some(compiled)
    }
}

fn compile_or_log(
    plan: &QueryPlan,
    projection: &Arc<ProjectionPlan>,
    schema: &SchemaMetadata,
    root_type: &str,
    seed: Seed,
) -> Option<Arc<SlotPlan>> {
    match SlotPlan::compile(plan, projection, schema, root_type, seed) {
        Ok(compiled) => Some(Arc::new(compiled)),
        Err(reason) => {
            tracing::debug!(reason = reason.0, "the slot pipeline can't run this plan");
            None
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
