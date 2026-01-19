#[cfg(test)]
mod hive_cdn_supergraph_e2e_tests {
    use hive_router::pipeline::execution::EXPOSE_QUERY_PLAN_HEADER;
    use ntex::web::test;
    use sonic_rs::{from_slice, json, JsonValueTrait, Value};
    use std::fs;
    use tempfile::NamedTempFile;

    use crate::testkit::{
        init_graphql_request, init_router_from_config_inline, wait_for_readiness, SubgraphsServer,
    };

    #[ntex::test]
    async fn should_allow_to_customize_graphql_endpoint() {
        let file = NamedTempFile::new().expect("failed to create temp file");
        let supergraph_file_path = file
            .path()
            .to_str()
            .expect("failed to convert path to string")
            .to_string();

        let first_supergraph = include_str!("../supergraph.graphql");
        fs::write(&supergraph_file_path, first_supergraph).expect("failed to write supergraph");

        let app = init_router_from_config_inline(&format!(
            r#"
            supergraph:
              source: file
              path: {supergraph_file_path}
            http:
              graphql_endpoint: /custom
        "#,
        ))
        .await
        .expect("failed to start router");
        wait_for_readiness(&app.app).await;

        let body = json!({
          "query": "{ __schema { types { name } } }",
        });

        let req = test::TestRequest::post()
            .uri("/custom")
            .header("content-type", "application/json")
            .set_payload(body.to_string());

        let resp = test::call_service(&app.app, req.to_request()).await;

        assert!(resp.status().is_success(), "Expected 200 OK");

        let req = test::TestRequest::post()
            .uri("/graphql")
            .header("content-type", "application/json")
            .set_payload(body.to_string());

        let resp = test::call_service(&app.app, req.to_request()).await;

        assert_eq!(resp.status(), 404);
    }

    #[ntex::test]
    async fn should_not_expose_query_plan_when_disabled() {
        let _subgraphs_server = SubgraphsServer::start().await;

        let router = init_router_from_config_inline(&format!(
            r#"
            supergraph:
                source: file
                path: supergraph.graphql
            # default is false
            # query_planner:
            #     allow_expose: false
            "#
        ))
        .await
        .unwrap();

        wait_for_readiness(&router.app).await;

        let req = init_graphql_request(
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
        )
        .header(EXPOSE_QUERY_PLAN_HEADER.as_str(), "true")
        .to_request();

        let res = test::call_service(&router.app, req).await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let body = test::read_body(res).await;
        let json_body: Value = from_slice(&body).unwrap();

        assert!(json_body["data"].is_object());
        assert!(json_body["errors"].is_null());
        assert!(json_body["extensions"].is_null());
    }

    #[ntex::test]
    async fn should_execute_and_expose_query_plan() {
        let _subgraphs_server = SubgraphsServer::start().await;

        let router = init_router_from_config_inline(&format!(
            r#"
            supergraph:
                source: file
                path: supergraph.graphql
            query_planner:
                allow_expose: true
            "#
        ))
        .await
        .unwrap();

        wait_for_readiness(&router.app).await;

        let req = init_graphql_request(
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
        )
        .header(EXPOSE_QUERY_PLAN_HEADER.as_str(), "true")
        .to_request();

        let res = test::call_service(&router.app, req).await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let body = test::read_body(res).await;
        let json_body: Value = from_slice(&body).unwrap();

        assert!(json_body["data"].is_object());
        assert!(json_body["errors"].is_null());
        assert!(json_body["extensions"].is_object());
        assert!(json_body["extensions"]["queryPlan"].is_object());
    }

    #[ntex::test]
    async fn should_dry_run_and_expose_query_plan() {
        let subgraphs_server = SubgraphsServer::start().await;

        let router = init_router_from_config_inline(&format!(
            r#"
            supergraph:
                source: file
                path: supergraph.graphql
            query_planner:
                allow_expose: true
            "#
        ))
        .await
        .unwrap();

        wait_for_readiness(&router.app).await;

        let req = init_graphql_request(
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
        )
        .header(EXPOSE_QUERY_PLAN_HEADER.as_str(), "dry-run")
        .to_request();

        let res = test::call_service(&router.app, req).await;

        assert!(res.status().is_success(), "Expected 200 OK");

        let body = test::read_body(res).await;
        let json_body: Value = from_slice(&body).unwrap();

        assert!(json_body["data"].is_null());
        assert!(json_body["errors"].is_null());
        assert!(json_body["extensions"].is_object());
        assert!(json_body["extensions"]["queryPlan"].is_object());

        assert!(
            subgraphs_server
                .get_subgraph_requests_log("products") // we check products because our root query is topProducts
                .await
                .is_none(),
            "expected no requests to products subgraph"
        )
    }
}
