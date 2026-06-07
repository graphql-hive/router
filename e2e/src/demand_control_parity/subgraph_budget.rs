#[cfg(test)]
mod requests_exceeding_one_subgraph_cost_are_accepted_tests {
    use super::super::common::*;

    async fn run_case(fixture: Fixture, subgraph: &'static str, expected_calls: &[(&str, usize)]) {
        let outcome = run_fixture(
            &fixture,
            &format!(
                r#"
enabled: true
mode: enforce
expose_headers:
  estimated: true
  actual: true
  max: true
strategy:
  static_estimated:
    list_size: 10
    max: {MAX_COST}
    subgraph:
      subgraphs:
        {subgraph}:
          max: 1
"#
            ),
        )
        .await;
        let json = &outcome.json;

        // Top-level must not be rejected with COST_ESTIMATED_TOO_EXPENSIVE.
        let code = json["errors"][0]["extensions"]["code"].as_str();
        assert_ne!(
            code,
            Some("COST_ESTIMATED_TOO_EXPENSIVE"),
            "[{}] supergraph-level rejection unexpected: {json}",
            fixture.query_file
        );

        assert_call_counts(&outcome, fixture.query_file, expected_calls);
    }

    // (fixture, subgraph_to_throttle) — same pairs as the reference.
    // Expected result_by_subgraph and call_counts are taken verbatim
    // from the reference snapshots.
    #[ntex::test]
    async fn basic_fragments() {
        run_case(
            super::super::common::basic_fragments(),
            "products",
            &[("products", 0)],
        )
        .await
    }
    #[ntex::test]
    async fn basic_mutation() {
        run_case(
            super::super::common::basic_mutation(),
            "products",
            &[("products", 0)],
        )
        .await
    }
    #[ntex::test]
    async fn federated_ships_required() {
        run_case(
            super::super::common::federated_ships_required(),
            "users",
            &[("users", 0), ("vehicles", 2)],
        )
        .await
    }
    #[ntex::test]
    async fn federated_ships_fragment() {
        run_case(
            super::super::common::federated_ships_fragment(),
            "vehicles",
            &[("users", 1), ("vehicles", 0)],
        )
        .await
    }
    #[ntex::test]
    async fn custom_costs() {
        run_case(
            super::super::common::custom_costs(),
            "subgraphWithListSize",
            &[("subgraphWithCost", 1), ("subgraphWithListSize", 0)],
        )
        .await
    }
}
