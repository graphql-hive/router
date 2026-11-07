#[cfg(test)]
mod http1_pool_e2e_tests {
    use crate::testkit::{ClientResponseExt, TestRouter, TestSubgraphs};

    // The pool hands connections back after each response, so run enough requests, one after
    // another and side by side, that connections get reused in both client modes.
    #[ntex::test]
    async fn bench_operation_goes_through_the_http1_pool() {
        let expected: sonic_rs::Value =
            sonic_rs::from_str(include_str!("../../bench/expected_response.json")).unwrap();

        for mode in ["shared", "per_worker"] {
            let subgraphs = TestSubgraphs::builder().build().start().await;
            let router = TestRouter::builder()
                .with_subgraphs(&subgraphs)
                .inline_config(format!(
                    r#"
                      supergraph:
                        source: file
                        path: supergraph.graphql
                      traffic_shaping:
                        experimental_http1_pool: true
                        http_client_mode: {mode}
                        all:
                          dedupe_enabled: false
                      "#
                ))
                .build()
                .start()
                .await;

            for _ in 0..3 {
                let responses = futures::future::join_all((0..8).map(|_| {
                    router.send_graphql_request(
                        include_str!("../../bench/operation.graphql"),
                        None,
                        None,
                    )
                }))
                .await;
                for res in responses {
                    assert!(res.status().is_success(), "{mode}: expected 200 OK");
                    let body: sonic_rs::Value = res.json_body().await;
                    assert_eq!(body, expected, "{mode}: wrong response");
                }
            }
        }
    }
}
