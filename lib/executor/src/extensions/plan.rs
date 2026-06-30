use ahash::HashSet;

#[derive(Clone, Default)]
pub struct ExtensionsPlan {
    pub propagate: Option<ExtensionsPropagatePlan>,
}

#[derive(Clone)]
pub struct ExtensionsPropagatePlan {
    pub strategy: ExtensionsMergeStrategy,
    /// Setting this to `None` will allow ALL keys.
    pub allow: Option<HashSet<String>>,
}

#[derive(Clone, Copy, Debug)]
pub enum ExtensionsMergeStrategy {
    First,
    Last,
    Append,
}
