#[cfg(test)]
mod requests_within_max_are_accepted_tests {
    use super::super::common::*;

    /// Type alias to keep test signatures readable: per-subgraph
    /// (name, expected_estimated, expected_actual).
    type SgExpect = &'static [(&'static str, f64, f64)];

    async fn run_eq(
        fixture: Fixture,
        max: u64,
        expected_estimated: f64,
        expected_actual: f64,
        by_sg: SgExpect,
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
    actual_cost_mode: by_subgraph
    max: {max}
"#
            ),
        )
        .await;
        let json = &outcome.json;
        let label = format!("{} eq max={max}", fixture.query_file);
        assert_accepted(json, &label);
        assert_cost(json, &label, expected_estimated, expected_actual);
        for (sg, est, act) in by_sg {
            assert_cost_by_subgraph(json, &label, "estimatedCostBySubgraph", sg, *est);
            assert_cost_by_subgraph(json, &label, "actualCostBySubgraph", sg, *act);
        }
        assert_top_result(json, &label, "COST_OK");
        assert_result_by_subgraph(json, &label, expected_ok_result_by_subgraph(&fixture));
        assert_call_counts(&outcome, &label, expected_call_counts(&fixture));
    }

    // basic_fragments: estimated=12, actual=2; products(est=12, act=2)
    const BASIC_FRAGMENTS_SG: SgExpect = &[("products", 12.0, 2.0)];
    #[ntex::test]
    async fn basic_fragments_eq() {
        run_eq(basic_fragments(), 12, 12.0, 2.0, BASIC_FRAGMENTS_SG).await
    }
    #[ntex::test]
    async fn basic_fragments_lt() {
        run_eq(basic_fragments(), 15, 12.0, 2.0, BASIC_FRAGMENTS_SG).await
    }

    // basic_mutation: estimated=10, actual=0; products(est=10, act=0)
    const BASIC_MUTATION_SG: SgExpect = &[("products", 10.0, 0.0)];
    #[ntex::test]
    async fn basic_mutation_eq() {
        run_eq(basic_mutation(), 10, 10.0, 0.0, BASIC_MUTATION_SG).await
    }
    #[ntex::test]
    async fn basic_mutation_lt() {
        run_eq(basic_mutation(), 15, 10.0, 0.0, BASIC_MUTATION_SG).await
    }

    // federated_ships_required: estimated=140, actual=15;
    // users(est=110, act=6), vehicles(est=30, act=9)
    const FED_SHIPS_REQUIRED_SG: SgExpect = &[("users", 110.0, 6.0), ("vehicles", 30.0, 9.0)];
    #[ntex::test]
    async fn federated_ships_required_eq() {
        run_eq(
            federated_ships_required(),
            140,
            140.0,
            15.0,
            FED_SHIPS_REQUIRED_SG,
        )
        .await
    }
    #[ntex::test]
    async fn federated_ships_required_lt() {
        run_eq(
            federated_ships_required(),
            150,
            140.0,
            15.0,
            FED_SHIPS_REQUIRED_SG,
        )
        .await
    }

    // federated_ships_fragment: estimated=40, actual=15;
    // users(est=20, act=5), vehicles(est=20, act=10)
    const FED_SHIPS_FRAGMENT_SG: SgExpect = &[("users", 20.0, 5.0), ("vehicles", 20.0, 10.0)];
    #[ntex::test]
    async fn federated_ships_fragment_eq() {
        run_eq(
            federated_ships_fragment(),
            40,
            40.0,
            15.0,
            FED_SHIPS_FRAGMENT_SG,
        )
        .await
    }
    #[ntex::test]
    async fn federated_ships_fragment_lt() {
        run_eq(
            federated_ships_fragment(),
            50,
            40.0,
            15.0,
            FED_SHIPS_FRAGMENT_SG,
        )
        .await
    }

    // custom_costs: estimated=127, actual=124;
    // subgraphWithCost(est=121, act=121), subgraphWithListSize(est=6, act=3)
    const CUSTOM_COSTS_SG: SgExpect = &[
        ("subgraphWithCost", 121.0, 121.0),
        ("subgraphWithListSize", 6.0, 3.0),
    ];
    #[ntex::test]
    async fn custom_costs_eq() {
        run_eq(custom_costs(), 127, 127.0, 124.0, CUSTOM_COSTS_SG).await
    }
    #[ntex::test]
    async fn custom_costs_lt() {
        run_eq(custom_costs(), 130, 127.0, 124.0, CUSTOM_COSTS_SG).await
    }
}
