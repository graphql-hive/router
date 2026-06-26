use ahash::HashSet;
use hive_router_config::extensions::{ExtensionsConfig, ExtensionsMergeAlgo};

use super::plan::{ExtensionsMergeStrategy, ExtensionsPlan, ExtensionsPropagatePlan};

pub fn compile_extensions_plan(cfg: &ExtensionsConfig) -> ExtensionsPlan {
    let propagate = cfg.propagate.as_ref().map(|p| ExtensionsPropagatePlan {
        strategy: match p.algorithm {
            ExtensionsMergeAlgo::First => ExtensionsMergeStrategy::First,
            ExtensionsMergeAlgo::Last => ExtensionsMergeStrategy::Last,
            ExtensionsMergeAlgo::Append => ExtensionsMergeStrategy::Append,
        },
        allow: p
            .allow
            .as_ref()
            .map(|keys| keys.iter().cloned().collect::<HashSet<String>>()),
    });

    ExtensionsPlan { propagate }
}
