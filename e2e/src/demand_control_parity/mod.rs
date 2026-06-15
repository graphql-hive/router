//! Demand-control parity tests, split by scenario.
//!
//! Phase 1 (`estimated_cost.rs`) checks pure estimated-cost numbers
//! against curated reference values. Phase 2 (the remaining files)
//! exercises the full pipeline end-to-end with canned subgraph mocks
//! and asserts on the response payload (`extensions.cost`,
//! per-subgraph result codes, per-subgraph call counts).

#[cfg(test)]
mod common;

#[cfg(test)]
mod actual_cost_modes;
#[cfg(test)]
mod estimated_cost;
#[cfg(test)]
mod exceeds_max;
#[cfg(test)]
mod exceeds_max_with_subgraph_config;
#[cfg(test)]
mod list_size_inheritance;
#[cfg(test)]
mod measure_mode;
#[cfg(test)]
mod subgraph_budget;
#[cfg(test)]
mod within_max;
