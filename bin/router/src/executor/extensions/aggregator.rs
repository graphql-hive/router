use std::collections::HashMap;

use ahash::HashMap as AHashMap;
use sonic_rs::Value as SonicValue;

use crate::executor::response::value::Value;

use super::plan::{ExtensionsMergeStrategy, ExtensionsPlan};

const RESERVED_KEY: &str = "queryPlan";

/// Subgraph extensions collected for the response, as the `sonic_rs` values the response's
/// extensions are
#[derive(Default)]
pub struct ExtensionsAggregator {
    entries: AHashMap<String, (ExtensionsMergeStrategy, Vec<SonicValue>)>,
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
                    let arr: sonic_rs::Array = values.into_iter().collect();
                    SonicValue::from(arr)
                }
                ExtensionsMergeStrategy::First | ExtensionsMergeStrategy::Last => values
                    .into_iter()
                    .next()
                    .expect("First/Last entry guaranteed non-empty by write()"),
            };
            target.entry(key).or_insert(sonic_val);
        }
    }
}

/// Whether the plan passes the key on
fn propagated(plan: &ExtensionsPlan, key: &str) -> Option<ExtensionsMergeStrategy> {
    let propagate = plan.propagate.as_ref()?;
    if key == RESERVED_KEY {
        return None;
    }
    if let Some(ref allow) = propagate.allow {
        if !allow.contains(key) {
            return None;
        }
    }
    Some(propagate.strategy)
}

/// Apply top-level keys from `subgraph_extensions` into the aggregator,
/// filtered and merged per the plan.
pub fn apply_subgraph_extensions(
    plan: &ExtensionsPlan,
    subgraph_extensions: &Value<'_>,
    agg: &mut ExtensionsAggregator,
) {
    let Value::Object(entries) = subgraph_extensions else {
        return;
    };
    for (key, val) in entries {
        if let Some(strategy) = propagated(plan, key) {
            agg.write(key, SonicValue::from(val), strategy);
        }
    }
}

/// [`apply_subgraph_extensions`] for a subgraph's `extensions` as JSON, read straight into
/// the values the response keeps. Nothing is read when the plan passes nothing on.
pub fn apply_subgraph_extensions_json(
    plan: &ExtensionsPlan,
    json: &[u8],
    agg: &mut ExtensionsAggregator,
) {
    if plan.propagate.is_none() {
        return;
    }
    let Ok(mut extensions) = sonic_rs::from_slice::<sonic_rs::Object>(json) else {
        return;
    };
    for (key, val) in extensions.iter_mut() {
        if let Some(strategy) = propagated(plan, key) {
            agg.write(key, std::mem::take(val), strategy);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;
    use crate::executor::extensions::plan::{
        ExtensionsMergeStrategy, ExtensionsPlan, ExtensionsPropagatePlan,
    };
    use crate::executor::response::value::Value;
    use ahash::HashSet;
    use sonic_rs::json;

    fn make_plan(strategy: ExtensionsMergeStrategy, allow: Option<Vec<&str>>) -> ExtensionsPlan {
        ExtensionsPlan {
            propagate: Some(ExtensionsPropagatePlan {
                strategy,
                allow: allow.map(|keys| {
                    keys.into_iter()
                        .map(|s| s.to_string())
                        .collect::<HashSet<_>>()
                }),
            }),
        }
    }

    fn obj<'a>(pairs: Vec<(&'a str, Value<'a>)>) -> Value<'a> {
        Value::Object(pairs)
    }

    // Value::String holds a Cow<str>; Borrowed wraps a &'static str with no allocation
    fn str(v: &'static str) -> Value<'static> {
        Value::String(Cow::Borrowed(v))
    }

    #[test]
    fn test_first_keeps_first_value() {
        let plan = make_plan(ExtensionsMergeStrategy::First, None);
        let mut agg = ExtensionsAggregator::default();

        apply_subgraph_extensions(&plan, &obj(vec![("foo", str("a"))]), &mut agg);
        apply_subgraph_extensions(&plan, &obj(vec![("foo", str("b"))]), &mut agg);

        let mut out = HashMap::new();
        agg.merge_into(&mut out);

        assert_eq!(out["foo"], json!("a"));
    }

    #[test]
    fn test_last_keeps_last_value() {
        let plan = make_plan(ExtensionsMergeStrategy::Last, None);
        let mut agg = ExtensionsAggregator::default();

        apply_subgraph_extensions(&plan, &obj(vec![("foo", str("a"))]), &mut agg);
        apply_subgraph_extensions(&plan, &obj(vec![("foo", str("b"))]), &mut agg);

        let mut out = HashMap::new();
        agg.merge_into(&mut out);

        assert_eq!(out["foo"], json!("b"));
    }

    #[test]
    fn test_append_collects_all_values() {
        let plan = make_plan(ExtensionsMergeStrategy::Append, None);
        let mut agg = ExtensionsAggregator::default();

        apply_subgraph_extensions(&plan, &obj(vec![("foo", str("a"))]), &mut agg);
        apply_subgraph_extensions(&plan, &obj(vec![("foo", str("b"))]), &mut agg);

        let mut out = HashMap::new();
        agg.merge_into(&mut out);

        assert_eq!(out["foo"], json!(["a", "b"]));
    }

    #[test]
    fn test_append_single_value_is_array() {
        let plan = make_plan(ExtensionsMergeStrategy::Append, None);
        let mut agg = ExtensionsAggregator::default();

        apply_subgraph_extensions(&plan, &obj(vec![("foo", str("a"))]), &mut agg);

        let mut out = HashMap::new();
        agg.merge_into(&mut out);

        assert_eq!(out["foo"], json!(["a"]));
    }

    #[test]
    fn test_whitelist_drops_unlisted_key() {
        let plan = make_plan(ExtensionsMergeStrategy::Last, Some(vec!["foo"]));
        let mut agg = ExtensionsAggregator::default();

        apply_subgraph_extensions(
            &plan,
            &obj(vec![("foo", str("a")), ("bar", str("b"))]),
            &mut agg,
        );

        let mut out = HashMap::new();
        agg.merge_into(&mut out);

        assert!(out.contains_key("foo"));
        assert!(!out.contains_key("bar"));
    }

    #[test]
    fn test_no_whitelist_keeps_all_keys() {
        let plan = make_plan(ExtensionsMergeStrategy::Last, None);
        let mut agg = ExtensionsAggregator::default();

        apply_subgraph_extensions(
            &plan,
            &obj(vec![("foo", str("a")), ("bar", str("b"))]),
            &mut agg,
        );

        let mut out = HashMap::new();
        agg.merge_into(&mut out);

        assert!(out.contains_key("foo"));
        assert!(out.contains_key("bar"));
    }

    #[test]
    fn test_merge_into_existing_key_not_overwritten() {
        let plan = make_plan(ExtensionsMergeStrategy::Last, None);
        let mut agg = ExtensionsAggregator::default();

        apply_subgraph_extensions(&plan, &obj(vec![("foo", str("a"))]), &mut agg);

        let mut out = HashMap::new();
        out.insert("foo".to_string(), json!("existing"));
        agg.merge_into(&mut out);

        assert_eq!(out["foo"], json!("existing"));
    }

    #[test]
    fn test_reserved_query_plan_key_is_ignored() {
        let plan = make_plan(ExtensionsMergeStrategy::Last, None);
        let mut agg = ExtensionsAggregator::default();

        apply_subgraph_extensions(
            &plan,
            &obj(vec![("queryPlan", str("a")), ("foo", str("b"))]),
            &mut agg,
        );

        let mut out = HashMap::new();
        agg.merge_into(&mut out);

        assert!(!out.contains_key("queryPlan"));
        assert!(out.contains_key("foo"));
    }

    #[test]
    fn test_no_propagate_plan_is_noop() {
        let plan = ExtensionsPlan { propagate: None };
        let mut agg = ExtensionsAggregator::default();

        apply_subgraph_extensions(&plan, &obj(vec![("foo", str("a"))]), &mut agg);

        let mut out = HashMap::new();
        agg.merge_into(&mut out);

        assert!(out.is_empty());
    }

    #[test]
    fn test_non_object_extensions_ignored() {
        let plan = make_plan(ExtensionsMergeStrategy::Last, None);
        let mut agg = ExtensionsAggregator::default();

        apply_subgraph_extensions(&plan, &Value::String(Cow::Borrowed("oops")), &mut agg);

        let mut out = HashMap::new();
        agg.merge_into(&mut out);

        assert!(out.is_empty());
    }
}
