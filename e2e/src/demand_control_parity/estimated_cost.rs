#[cfg(test)]
mod demand_control_parity_tests {
    use std::path::PathBuf;

    use sonic_rs::JsonValueTrait;

    use crate::testkit::{ClientResponseExt, TestRouter};

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("demand_control")
            .join(name)
    }

    fn read_fixture(name: &str) -> String {
        std::fs::read_to_string(fixture_path(name))
            .unwrap_or_else(|e| panic!("must read fixture {name}: {e}"))
    }

    /// Asserts that the estimated cost reported by the router matches the
    /// expected reference value. Used by the
    /// `requests_within_max_are_accepted` cases below.
    /// reference value. We force a rejection by configuring `max = 0`, then
    /// parse the actual estimated cost out of the rejection message:
    /// `Operation estimated cost <N> exceeds configured max cost 0`.
    /// This avoids ever hitting subgraphs (which we do not stand up in this
    /// estimated-only parity phase) and makes mismatches surface as clear
    /// numeric diffs instead of timeouts.
    async fn assert_estimated_cost_parity(
        schema_fixture: &str,
        query_fixture: &str,
        list_size: u64,
        expected_cost: u64,
    ) {
        let schema_path = fixture_path(schema_fixture);
        let query = read_fixture(query_fixture);

        let router = TestRouter::builder()
            .inline_config(format!(
                r#"
                supergraph:
                    source: file
                    path: "{schema}"
                demand_control:
                    enabled: true
                    operation_cost:
                      max: 0
                      mode: enforce
                    subgraphs_budget:
                      mode: enforce
                    default_list_size:
                      all: {list_size}
                    expose_headers:
                      estimated: true
                      actual: true
                      max: true
                "#,
                schema = schema_path.to_string_lossy(),
                list_size = list_size,
            ))
            .build()
            .start()
            .await;

        let res = router.send_graphql_request(&query, None, None).await;
        let json = res.json_body().await;

        assert_eq!(
            json["errors"][0]["extensions"]["code"].as_str(),
            Some("COST_ESTIMATED_TOO_EXPENSIVE"),
            "expected COST_ESTIMATED_TOO_EXPENSIVE rejection for {query_fixture}, got: {json}"
        );

        let message = json["errors"][0]["message"]
            .as_str()
            .unwrap_or_else(|| panic!("missing rejection message for {query_fixture}: {json}"));

        assert_eq!(message, "Operation estimated cost exceeds max cost");

        println!("h_v: {:?}", res.headers().get("x-cost-actual"));

        let estimated_cost: u64 = res
            .header("x-cost-estimated")
            .and_then(|n| n.to_str().unwrap().parse().ok())
            .unwrap_or_else(|| panic!("could not parse estimated cost from headers"));

        assert_eq!(
            estimated_cost, expected_cost,
            "estimated cost parity drift for {query_fixture}: \
             expected {expected_cost}, our router reports {estimated_cost}"
        );
    }

    // Reference: `requests_within_max_are_accepted` case::eq:
    // basic_fragments() with max=12.0 → estimated cost is exactly 12 (list_size=10).
    #[ntex::test]
    async fn estimated_cost_parity_basic_fragments() {
        assert_estimated_cost_parity(
            "basic_supergraph_schema.graphql",
            "basic_fragments_query.graphql",
            10,
            12,
        )
        .await;
    }

    // Reference: basic_mutation() with max=10.0.
    #[ntex::test]
    async fn estimated_cost_parity_basic_mutation() {
        assert_estimated_cost_parity(
            "basic_supergraph_schema.graphql",
            "basic_mutation.graphql",
            10,
            10,
        )
        .await;
    }

    // Reference: federated_ships_required() with max=140.0.
    #[ntex::test]
    async fn estimated_cost_parity_federated_ships_required() {
        assert_estimated_cost_parity(
            "federated_ships_schema.graphql",
            "federated_ships_required_query.graphql",
            10,
            140,
        )
        .await;
    }

    // Reference: federated_ships_fragment() with max=40.0.
    #[ntex::test]
    async fn estimated_cost_parity_federated_ships_fragment() {
        assert_estimated_cost_parity(
            "federated_ships_schema.graphql",
            "federated_ships_fragment_query.graphql",
            10,
            40,
        )
        .await;
    }

    // Reference: custom_costs() with max=127.0.
    #[ntex::test]
    async fn estimated_cost_parity_custom_costs() {
        assert_estimated_cost_parity(
            "custom_cost_schema.graphql",
            "custom_cost_query.graphql",
            10,
            127,
        )
        .await;
    }
}
