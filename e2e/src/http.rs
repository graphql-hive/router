#[cfg(test)]
mod http_tests {
    use hive_router::pipeline::execution::EXPOSE_QUERY_PLAN_HEADER;
    use sonic_rs::JsonValueTrait;

    use crate::testkit::{some_header_map, TestRouterBuilder, TestSubgraphsBuilder};

    #[ntex::test]
    async fn should_allow_to_customize_graphql_endpoint() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                http:
                    graphql_endpoint: /custom
                "#,
            )
            .build()
            .start()
            .await;

        let json_body = sonic_rs::json!({
          "query": "{ __schema { types { name } } }",
        });

        let res = router
            .serv()
            // even though testrouter will pick up the changed graphql route
            // we use a custom post anyways just to be extra sure
            .post("/custom")
            .content_type("application/json")
            .send_json(&json_body)
            .await
            .expect("failed to send graphql request");

        assert_eq!(res.status(), 200, "Expected 200 OK");

        let res = router
            .serv()
            // we changed and dont use the default
            .post("/graphql")
            .content_type("application/json")
            .send_json(&json_body)
            .await
            .expect("failed to send graphql request");

        assert_eq!(res.status(), 404);
    }

    #[ntex::test]
    async fn should_not_expose_query_plan_when_disabled() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                # default is false
                # query_planner:
                #     allow_expose: false
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                {
                    topProducts {
                        name
                        price
                        reviews {
                            author {
                                name
                            }
                        }
                    }
                }
                "#,
                None,
                some_header_map! {
                    http::header::HeaderName::from_static(EXPOSE_QUERY_PLAN_HEADER.as_str()) => "true"
                },
            )
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let body = res.body().await.unwrap();
        let json_body: sonic_rs::Value = sonic_rs::from_slice(&body).unwrap();

        assert!(json_body["data"].is_object());
        assert!(json_body["errors"].is_null());
        assert!(json_body["extensions"].is_null());
    }

    #[ntex::test]
    async fn should_execute_and_expose_query_plan() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                query_planner:
                    allow_expose: true
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                {
                    topProducts {
                        name
                        price
                        reviews {
                            author {
                                name
                            }
                        }
                    }
                }
                "#,
                None,
                some_header_map! {
                    http::header::HeaderName::from_static(EXPOSE_QUERY_PLAN_HEADER.as_str()) => "true"
                },
            )
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let body = res.body().await.unwrap();
        let json_body: sonic_rs::Value = sonic_rs::from_slice(&body).unwrap();

        assert!(json_body["data"].is_object());
        assert!(json_body["errors"].is_null());
        assert!(json_body["extensions"].is_object());
        assert!(json_body["extensions"]["queryPlan"].is_object());
    }

    #[ntex::test]
    async fn should_dry_run_and_expose_query_plan() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;
        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                query_planner:
                    allow_expose: true
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                {
                    topProducts {
                        name
                        price
                        reviews {
                            author {
                                name
                            }
                        }
                    }
                }
                "#,
                None,
                some_header_map! {
                    http::header::HeaderName::from_static(EXPOSE_QUERY_PLAN_HEADER.as_str()) => "dry-run"
                },
            )
            .await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let body = res.body().await.unwrap();
        let json_body: sonic_rs::Value = sonic_rs::from_slice(&body).unwrap();

        assert!(json_body["data"].is_null());
        assert!(json_body["errors"].is_null());
        assert!(json_body["extensions"].is_object());
        assert!(json_body["extensions"]["queryPlan"].is_object());

        assert!(
            subgraphs.get_requests_log("products").is_none(),
            "expected no requests to products subgraph"
        );
    }
}
