#[cfg(test)]
mod list_size_subgraph_inheritance_changes_estimates_tests {
    use super::super::common::*;

    async fn run_case(
        fixture: Fixture,
        subgraph: &'static str,
        list_size: u64,
        all_list_size: Option<u64>,
        subgraph_list_size: Option<u64>,
        expected_estimated: f64,
        expected_actual: f64,
    ) {
        let mut subgraph_block = String::new();
        if all_list_size.is_some() || subgraph_list_size.is_some() {
            subgraph_block.push_str("    subgraph:\n");
            if let Some(n) = all_list_size {
                subgraph_block.push_str(&format!("      all:\n        list_size: {n}\n"));
            }
            if let Some(n) = subgraph_list_size {
                subgraph_block.push_str(&format!(
                    "      subgraphs:\n        {subgraph}:\n          list_size: {n}\n"
                ));
            }
        }

        let yaml = format!(
            r#"
enabled: true
mode: enforce
expose_headers:
  estimated: true
  actual: true
  max: true
strategy:
  static_estimated:
    list_size: {list_size}
    actual_cost_mode: by_subgraph
    max: {MAX_COST}
{subgraph_block}"#
        );

        let outcome = run_fixture(&fixture, &yaml).await;
        let json = &outcome.json;
        let label = format!(
            "{}_{list_size}_{}_{}",
            fixture.query_file,
            all_list_size
                .map(|n| n.to_string())
                .unwrap_or("null".into()),
            subgraph_list_size
                .map(|n| n.to_string())
                .unwrap_or("null".into()),
        );
        assert_accepted(json, &label);
        assert_cost(&outcome, &label, expected_estimated, expected_actual);
        assert_call_counts(&outcome, &label, expected_call_counts(&fixture));
    }

    // ---- basic_fragments × products ---------------------------
    // actual is always 2.0, products actual is always 2.0.

    #[ntex::test]
    async fn basic_fragments_1_null_null() {
        run_case(basic_fragments(), "products", 1, None, None, 3.0, 2.0).await
    }
    #[ntex::test]
    async fn basic_fragments_1_2_null() {
        run_case(basic_fragments(), "products", 1, Some(2), None, 4.0, 2.0).await
    }
    #[ntex::test]
    async fn basic_fragments_1_null_3() {
        run_case(basic_fragments(), "products", 1, None, Some(3), 5.0, 2.0).await
    }
    #[ntex::test]
    async fn basic_fragments_1_2_3() {
        run_case(basic_fragments(), "products", 1, Some(2), Some(3), 5.0, 2.0).await
    }
    #[ntex::test]
    async fn basic_fragments_10_null_null() {
        run_case(basic_fragments(), "products", 10, None, None, 12.0, 2.0).await
    }
    #[ntex::test]
    async fn basic_fragments_10_2_null() {
        run_case(basic_fragments(), "products", 10, Some(2), None, 4.0, 2.0).await
    }
    #[ntex::test]
    async fn basic_fragments_10_null_3() {
        run_case(basic_fragments(), "products", 10, None, Some(3), 5.0, 2.0).await
    }
    #[ntex::test]
    async fn basic_fragments_10_2_3() {
        run_case(
            basic_fragments(),
            "products",
            10,
            Some(2),
            Some(3),
            5.0,
            2.0,
        )
        .await
    }

    #[ntex::test]
    async fn federated_ships_fragment_1_null_null() {
        run_case(
            federated_ships_fragment(),
            "vehicles",
            1,
            None,
            None,
            4.0,
            15.0,
        )
        .await
    }
    #[ntex::test]
    async fn federated_ships_fragment_1_2_null() {
        run_case(
            federated_ships_fragment(),
            "vehicles",
            1,
            Some(2),
            None,
            8.0,
            15.0,
        )
        .await
    }
    #[ntex::test]
    async fn federated_ships_fragment_1_null_3() {
        run_case(
            federated_ships_fragment(),
            "vehicles",
            1,
            None,
            Some(3),
            8.0,
            15.0,
        )
        .await
    }
    #[ntex::test]
    async fn federated_ships_fragment_1_2_3() {
        run_case(
            federated_ships_fragment(),
            "vehicles",
            1,
            Some(2),
            Some(3),
            10.0,
            15.0,
        )
        .await
    }
    #[ntex::test]
    async fn federated_ships_fragment_10_null_null() {
        run_case(
            federated_ships_fragment(),
            "vehicles",
            10,
            None,
            None,
            40.0,
            15.0,
        )
        .await
    }
    #[ntex::test]
    async fn federated_ships_fragment_10_2_null() {
        run_case(
            federated_ships_fragment(),
            "vehicles",
            10,
            Some(2),
            None,
            8.0,
            15.0,
        )
        .await
    }
    #[ntex::test]
    async fn federated_ships_fragment_10_null_3() {
        run_case(
            federated_ships_fragment(),
            "vehicles",
            10,
            None,
            Some(3),
            26.0,
            15.0,
        )
        .await
    }
    #[ntex::test]
    async fn federated_ships_fragment_10_2_3() {
        run_case(
            federated_ships_fragment(),
            "vehicles",
            10,
            Some(2),
            Some(3),
            10.0,
            15.0,
        )
        .await
    }
}
