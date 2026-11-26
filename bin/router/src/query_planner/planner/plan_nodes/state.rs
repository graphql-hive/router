use crate::query_planner::{
    ast::{
        operation::{PlanningFetchOperation, SubgraphFetchOperation},
        selection_set::SelectionSet,
    },
    planner::plan_nodes::{FlattenNodePath, MergePaths, TriePath, TriePaths},
    utils::pretty_display::PrettyDisplay,
};
use serde::{de::DeserializeOwned, Serialize};
use smallvec::SmallVec;
use std::{
    borrow::Cow,
    fmt::{Debug, Display},
};

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
    /// A flatten node's path. An executable plan keeps only the trie of it.
    type FlattenPath: Debug + Clone + Serialize + DeserializeOwned + Display;
    /// A batch alias's merge paths. An executable plan keeps them in the batch's trie.
    type MergePaths: Debug + Clone + Serialize + DeserializeOwned + PathList;
}

/// Merge paths, however a plan keeps them
pub trait PathList {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The paths, told again from the trie when that's all the plan keeps
    fn to_paths(&self) -> Cow<'_, [FlattenNodePath]>;
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
    type FlattenPath = FlattenNodePath;
    type MergePaths = MergePaths;
}

impl PlanState for Executable {
    type Operation = SubgraphFetchOperation;
    type Requires = ();
    type FetchIds = ();
    type FlattenPath = TriePath;
    type MergePaths = TriePaths;
}

mod private {
    pub trait Sealed {}
    impl Sealed for super::Planning {}
    impl Sealed for super::Executable {}
}
