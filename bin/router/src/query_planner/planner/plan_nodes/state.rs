use crate::query_planner::{
    ast::{
        operation::{PlanningFetchOperation, SubgraphFetchOperation},
        selection_set::SelectionSet,
    },
    utils::pretty_display::PrettyDisplay,
};
use serde::{de::DeserializeOwned, Serialize};
use smallvec::SmallVec;
use std::fmt::Debug;

use super::FetchId;

/// Defines what kind of operation a query plan stores
pub trait PlanState: private::Sealed + Debug + Clone {
    type Operation: Debug
        + Clone
        + Serialize
        + DeserializeOwned
        + PrettyDisplay
        + AsRef<SubgraphFetchOperation>;
    type Requires: Debug + Clone + Default;
    /// Fetch ids a fetch depends on, or that a batch completes. Only the planner needs them: it
    /// turns them into the plan's schedule, executable nodes don't keep them.
    type FetchIds: Debug + Clone + Default;
}

/// Most fetches depend on one or two others, so they fit inline.
pub type FetchIdList = SmallVec<[FetchId; 2]>;

/// A query plan that is still being built
#[derive(Debug, Clone)]
pub struct Planning;

/// A query plan that is ready to execute or cache
#[derive(Debug, Clone)]
pub struct Executable;

impl PlanState for Planning {
    type Operation = PlanningFetchOperation;
    type Requires = Option<Box<SelectionSet>>;
    type FetchIds = FetchIdList;
}

impl PlanState for Executable {
    type Operation = SubgraphFetchOperation;
    type Requires = ();
    type FetchIds = ();
}

mod private {
    pub trait Sealed {}
    impl Sealed for super::Planning {}
    impl Sealed for super::Executable {}
}
