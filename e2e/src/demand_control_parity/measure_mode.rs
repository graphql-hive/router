#[cfg(test)]
mod requests_exceeding_max_are_not_rejected_in_measure_mode_tests {
    use super::super::common::parametric_per_fixture;
    use super::super::common::*;

    async fn run_case(fixture: Fixture) {
        let outcome = run_fixture(
            &fixture,
            r#"
enabled: true
mode: measure
include_extension_metadata: true
strategy:
  static_estimated:
    list_size: 100
    max: 1
    subgraph:
      all:
        max: 1
"#,
        )
        .await;
        let json = &outcome.json;
        // Must NOT be rejected: no COST_ESTIMATED_TOO_EXPENSIVE error.
        let code = json["errors"][0]["extensions"]["code"].as_str();
        assert_ne!(
            code,
            Some("COST_ESTIMATED_TOO_EXPENSIVE"),
            "[{}] measure mode must not reject: {json}",
            fixture.query_file
        );
        // The cost extension still records the over-budget result.
        assert_top_result(json, fixture.query_file, "COST_ESTIMATED_TOO_EXPENSIVE");
        // In measure mode, every subgraph that's over the per-subgraph
        // budget surfaces SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE.
        let by_sg: Vec<(&str, &str)> = expected_call_counts(&fixture)
            .iter()
            .map(|(sg, _)| (*sg, "SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE"))
            .collect();
        assert_result_by_subgraph(json, fixture.query_file, &by_sg);
        // And subgraphs are still actually called (full execution).
        assert_call_counts(&outcome, fixture.query_file, expected_call_counts(&fixture));
    }

    parametric_per_fixture!();
}
