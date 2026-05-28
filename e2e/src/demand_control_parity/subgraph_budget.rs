#[cfg(test)]
mod requests_exceeding_one_subgraph_cost_are_accepted_tests {
    use super::super::common::*;

    async fn run_case(
        fixture: Fixture,
        subgraph: &'static str,
        expected_result_by_sg: &[(&str, &str)],
        expected_calls: &[(&str, usize)],
    ) {
        let outcome = run_fixture(
            &fixture,
            &format!(
                r#"
enabled: true
mode: enforce
include_extension_metadata: true
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

        assert_top_result(json, fixture.query_file, "COST_OK");
        assert_result_by_subgraph(json, fixture.query_file, expected_result_by_sg);
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
            &[("products", "SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE")],
            &[("products", 0)],
        )
        .await
    }
    #[ntex::test]
    async fn basic_mutation() {
        run_case(
            super::super::common::basic_mutation(),
            "products",
            &[("products", "SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE")],
            &[("products", 0)],
        )
        .await
    }
    #[ntex::test]
    async fn federated_ships_required() {
        run_case(
            super::super::common::federated_ships_required(),
            "users",
            &[
                ("users", "SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE"),
                ("vehicles", "COST_OK"),
            ],
            &[("users", 0), ("vehicles", 2)],
        )
        .await
    }
    #[ntex::test]
    async fn federated_ships_fragment() {
        run_case(
            super::super::common::federated_ships_fragment(),
            "vehicles",
            &[
                ("users", "COST_OK"),
                ("vehicles", "SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE"),
            ],
            &[("users", 1), ("vehicles", 0)],
        )
        .await
    }
    #[ntex::test]
    async fn custom_costs() {
        run_case(
            super::super::common::custom_costs(),
            "subgraphWithListSize",
            &[
                ("subgraphWithCost", "COST_OK"),
                (
                    "subgraphWithListSize",
                    "SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE",
                ),
            ],
            &[("subgraphWithCost", 1), ("subgraphWithListSize", 0)],
        )
        .await
    }
}
