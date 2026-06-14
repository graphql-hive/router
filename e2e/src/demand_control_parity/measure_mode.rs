#[cfg(test)]
mod requests_exceeding_max_are_not_rejected_in_measure_mode_tests {
    use super::super::common::parametric_per_fixture;
    use super::super::common::*;

    async fn run_case(fixture: Fixture) {
        let outcome = run_fixture(
            &fixture,
            r#"
enabled: true
operation_cost:
  max: 1
  mode: measure
default_list_size:
  all: 100
subgraphs_budget:
  mode: measure
  all: 1
expose_headers:
  estimated: true
  actual: true
  max: true
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
        // And subgraphs are still actually called (full execution).
        assert_call_counts(&outcome, fixture.query_file, expected_call_counts(&fixture));
    }

    parametric_per_fixture!();
}
