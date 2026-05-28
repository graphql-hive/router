#[cfg(test)]
mod actual_cost_can_vary_based_on_mode_tests {
    use super::super::common::*;

    async fn run_case(
        fixture: Fixture,
        mode: &str,
        expected_estimated: f64,
        expected_actual: f64,
        est_by_sg: &[(&str, f64)],
        act_by_sg: Option<&[(&str, f64)]>,
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
    actual_cost_mode: {mode}
    max: {MAX_COST}
"#
            ),
        )
        .await;
        let json = &outcome.json;
        let label = format!("{}_{mode}", fixture.query_file);
        assert_accepted(json, &label);
        assert_cost(json, &label, expected_estimated, expected_actual);
        for (sg, est) in est_by_sg {
            assert_cost_by_subgraph(json, &label, "estimatedCostBySubgraph", sg, *est);
        }
        match act_by_sg {
            Some(pairs) => {
                for (sg, act) in pairs {
                    assert_cost_by_subgraph(json, &label, "actualCostBySubgraph", sg, *act);
                }
            }
            None => assert_actual_by_subgraph_null(json, &label),
        }
        assert_top_result(json, &label, "COST_OK");
        assert_result_by_subgraph(json, &label, expected_ok_result_by_subgraph(&fixture));
        assert_call_counts(&outcome, &label, expected_call_counts(&fixture));
    }

    // basic_fragments: est=12, act=2; products(est=12, act=2 / null)
    #[ntex::test]
    async fn basic_fragments_by_subgraph() {
        run_case(
            basic_fragments(),
            "by_subgraph",
            12.0,
            2.0,
            &[("products", 12.0)],
            Some(&[("products", 2.0)]),
        )
        .await
    }
    #[ntex::test]
    async fn basic_fragments_by_response_shape() {
        run_case(
            basic_fragments(),
            "by_response_shape",
            12.0,
            2.0,
            &[("products", 12.0)],
            None,
        )
        .await
    }

    // basic_mutation: est=10, act=0; products(est=10, act=0 / null)
    #[ntex::test]
    async fn basic_mutation_by_subgraph() {
        run_case(
            basic_mutation(),
            "by_subgraph",
            10.0,
            0.0,
            &[("products", 10.0)],
            Some(&[("products", 0.0)]),
        )
        .await
    }
    #[ntex::test]
    async fn basic_mutation_by_response_shape() {
        run_case(
            basic_mutation(),
            "by_response_shape",
            10.0,
            0.0,
            &[("products", 10.0)],
            None,
        )
        .await
    }

    // federated_ships_required:
    // by_subgraph        : est=140, act=15, users(est=110,act=6), vehicles(est=30,act=9)
    // by_response_shape  : est=140, act=3,  est_by_sg same, act_by_sg null
    #[ntex::test]
    async fn federated_ships_required_by_subgraph() {
        run_case(
            federated_ships_required(),
            "by_subgraph",
            140.0,
            15.0,
            &[("users", 110.0), ("vehicles", 30.0)],
            Some(&[("users", 6.0), ("vehicles", 9.0)]),
        )
        .await
    }
    #[ntex::test]
    async fn federated_ships_required_by_response_shape() {
        run_case(
            federated_ships_required(),
            "by_response_shape",
            140.0,
            3.0,
            &[("users", 110.0), ("vehicles", 30.0)],
            None,
        )
        .await
    }

    // federated_ships_fragment:
    // by_subgraph        : est=40, act=15, users(est=20,act=5), vehicles(est=20,act=10)
    // by_response_shape  : est=40, act=12, est_by_sg same, act_by_sg null
    #[ntex::test]
    async fn federated_ships_fragment_by_subgraph() {
        run_case(
            federated_ships_fragment(),
            "by_subgraph",
            40.0,
            15.0,
            &[("users", 20.0), ("vehicles", 20.0)],
            Some(&[("users", 5.0), ("vehicles", 10.0)]),
        )
        .await
    }
    #[ntex::test]
    async fn federated_ships_fragment_by_response_shape() {
        run_case(
            federated_ships_fragment(),
            "by_response_shape",
            40.0,
            12.0,
            &[("users", 20.0), ("vehicles", 20.0)],
            None,
        )
        .await
    }

    // custom_costs:
    // by_subgraph        : est=127, act=124,
    //                      subgraphWithCost(121,121), subgraphWithListSize(6,3)
    // by_response_shape  : est=127, act=124, est_by_sg same, act_by_sg null
    #[ntex::test]
    async fn custom_costs_by_subgraph() {
        run_case(
            custom_costs(),
            "by_subgraph",
            127.0,
            124.0,
            &[("subgraphWithCost", 121.0), ("subgraphWithListSize", 6.0)],
            Some(&[("subgraphWithCost", 121.0), ("subgraphWithListSize", 3.0)]),
        )
        .await
    }
    #[ntex::test]
    async fn custom_costs_by_response_shape() {
        run_case(
            custom_costs(),
            "by_response_shape",
            127.0,
            124.0,
            &[("subgraphWithCost", 121.0), ("subgraphWithListSize", 6.0)],
            None,
        )
        .await
    }
}
