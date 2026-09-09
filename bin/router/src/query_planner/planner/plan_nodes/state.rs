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
    /// The parsed entity requirements, which only the planner needs.
    type Requires: Debug + Clone + Default;
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
}

impl PlanState for Executable {
    type Operation = SubgraphFetchOperation;
    type Requires = ();
}

mod private {
    pub trait Sealed {}
    impl Sealed for super::Planning {}
    impl Sealed for super::Executable {}
}
