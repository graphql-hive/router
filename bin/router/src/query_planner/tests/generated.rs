//! Random operations, from `bin/differential`'s generator, planned against every fixture
//! supergraph. Planning shouldn't panic or fail, the debug checks have to hold, and two planners
//! built from the same supergraph give the same plan. Two planners, not one planner twice: each
//! `HashMap` gets its own seed, so only a second planner catches plans that depend on it. A failing operation is written to `tests/generated/`, add it as a
//! regular test.
//!
//! Two errors are left alone: the walker finding no path, which is search and not the fetch
//! layer (and what `corrupted-supergraph-node-id` is for), and an empty plan, when every field
//! is skipped. Both happen on `main` too.
//!
//! `QP_GENERATED_OPS` sets how many operations per supergraph, 10 by default; set it to 200 for
//! a larger investigation run.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use graphql_differential::{GeneratorConfig, QueryGenerator};
use graphql_tools::validation::{rules::default_rules_validation_plan, validate::validate};

use crate::query_planner::{
    ast::normalization::normalize_operation,
    consumer_schema::{
        prune_inacessible::PruneInaccessible, strip_schema_internals::StripSchemaInternals,
        ConsumerSchema,
    },
    graph::edge::PlannerOverrideContext,
    planner::Planner,
    utils::{
        cancellation::CancellationToken,
        parsing::{parse_operation, parse_schema},
    },
};

fn fixtures() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixture");
    let mut found: Vec<PathBuf> = ["tests", "issues"]
        .iter()
        .flat_map(|dir| std::fs::read_dir(root.join(dir)).expect("fixture dir"))
        .map(|entry| entry.expect("fixture").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "graphql"))
        .collect();
    found.sort();
    found
}

/// Plans `operation`, or says why it couldn't.
fn plan(planner: &Planner, operation: &str) -> Result<String, String> {
    let document = parse_operation(operation);
    let normalized = normalize_operation(&planner.supergraph, &document, None)
        .map_err(|err| format!("normalization: {err}"))?;
    planner
        .plan_from_normalized_operation(
            normalized.executable_operation(),
            PlannerOverrideContext::default(),
            &CancellationToken::new(),
        )
        .map(|plan| plan.to_string())
        .map_err(|err| format!("planning: {err}"))
}

#[test]
fn generated_operations_plan() {
    let ops: u64 = std::env::var("QP_GENERATED_OPS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(10);
    let config = GeneratorConfig {
        max_depth: 6,
        max_width: 6,
        ..GeneratorConfig::default()
    };
    let rules = default_rules_validation_plan();
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/query_planner/tests/generated");
    let mut failures = Vec::new();
    let (mut planned, mut left_alone) = (0, 0);

    for fixture in fixtures() {
        let name = fixture.file_stem().unwrap().to_string_lossy().to_string();
        let supergraph = parse_schema(&std::fs::read_to_string(&fixture).unwrap());
        let Ok(planner) = Planner::new_from_supergraph(&supergraph, Default::default()) else {
            continue;
        };
        let other_planner = Planner::new_from_supergraph(&supergraph, Default::default()).unwrap();
        // The generator doesn't always make valid operations, and the router refuses those.
        // Validate against the client-visible schema, with built-in scalars and directives.
        let api =
            StripSchemaInternals::strip_schema_internals(&PruneInaccessible::prune(&supergraph));
        let schema = ConsumerSchema::new_from_supergraph(&supergraph);
        for seed in 0..ops {
            let case = QueryGenerator::new(&api, seed, config.clone()).generate();
            if !validate(&schema.document, &parse_operation(&case.document), &rules).is_empty() {
                continue;
            }
            planned += 1;
            let result = catch_unwind(AssertUnwindSafe(|| {
                let first = plan(&planner, &case.document)?;
                let second = plan(&other_planner, &case.document)?;
                if first != second {
                    return Err(format!("two planners, two plans:\n{first}\n{second}"));
                }
                Ok(())
            }));
            let error = match result {
                Ok(Ok(())) => continue,
                Ok(Err(error))
                    if error.contains("walker failed to locate path")
                        || error.ends_with("Failed to build a plan") =>
                {
                    left_alone += 1;
                    continue;
                }
                Ok(Err(error)) => error,
                Err(panic) => format!(
                    "panic: {}",
                    panic
                        .downcast_ref::<String>()
                        .map(String::as_str)
                        .or_else(|| panic.downcast_ref::<&str>().copied())
                        .unwrap_or("?")
                ),
            };
            std::fs::create_dir_all(&out).unwrap();
            let file = out.join(format!("{name}-{seed}.graphql"));
            std::fs::write(&file, &case.document).unwrap();
            failures.push(format!("{}: {error}", file.display()));
        }
    }

    eprintln!("planned {planned} operations, left {left_alone} errors alone");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
