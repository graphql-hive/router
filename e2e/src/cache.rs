#[cfg(test)]
mod cache_e2e_tests {
    use std::time::Duration;

    use sonic_rs::JsonValueTrait;

    use crate::testkit::{ClientResponseExt, TestRouter};

    /// Introspection needs no subgraph resolution, so it works against the default
    /// file supergraph with no `TestSubgraphs` and exercises every router-runtime
    /// cache (parse, validate, normalize, plan) on each request.
    const INTROSPECT_QUERY: &str = r#"{ __type(name: "Query") { fields { name } } }"#;

    #[ntex::test]
    async fn router_runtime_caches_honor_configured_limits() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                cache:
                  router:
                    parsing:
                      max_entries: 11
                      time_to_live: 30m
                      time_to_idle: 5m
                  supergraph:
                    validation:
                      max_entries: 12
                    normalization:
                      max_entries: 13
                      time_to_idle: 1m
                    query_plans:
                      max_entries: 14
                      time_to_live: 2m
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(INTROSPECT_QUERY, None, None)
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        let parse_cache = &router.shared_state().parse_cache;
        assert_eq!(parse_cache.policy().max_capacity(), Some(11));
        assert_eq!(
            parse_cache.policy().time_to_live(),
            Some(Duration::from_secs(30 * 60))
        );
        assert_eq!(
            parse_cache.policy().time_to_idle(),
            Some(Duration::from_secs(5 * 60))
        );

        let runtime = router
            .schema_state()
            .configured_runtime()
            .expect("configured runtime to exist");
        assert_eq!(runtime.validate_cache.policy().max_capacity(), Some(12));
        assert_eq!(runtime.validate_cache.policy().time_to_live(), None);
        assert_eq!(runtime.validate_cache.policy().time_to_idle(), None);
        assert_eq!(runtime.normalize_cache.policy().max_capacity(), Some(13));
        assert_eq!(runtime.normalize_cache.policy().time_to_live(), None);
        assert_eq!(
            runtime.normalize_cache.policy().time_to_idle(),
            Some(Duration::from_secs(60))
        );
        assert_eq!(runtime.plan_cache.policy().max_capacity(), Some(14));
        assert_eq!(
            runtime.plan_cache.policy().time_to_live(),
            Some(Duration::from_secs(2 * 60))
        );
        assert_eq!(runtime.plan_cache.policy().time_to_idle(), None);
    }

    #[ntex::test]
    async fn zero_max_entries_disables_caches() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                cache:
                  router:
                    parsing:
                      max_entries: 0
                  supergraph:
                    validation:
                      max_entries: 0
                    normalization:
                      max_entries: 0
                    query_plans:
                      max_entries: 0
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(INTROSPECT_QUERY, None, None)
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");

        let shared = router.shared_state();
        shared.parse_cache.run_pending_tasks().await;
        assert_eq!(shared.parse_cache.entry_count(), 0);

        let runtime = router
            .schema_state()
            .configured_runtime()
            .expect("configured runtime to exist");
        runtime.validate_cache.run_pending_tasks().await;
        runtime.normalize_cache.run_pending_tasks().await;
        runtime.plan_cache.run_pending_tasks().await;
        assert_eq!(runtime.validate_cache.entry_count(), 0);
        assert_eq!(runtime.normalize_cache.entry_count(), 0);
        assert_eq!(runtime.plan_cache.entry_count(), 0);
    }

    #[ntex::test]
    async fn short_ttl_evicts_plan_cache_entries() {
        let router = TestRouter::builder()
            .inline_config(
                r#"
                cache:
                  supergraph:
                    query_plans:
                      time_to_live: 200ms
                "#,
            )
            .build()
            .start()
            .await;

        let runtime = router
            .schema_state()
            .configured_runtime()
            .expect("configured runtime to exist");

        // wait for the request below to populate the plan cache
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let res = router
                .send_graphql_request(INTROSPECT_QUERY, None, None)
                .await;
            assert!(res.status().is_success(), "Expected 200 OK");

            runtime.plan_cache.run_pending_tasks().await;
            if runtime.plan_cache.entry_count() >= 1 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for the plan cache to populate"
            );
            ntex::time::sleep(Duration::from_millis(50)).await;
        }

        // the entry must age out on its own once the TTL passes
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            runtime.plan_cache.run_pending_tasks().await;
            if runtime.plan_cache.entry_count() == 0 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for the plan cache entry to expire"
            );
            ntex::time::sleep(Duration::from_millis(50)).await;
        }

        // ... and the next request transparently recomputes the evicted plan
        let res = router
            .send_graphql_request(INTROSPECT_QUERY, None, None)
            .await;
        assert!(res.status().is_success(), "Expected 200 OK");
        let body = res.json_body().await;
        assert!(body["data"].is_object());
        assert!(body["errors"].is_null());
    }
}
