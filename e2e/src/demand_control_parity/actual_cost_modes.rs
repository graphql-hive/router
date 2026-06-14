#[cfg(test)]
mod actual_cost_can_vary_based_on_mode_tests {
    use super::super::common::*;

    async fn run_case(fixture: Fixture, mode: &str, expected_estimated: f64, expected_actual: f64) {
        let outcome = run_fixture(
            &fixture,
            &format!(
                r#"
enabled: true
operation_cost:
  max: {MAX_COST}
  mode: enforce
default_list_size:
  all: 10
subgraphs_budget:
  mode: enforce
actual_cost_mode: {mode}
expose_headers:
  estimated: true
  actual: true
  max: true
"#
            ),
        )
        .await;
        let json = &outcome.json;
        let label = format!("{}_{mode}", fixture.query_file);
        assert_accepted(json, &label);
        assert_cost(&outcome, &label, expected_estimated, expected_actual);
        assert_call_counts(&outcome, &label, expected_call_counts(&fixture));
    }

    // basic_fragments: est=12, act=2; products(est=12, act=2 / null)
    #[ntex::test]
    async fn basic_fragments_by_subgraph() {
        run_case(basic_fragments(), "by_subgraph", 12.0, 2.0).await
    }
    #[ntex::test]
    async fn basic_fragments_by_response_shape() {
        run_case(basic_fragments(), "by_response_shape", 12.0, 2.0).await
    }

    // basic_mutation: est=10, act=0; products(est=10, act=0 / null)
    #[ntex::test]
    async fn basic_mutation_by_subgraph() {
        run_case(basic_mutation(), "by_subgraph", 10.0, 0.0).await
    }
    #[ntex::test]
    async fn basic_mutation_by_response_shape() {
        run_case(basic_mutation(), "by_response_shape", 10.0, 0.0).await
    }

    // federated_ships_required:
    // by_subgraph        : est=140, act=15, users(est=110,act=6), vehicles(est=30,act=9)
    // by_response_shape  : est=140, act=3,  est_by_sg same, act_by_sg null
    #[ntex::test]
    async fn federated_ships_required_by_subgraph() {
        run_case(federated_ships_required(), "by_subgraph", 140.0, 15.0).await
    }
    #[ntex::test]
    async fn federated_ships_required_by_response_shape() {
        run_case(federated_ships_required(), "by_response_shape", 140.0, 3.0).await
    }

    // federated_ships_fragment:
    // by_subgraph        : est=40, act=15, users(est=20,act=5), vehicles(est=20,act=10)
    // by_response_shape  : est=40, act=12, est_by_sg same, act_by_sg null
    #[ntex::test]
    async fn federated_ships_fragment_by_subgraph() {
        run_case(federated_ships_fragment(), "by_subgraph", 40.0, 15.0).await
    }
    #[ntex::test]
    async fn federated_ships_fragment_by_response_shape() {
        run_case(federated_ships_fragment(), "by_response_shape", 40.0, 12.0).await
    }

    // custom_costs:
    // by_subgraph        : est=127, act=124,
    //                      subgraphWithCost(121,121), subgraphWithListSize(6,3)
    // by_response_shape  : est=127, act=124, est_by_sg same, act_by_sg null
    #[ntex::test]
    async fn custom_costs_by_subgraph() {
        run_case(custom_costs(), "by_subgraph", 127.0, 124.0).await
    }
    #[ntex::test]
    async fn custom_costs_by_response_shape() {
        run_case(custom_costs(), "by_response_shape", 127.0, 124.0).await
    }
}
