use crate::query_planner::{
    ast::operation::{PlanningFetchOperation, SubgraphFetchOperation},
    utils::pretty_display::PrettyDisplay,
};
use serde::{de::DeserializeOwned, Serialize};
use std::fmt::Debug;

/// Which data a plan is allowed to hold. A plan is either still being built, or ready to be
/// executed and cached. There is no third state, so this trait is sealed.
pub trait PlanState: private::Sealed + Debug + Clone {
    type Operation: Debug
        + Clone
        + Serialize
        + DeserializeOwned
        + PrettyDisplay
        + AsRef<SubgraphFetchOperation>;
}

/// The state the planner works in. Fetches still carry their parsed documents.
#[derive(Debug, Clone)]
pub struct Planning;

/// The state execution reads and the cache stores. Fetches carry only operation text, with no
/// parsed documents.
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
