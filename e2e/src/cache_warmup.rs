#[cfg(test)]
mod cache_warmup_e2e_tests {
    use std::time::{Duration, Instant};

    use hive_router::{
        async_trait,
        plugins::{
            hooks::{
                on_cache_warmup::{CacheWarmupOperation, OnCacheWarmupHookPayload},
                on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
            },
            plugin_trait::RouterPlugin,
        },
    };
    use mockito::{Matcher, Mock, Server, ServerGuard};

    use crate::testkit::{wait_until_mock_matched, ClientResponseExt, Started, TestRouter};

    const SUPERGRAPH: &str = include_str!("../supergraph.graphql");

    const PREVIOUS_OPERATION: &str = "query FromTraffic { users { id } }";
    const PLUGIN_OPERATION: &str = "query FromPlugin { topProducts { upc name } }";
    const PLUGIN_BROKEN_OPERATION: &str = "query Broken { doesNotExist }";

    // serves the first supergraph, then 304s until `publish_second_supergraph` is called
    fn serve_first_supergraph(server: &mut ServerGuard) -> (Mock, Mock) {
        let first = server
            .mock("GET", "/supergraph")
            .match_header("if-none-match", Matcher::Missing)
            .with_status(200)
            .with_header("etag", "v1")
            .with_body(SUPERGRAPH)
            .create();
        let not_modified = server
            .mock("GET", "/supergraph")
            .match_header("if-none-match", "v1")
            .with_status(304)
            .create();
        (first, not_modified)
    }

    // the same schema with one more line, enough to count as a new supergraph
    async fn publish_second_supergraph(server: &mut ServerGuard, previous: (Mock, Mock)) -> Mock {
        previous.0.remove();
        previous.1.remove();
        let second = server
            .mock("GET", "/supergraph")
            .match_header("if-none-match", "v1")
            .with_status(200)
            .with_header("etag", "v2")
            .with_body(format!("{SUPERGRAPH}\n# second publish\n"))
            .create();
        server
            .mock("GET", "/supergraph")
            .match_header("if-none-match", "v2")
            .with_status(304)
            .create();
        wait_until_mock_matched(&second)
            .await
            .expect("the router never fetched the second supergraph");
        second
    }

    async fn scrape(router: &TestRouter<Started>) -> String {
        router
            .serv()
            .get("/metrics")
            .send()
            .await
            .expect("failed to scrape /metrics")
            .string_body()
            .await
    }

    // sum of every sample of `name` that has all of `labels`, 0 when there's none
    fn sample(body: &str, name: &str, labels: &[&str]) -> f64 {
        body.lines()
            .filter(|line| line.starts_with(name) && !line.starts_with('#'))
            .filter(|line| line[name.len()..].starts_with(['{', ' ']))
            .filter(|line| labels.iter().all(|label| line.contains(label)))
            .filter_map(|line| line.rsplit(' ').next())
            .map(|value| value.parse::<f64>().expect("not a number"))
            .sum()
    }

    fn warmup_ops(body: &str, source: &str, result: &str) -> f64 {
        sample(
            body,
            "hive_router_plan_cache_warmup_operations_total",
            &[
                &format!("source=\"{source}\""),
                &format!("result=\"{result}\""),
            ],
        )
    }

    // warmup is part of the reload, so a finished run means the new supergraph is in use
    async fn wait_for_warmup_runs(router: &TestRouter<Started>, runs: f64) -> String {
        let started = Instant::now();
        loop {
            let body = scrape(router).await;
            if sample(
                &body,
                "hive_router_plan_cache_warmup_duration_seconds_count",
                &[],
            ) >= runs
            {
                return body;
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "warmup didn't run {runs} times, metrics:\n{body}"
            );
            ntex::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn start_router(
        server: &ServerGuard,
        warmup: &str,
        plugins: &str,
    ) -> TestRouter<Started> {
        let host = server.host_with_port();
        TestRouter::builder()
            .inline_config(format!(
                r#"
                supergraph:
                    source: hive
                    endpoint: http://{host}/supergraph
                    key: dummy_key
                    poll_interval: 100ms
                cache:
                    warmup:
{warmup}
                telemetry:
                    metrics:
                        exporters:
                            - kind: prometheus
{plugins}
                "#,
            ))
            .register_plugin::<WarmupPlugin>()
            .build()
            .start()
            .await
    }

    #[ntex::test]
    async fn plans_previous_operations_before_switching() {
        let mut server = Server::new_async().await;
        let first = serve_first_supergraph(&mut server);
        let router = start_router(
            &server,
            r#"
                        enabled: true
            "#,
            "",
        )
        .await;

        // the first load has nothing to warm from
        let body = wait_for_warmup_runs(&router, 1.0).await;
        assert_eq!(
            warmup_ops(&body, "previous_supergraph", "warmed"),
            0.0,
            "{body}"
        );

        // real traffic fills the normalization cache the next warmup reads from
        router
            .send_graphql_request(PREVIOUS_OPERATION, None, None)
            .await;

        publish_second_supergraph(&mut server, first).await;
        let body = wait_for_warmup_runs(&router, 2.0).await;

        assert_eq!(
            warmup_ops(&body, "previous_supergraph", "warmed"),
            1.0,
            "{body}"
        );
        assert_eq!(
            sample(
                &body,
                "hive_router_plan_cache_warmup_duration_seconds_count",
                &["result=\"completed\""]
            ),
            2.0,
            "{body}"
        );

        // the operation gets a plan cache hit on the new supergraph, first try
        let hits_before = sample(
            &body,
            "hive_router_plan_cache_requests_total",
            &["result=\"hit\""],
        );
        router
            .send_graphql_request(PREVIOUS_OPERATION, None, None)
            .await;
        let body = scrape(&router).await;
        assert_eq!(
            sample(
                &body,
                "hive_router_plan_cache_requests_total",
                &["result=\"hit\""]
            ),
            hits_before + 1.0,
            "{body}"
        );
    }

    #[ntex::test]
    async fn switches_over_with_what_got_planned_when_the_timeout_runs_out() {
        let mut server = Server::new_async().await;
        let first = serve_first_supergraph(&mut server);
        let router = start_router(
            &server,
            r#"
                        enabled: true
                        timeout: 0s
            "#,
            "",
        )
        .await;

        router
            .send_graphql_request(PREVIOUS_OPERATION, None, None)
            .await;
        publish_second_supergraph(&mut server, first).await;
        let body = wait_for_warmup_runs(&router, 2.0).await;

        assert_eq!(
            warmup_ops(&body, "previous_supergraph", "timed_out"),
            1.0,
            "{body}"
        );
        assert_eq!(
            warmup_ops(&body, "previous_supergraph", "warmed"),
            0.0,
            "{body}"
        );
        assert_eq!(
            sample(
                &body,
                "hive_router_plan_cache_warmup_duration_seconds_count",
                &["result=\"timed_out\""]
            ),
            1.0,
            "{body}"
        );

        // the new supergraph is serving, it just plans on the first request
        let response = router
            .send_graphql_request("{ __typename }", None, None)
            .await;
        assert!(response.status().is_success());
    }

    #[ntex::test]
    async fn plugins_add_and_drop_operations() {
        let mut server = Server::new_async().await;
        let first = serve_first_supergraph(&mut server);
        let router = start_router(
            &server,
            r#"
                        enabled: true
            "#,
            r#"
                plugins:
                    test_warmup:
                        enabled: true
            "#,
        )
        .await;

        let body = wait_for_warmup_runs(&router, 1.0).await;
        assert_eq!(warmup_ops(&body, "plugin", "warmed"), 1.0, "{body}");
        assert_eq!(warmup_ops(&body, "plugin", "failed"), 1.0, "{body}");

        router
            .send_graphql_request(PREVIOUS_OPERATION, None, None)
            .await;
        publish_second_supergraph(&mut server, first).await;
        let body = wait_for_warmup_runs(&router, 2.0).await;

        // the plugin dropped everything the previous supergraph served
        assert_eq!(
            warmup_ops(&body, "previous_supergraph", "warmed"),
            0.0,
            "{body}"
        );
        assert_eq!(warmup_ops(&body, "plugin", "warmed"), 2.0, "{body}");

        // what the plugin added gets a plan cache hit on the new supergraph
        let hits_before = sample(
            &body,
            "hive_router_plan_cache_requests_total",
            &["result=\"hit\""],
        );
        router
            .send_graphql_request(PLUGIN_OPERATION, None, None)
            .await;
        let body = scrape(&router).await;
        assert_eq!(
            sample(
                &body,
                "hive_router_plan_cache_requests_total",
                &["result=\"hit\""]
            ),
            hits_before + 1.0,
            "{body}"
        );
    }

    // replaces the list with its own two operations, one the schema rejects
    #[derive(Default)]
    struct WarmupPlugin;

    #[async_trait]
    impl RouterPlugin for WarmupPlugin {
        type Config = ();

        fn plugin_name() -> &'static str {
            "test_warmup"
        }

        fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
            payload.initialize_plugin_with_defaults()
        }

        async fn on_cache_warmup<'exec>(
            &'exec self,
            payload: &mut OnCacheWarmupHookPayload<'exec>,
        ) {
            payload.operations.clear();
            payload.operations.push(CacheWarmupOperation::new(
                PLUGIN_OPERATION,
                Some("FromPlugin".to_string()),
            ));
            payload.operations.push(CacheWarmupOperation::new(
                PLUGIN_BROKEN_OPERATION,
                Some("Broken".to_string()),
            ));
        }
    }
}
