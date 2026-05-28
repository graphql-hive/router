#[cfg(test)]
mod requests_exceeding_max_are_rejected_tests {
    use super::super::common::parametric_per_fixture;
    use super::super::common::*;

    async fn run_case(fixture: Fixture) {
        let outcome = run_fixture(
            &fixture,
            r#"
enabled: true
mode: enforce
strategy:
  static_estimated:
    list_size: 100
    max: 1
"#,
        )
        .await;
        assert_rejected_estimated(&outcome.json, fixture.query_file);
        // The reference snapshot reports `subgraph_call_count: null` for
        // rejected requests — no subgraph must have been called.
        for (sg, _) in expected_call_counts(&fixture) {
            assert_call_counts(&outcome, fixture.query_file, &[(*sg, 0)]);
        }
    }

    parametric_per_fixture!();
}
