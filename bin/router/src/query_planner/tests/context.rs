use super::testkit::build_query_plan_with_defaults;
use crate::query_planner::utils::parsing::parse_operation;

fn request_rounds(node: &serde_json::Value) -> usize {
    match node["kind"].as_str() {
        Some("Fetch" | "BatchFetch") => 1,
        Some("Sequence") => node["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(request_rounds)
            .sum(),
        Some("Parallel") => node["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(request_rounds)
            .max()
            .unwrap_or_default(),
        Some("Condition") => {
            request_rounds(&node["ifClause"]).max(request_rounds(&node["elseClause"]))
        }
        Some("Flatten") => request_rounds(&node["node"]),
        _ => 0,
    }
}

fn context_paths(
    value: &serde_json::Value,
    service: &str,
    paths: &mut std::collections::BTreeSet<(String, Vec<String>)>,
) {
    if let Some(object) = value.as_object() {
        let service = object
            .get("serviceName")
            .and_then(|s| s.as_str())
            .unwrap_or(service);
        if let Some(rewrites) = object.get("contextRewrites").and_then(|v| v.as_array()) {
            for rewrite in rewrites {
                let path = rewrite["path"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|segment| {
                        if let Some(key) = segment["Key"].as_str() {
                            Some(
                                if key.starts_with("_internal_qp_alias_") {
                                    "currency"
                                } else {
                                    key
                                }
                                .to_string(),
                            )
                        } else {
                            segment
                                .as_str()
                                .filter(|s| !s.starts_with("... on "))
                                .map(|s| if s == "Parent" { ".." } else { s }.to_string())
                        }
                    })
                    .collect();
                paths.insert((service.to_string(), path));
            }
        }
        for value in object.values() {
            context_paths(value, service, paths);
        }
    } else if let Some(array) = value.as_array() {
        for value in array {
            context_paths(value, service, paths);
        }
    }
}

#[test]
fn context_plans() {
    for fixture in ["context-basic", "context-cross"] {
        let reference: serde_json::Value = serde_json::from_str(&super::testkit::read_supergraph(
            &format!("fixture/tests/{fixture}.apollo.json"),
        ))
        .unwrap();
        for (index, case) in reference.as_array().unwrap().iter().enumerate() {
            let query = case["query"].as_str().unwrap();
            let plan = build_query_plan_with_defaults(
                &format!("fixture/tests/{fixture}.supergraph.graphql"),
                parse_operation(query),
            )
            .unwrap_or_else(|error| panic!("{query}: {error}"));
            let serialized = serde_json::to_value(&plan).unwrap();
            assert_eq!(
                request_rounds(&serialized["node"]),
                request_rounds(&case["plan"]["node"]),
                "{query}"
            );
            let mut actual = std::collections::BTreeSet::new();
            let mut expected = std::collections::BTreeSet::new();
            context_paths(&serialized, "", &mut actual);
            context_paths(&case["plan"], "", &mut expected);
            assert_eq!(actual, expected, "{query}");
            insta::assert_snapshot!(format!("{fixture}_{index}"), plan.to_string());
            insta::assert_json_snapshot!(format!("{fixture}_{index}_json"), plan);
        }
    }
}
