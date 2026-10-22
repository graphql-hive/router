use crate::query_planner::{
    ast::{
        operation::{PlanningFetchOperation, SubgraphFetchOperation},
        selection_set::SelectionSet,
    },
    utils::pretty_display::PrettyDisplay,
};
use serde::{de::DeserializeOwned, Serialize};
use std::fmt::Debug;

/// Defines what kind of operation a query plan stores
pub trait PlanState: private::Sealed + Debug + Clone {
    type Operation: Debug
        + Clone
        + Serialize
        + DeserializeOwned
        + PrettyDisplay
        + AsRef<SubgraphFetchOperation>;
    type Requires: Debug + Clone + Default;
    /// Ids of the fetches merged into a batch. Only the planner needs them.
    type FetchNodeIds: Debug + Clone + Default;
}

/// A query plan that is still being built
#[derive(Debug, Clone)]
pub struct Planning;

/// A query plan that is ready to execute or cache
#[derive(Debug, Clone)]
pub struct Executable;

impl PlanState for Planning {
    type Operation = PlanningFetchOperation;
    type Requires = Option<Box<SelectionSet>>;
    type FetchNodeIds = Vec<i64>;
}

impl PlanState for Executable {
    type Operation = SubgraphFetchOperation;
    type Requires = ();
    type FetchNodeIds = ();
}

mod private {
    pub trait Sealed {}
    impl Sealed for super::Planning {}
    impl Sealed for super::Executable {}
}
