use crate::query_planner::{
    ast::operation::{PlanningFetchOperation, SubgraphFetchOperation},
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
}

/// The planner's own state: fetches still carry their parsed documents.
#[derive(Debug, Clone)]
pub struct Planning;

/// What execution reads and the cache stores: operation text, no parsed documents.
#[derive(Debug, Clone)]
pub struct Executable;

impl PlanState for Planning {
    type Operation = PlanningFetchOperation;
}

impl PlanState for Executable {
    type Operation = SubgraphFetchOperation;
}

mod private {
    pub trait Sealed {}
    impl Sealed for super::Planning {}
    impl Sealed for super::Executable {}
}
