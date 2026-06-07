#[cfg(test)]
mod requests_within_max_are_accepted_tests {
    use super::super::common::*;

    async fn run_eq(fixture: Fixture, max: u64, expected_estimated: f64, expected_actual: f64) {
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
    actual_cost_mode: by_subgraph
    max: {max}
"#
            ),
        )
        .await;
        let json = &outcome.json;
        let label = format!("{} eq max={max}", fixture.query_file);
        assert_accepted(json, &label);
        assert_cost(&outcome, &label, expected_estimated, expected_actual);
        assert_call_counts(&outcome, &label, expected_call_counts(&fixture));
    }

    #[ntex::test]
    async fn basic_fragments_eq() {
        run_eq(basic_fragments(), 12, 12.0, 2.0).await
    }
    #[ntex::test]
    async fn basic_fragments_lt() {
        run_eq(basic_fragments(), 15, 12.0, 2.0).await
    }

    // basic_mutation: estimated=10, actual=0;
    #[ntex::test]
    async fn basic_mutation_eq() {
        run_eq(basic_mutation(), 10, 10.0, 0.0).await
    }
    #[ntex::test]
    async fn basic_mutation_lt() {
        run_eq(basic_mutation(), 15, 10.0, 0.0).await
    }

    // federated_ships_required: estimated=140, actual=15;
    #[ntex::test]
    async fn federated_ships_required_eq() {
        run_eq(federated_ships_required(), 140, 140.0, 15.0).await
    }
    #[ntex::test]
    async fn federated_ships_required_lt() {
        run_eq(federated_ships_required(), 150, 140.0, 15.0).await
    }

    // federated_ships_fragment: estimated=40, actual=15;
    #[ntex::test]
    async fn federated_ships_fragment_eq() {
        run_eq(federated_ships_fragment(), 40, 40.0, 15.0).await
    }
    #[ntex::test]
    async fn federated_ships_fragment_lt() {
        run_eq(federated_ships_fragment(), 50, 40.0, 15.0).await
    }

    // custom_costs: estimated=127, actual=124;
    #[ntex::test]
    async fn custom_costs_eq() {
        run_eq(custom_costs(), 127, 127.0, 124.0).await
    }
    #[ntex::test]
    async fn custom_costs_lt() {
        run_eq(custom_costs(), 130, 127.0, 124.0).await
    }
}
