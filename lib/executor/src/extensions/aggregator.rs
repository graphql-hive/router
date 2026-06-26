use std::collections::HashMap;

use ahash::{HashMap as AHashMap, HashMapExt};
use sonic_rs::Value as SonicValue;

use crate::response::value::Value;

use super::plan::{ExtensionsMergeStrategy, ExtensionsPlan};

const RESERVED_KEY: &str = "queryPlan";

pub struct ExtensionsAggregator {
    entries: AHashMap<String, (ExtensionsMergeStrategy, Vec<SonicValue>)>,
}

impl Default for ExtensionsAggregator {
    fn default() -> Self {
        Self {
            entries: AHashMap::new(),
        }
    }
}

impl ExtensionsAggregator {
    fn write(&mut self, key: &str, value: SonicValue, strategy: ExtensionsMergeStrategy) {
        match self.entries.get_mut(key) {
            None => {
                self.entries
                    .insert(key.to_string(), (strategy, vec![value]));
            }
            Some((_, values)) => match strategy {
                ExtensionsMergeStrategy::First => {
                    // ignore
                }
                ExtensionsMergeStrategy::Last => {
                    values.clear();
                    values.push(value);
                }
                ExtensionsMergeStrategy::Append => {
                    values.push(value);
                }
            },
        }
    }

    /// Drain all collected extensions into the target map.
    /// Existing keys in target win (router/plugin-set extensions are authoritative).
    pub fn merge_into(self, target: &mut HashMap<String, SonicValue>) {
        for (key, (strategy, values)) in self.entries {
            let sonic_val = match strategy {
                ExtensionsMergeStrategy::Append => {
                    SonicValue::from(values.into_iter().collect::<sonic_rs::Array>())
                }
                ExtensionsMergeStrategy::First | ExtensionsMergeStrategy::Last => {
                    // guaranteed non-empty by write()
                    values.into_iter().next().unwrap()
                }
            };
            target.entry(key).or_insert(sonic_val);
        }
    }
}

/// Apply top-level keys from `subgraph_extensions` into the aggregator,
/// filtered and merged per the plan.
pub fn apply_subgraph_extensions(
    plan: &ExtensionsPlan,
    subgraph_extensions: &Value<'_>,
    agg: &mut ExtensionsAggregator,
) {
    let Some(ref propagate) = plan.propagate else {
        return;
    };

    let Value::Object(entries) = subgraph_extensions else {
        return;
    };

    for (key, val) in entries {
        if *key == RESERVED_KEY {
            continue;
        }
        if let Some(ref allow) = propagate.allow {
            if !allow.contains(*key) {
                continue;
            }
        }
        let sonic_val = match sonic_rs::to_value(val) {
            Ok(v) => v,
            Err(_) => continue,
        };
        agg.write(key, sonic_val, propagate.strategy);
    }
}
