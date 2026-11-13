#[cfg(test)]
mod fetch_scheduling_e2e_tests {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use sonic_rs::JsonValueTrait;

    use crate::testkit::{ClientResponseExt, TestRouter, TestSubgraphs};

    /// A fetch starts as soon as the fetches it reads from are done. The authors only need the
    /// reviews, so they're asked for while the slow `inStock` call is still running, not after
    /// it, even though the plan has both in one wave.
    #[ntex::test]
    async fn a_fetch_does_not_wait_for_a_slow_fetch_it_does_not_read() {
        const SLOW: Duration = Duration::from_millis(1000);
        let accounts_asked_at = Arc::new(Mutex::new(None));
        let recorder = accounts_asked_at.clone();
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(move |request| {
                if request.path == "/accounts" {
                    recorder.lock().unwrap().get_or_insert_with(Instant::now);
                }
                None
            })
            .with_path_delay("/inventory", SLOW)
            .build()
            .start()
            .await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                  supergraph:
                    source: file
                    path: supergraph.graphql
                  "#,
            )
            .build()
            .start()
            .await;

        let started = Instant::now();
        let response = router
            .send_graphql_request(
                "{ topProducts { upc inStock reviews { author { name } } } }",
                None,
                None,
            )
            .await;
        let took = started.elapsed();
        let body = response.json_body().await;

        assert!(body["errors"].is_null(), "{body}");
        assert!(body["data"]["topProducts"][0]["reviews"][0]["author"]["name"].is_str());
        assert!(took >= SLOW, "the slow call is part of the response");
        let accounts_after = accounts_asked_at
            .lock()
            .unwrap()
            .expect("the authors were asked for")
            - started;
        assert!(
            accounts_after < SLOW / 2,
            "the authors waited {accounts_after:?}, as if for the {SLOW:?} inventory call"
        );
    }
}
