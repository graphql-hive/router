use crate::query_planner::{
    ast::{
        operation::{PlanningFetchOperation, SubgraphFetchOperation},
        selection_set::SelectionSet,
    },
    utils::pretty_display::PrettyDisplay,
};
use serde::{de::DeserializeOwned, Serialize};
use std::fmt::Debug;

/// Which data a plan is allowed to hold. A plan is either still being built, or ready to execute
/// and to cache; there is no third state, so the trait is sealed.
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

/// The planner's own state: fetches still carry their parsed documents.
#[derive(Debug, Clone)]
pub struct Planning;

/// What execution reads and the cache stores: operation text, no parsed documents.
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
