#[cfg(test)]
mod requests_exceeding_max_are_rejected_regardless_of_subgraph_config_tests {
    use super::super::common::parametric_per_fixture;
    use super::super::common::*;

    async fn run_case(fixture: Fixture) {
        let outcome = run_fixture(
            &fixture,
            &format!(
                r#"
enabled: true
mode: enforce
strategy:
  static_estimated:
    list_size: 10
    max: 1
    subgraph:
      all:
        max: {MAX_COST}
"#
            ),
        )
        .await;
        assert_rejected_estimated(&outcome.json, fixture.query_file);
        for (sg, _) in expected_call_counts(&fixture) {
            assert_call_counts(&outcome, fixture.query_file, &[(*sg, 0)]);
        }
    }

    parametric_per_fixture!();
}
