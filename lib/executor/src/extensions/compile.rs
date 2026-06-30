use ahash::HashSet;
use hive_router_config::response_extensions::{ExtensionsConfig, ExtensionsMergeAlgo};

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

#[cfg(test)]
mod tests {
    use hive_router_config::response_extensions::{
        ExtensionsConfig, ExtensionsMergeAlgo, ExtensionsPropagateConfig,
    };

    use super::*;
    use crate::extensions::plan::ExtensionsMergeStrategy;

    #[test]
    fn test_compile_last() {
        let cfg = ExtensionsConfig {
            propagate: Some(ExtensionsPropagateConfig {
                algorithm: ExtensionsMergeAlgo::Last,
                allow: None,
            }),
        };
        let plan = compile_extensions_plan(&cfg);
        let p = plan.propagate.unwrap();
        assert!(matches!(p.strategy, ExtensionsMergeStrategy::Last));
        assert!(p.allow.is_none());
    }

    #[test]
    fn test_compile_first() {
        let cfg = ExtensionsConfig {
            propagate: Some(ExtensionsPropagateConfig {
                algorithm: ExtensionsMergeAlgo::First,
                allow: None,
            }),
        };
        let plan = compile_extensions_plan(&cfg);
        assert!(matches!(
            plan.propagate.unwrap().strategy,
            ExtensionsMergeStrategy::First
        ));
    }

    #[test]
    fn test_compile_append() {
        let cfg = ExtensionsConfig {
            propagate: Some(ExtensionsPropagateConfig {
                algorithm: ExtensionsMergeAlgo::Append,
                allow: None,
            }),
        };
        let plan = compile_extensions_plan(&cfg);
        assert!(matches!(
            plan.propagate.unwrap().strategy,
            ExtensionsMergeStrategy::Append
        ));
    }

    #[test]
    fn test_compile_allow_list() {
        let cfg = ExtensionsConfig {
            propagate: Some(ExtensionsPropagateConfig {
                algorithm: ExtensionsMergeAlgo::Last,
                allow: Some(vec!["foo".to_string(), "bar".to_string()]),
            }),
        };
        let plan = compile_extensions_plan(&cfg);
        let allow = plan.propagate.unwrap().allow.unwrap();
        assert!(allow.contains("foo"));
        assert!(allow.contains("bar"));
        assert!(!allow.contains("baz"));
        assert_eq!(allow.len(), 2);
    }

    #[test]
    fn test_compile_no_propagate() {
        let cfg = ExtensionsConfig { propagate: None };
        let plan = compile_extensions_plan(&cfg);
        assert!(plan.propagate.is_none());
    }
}
