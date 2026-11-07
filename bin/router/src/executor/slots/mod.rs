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
pub mod write;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, OnceLock};

use crate::executor::introspection::schema::SchemaMetadata;
use crate::executor::projection::plan::ProjectionPlan;
use crate::query_planner::planner::plan_nodes::QueryPlan;
use compile::SlotPlan;

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

/// The slot plan of a cached query plan, compiled by the first request that needs it.
#[derive(Default)]
pub struct SlotPlanCell(OnceLock<Option<SlotPlan>>);

impl SlotPlanCell {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The slot plan, when the query plan is supported and was compiled with this projection
    /// plan.
    pub fn get(
        &self,
        plan: &QueryPlan,
        projection: &Arc<ProjectionPlan>,
        schema: &SchemaMetadata,
        root_type: &str,
    ) -> Option<&SlotPlan> {
        self.0
            .get_or_init(
                || match SlotPlan::compile(plan, projection, schema, root_type) {
                    Ok(compiled) => Some(compiled),
                    Err(reason) => {
                        tracing::debug!(reason = reason.0, "the slot pipeline can't run this plan");
                        None
                    }
                },
            )
            .as_ref()
            .filter(|compiled| Arc::ptr_eq(&compiled.projection, projection))
    }
}

#[cfg(test)]
mod tests;
